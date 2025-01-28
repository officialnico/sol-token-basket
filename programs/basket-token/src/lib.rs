use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount};
use solana_program::instruction::{AccountMeta, Instruction};

declare_id!("E5T9eLjbfeWRCegwocsrWfn1CkH4a5MnySjheUuwfNMt");

#[account]
#[derive(Default)]
pub struct BasketState {
    pub authority: Pubkey,
    pub tokens: Vec<TokenInfo>,
    pub total_supply: u64,
    pub bump: u8,
    pub max_tokens: u8,
    pub paused: bool,
    pub reentrancy_guard: bool,
}

impl BasketState {
    pub const DISCRIMINATOR_SIZE: usize = 8;
    pub const AUTHORITY_SIZE: usize = 32;
    pub const TOKEN_ENTRY_SIZE: usize = 32 + 1 + 32; // Pubkey + weight(u8) + token_account
    pub const VEC_PREFIX_SIZE: usize = 4; // For Vec length
    pub const TOTAL_SUPPLY_SIZE: usize = 8;
    pub const BUMP_SIZE: usize = 1;
    pub const MAX_TOKENS_SIZE: usize = 1;
    pub const PAUSED_SIZE: usize = 1;
    pub const REENTRANCY_GUARD_SIZE: usize = 1;

    pub fn required_space(max_tokens: usize) -> usize {
        Self::DISCRIMINATOR_SIZE
            + Self::AUTHORITY_SIZE
            + Self::VEC_PREFIX_SIZE
            + (Self::TOKEN_ENTRY_SIZE * max_tokens)
            + Self::TOTAL_SUPPLY_SIZE
            + Self::BUMP_SIZE
            + Self::MAX_TOKENS_SIZE
            + Self::PAUSED_SIZE
            + Self::REENTRANCY_GUARD_SIZE
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct TokenInfo {
    pub mint: Pubkey,
    pub weight: u8,            // Percentage weight in basket
    pub token_account: Pubkey, // Associated token account owned by basket
}
pub mod jupiter {
    use anchor_lang::prelude::*;

    pub static JUPITER_V6_ID: Pubkey =
        solana_program::pubkey!("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");

    #[derive(AnchorSerialize, AnchorDeserialize)]
    pub struct RouteSwapParams {
        pub in_amount: u64,
        pub quote_id: [u8; 32],
        pub slippage_bps: u16,
    }
}

pub struct ReentrancyGuard<'info> {
    guard_account: &'info mut Account<'info, BasketState>,
}

impl<'info> Drop for ReentrancyGuard<'info> {
    fn drop(&mut self) {
        self.guard_account.reentrancy_guard = false;
    }
}

pub fn with_reentrancy_guard<'a, T, F>(account: &'a mut Account<'a, BasketState>, f: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    if account.reentrancy_guard {
        return err!(BasketError::ReentrancyDetected);
    }
    account.reentrancy_guard = true;
    let _guard = ReentrancyGuard {
        guard_account: account,
    };
    let result = f();
    result
}

#[program]
pub mod basket_token {
    use super::*;

    pub const MAGNIFIER: u128 = 1_000_000_000;
    pub const MINIMUM_DEPOSIT: u64 = 10_000_000;
    pub const MAX_TOKENS: usize = 10;

    pub fn initialize(ctx: Context<Initialize>, max_tokens: u8) -> Result<()> {
        require!(
            max_tokens as usize <= MAX_TOKENS,
            BasketError::TooManyTokens
        );

        let basket = &mut ctx.accounts.basket;
        basket.authority = ctx.accounts.authority.key();
        basket.tokens = vec![];
        basket.total_supply = 0;
        basket.bump = *ctx.bumps.get("basket").unwrap();
        basket.max_tokens = max_tokens;
        basket.paused = false;
        basket.reentrancy_guard = false;
        Ok(())
    }

    pub fn add_token(ctx: Context<AddToken>, token_mint: Pubkey, weight: u8) -> Result<()> {
        let basket = &mut ctx.accounts.basket;
        require!(!basket.paused, BasketError::ProgramPaused);
        require!(
            basket.authority == ctx.accounts.authority.key(),
            BasketError::Unauthorized
        );
        require!(
            basket.tokens.len() < basket.max_tokens as usize,
            BasketError::TooManyTokens
        );

        // Validate total weights
        let total_weight: u8 = basket
            .tokens
            .iter()
            .map(|t| t.weight)
            .sum::<u8>()
            .checked_add(weight)
            .ok_or(BasketError::WeightOverflow)?;
        require!(total_weight <= 100, BasketError::WeightOverflow);

        // Check for duplicate token
        require!(
            !basket.tokens.iter().any(|t| t.mint == token_mint),
            BasketError::DuplicateToken
        );

        basket.tokens.push(TokenInfo {
            mint: token_mint,
            weight,
            token_account: Pubkey::default(),
        });

        Ok(())
    }

    pub fn remove_token(ctx: Context<RemoveToken>, token_mint: Pubkey) -> Result<()> {
        let basket = &mut ctx.accounts.basket;
        require!(!basket.paused, BasketError::ProgramPaused);
        require!(
            basket.authority == ctx.accounts.authority.key(),
            BasketError::Unauthorized
        );

        let token_index = basket
            .tokens
            .iter()
            .position(|t| t.mint == token_mint)
            .ok_or(BasketError::TokenNotFound)?;

        basket.tokens.remove(token_index);

        Ok(())
    }

    pub fn set_paused(ctx: Context<SetPaused>, paused: bool) -> Result<()> {
        let basket = &mut ctx.accounts.basket;
        require!(
            basket.authority == ctx.accounts.authority.key(),
            BasketError::Unauthorized
        );
        basket.paused = paused;
        Ok(())
    }

    pub fn deposit(
        ctx: Context<Deposit>,
        amount: u64,
        jupiter_quote: [u8; 32],
        slippage_bps: u16,
        minimum_token_amounts: Vec<u64>,
    ) -> Result<()> {
        let basket = &mut ctx.accounts.basket;
        require!(!basket.paused, BasketError::ProgramPaused);
        require!(!basket.reentrancy_guard, BasketError::ReentrancyDetected);
        require!(amount >= MINIMUM_DEPOSIT, BasketError::InsufficientDeposit);
        require!(
            minimum_token_amounts.len() == basket.tokens.len(),
            BasketError::InvalidTokenCount
        );

        // Set reentrancy guard
        basket.reentrancy_guard = true;

        // Validate remaining accounts count
        let remaining_account_count = ctx.remaining_accounts.len();
        require!(
            remaining_account_count == basket.tokens.len() * 12,
            BasketError::InvalidAccountCount
        );

        // Transfer SOL from user to basket first
        let transfer_ix = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.user.key(),
            &basket.key(),
            amount,
        );

        anchor_lang::solana_program::program::invoke(
            &transfer_ix,
            &[
                ctx.accounts.user.to_account_info(),
                basket.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Update state before external calls
        basket.total_supply = basket
            .total_supply
            .checked_add(amount)
            .ok_or(BasketError::MathOverflow)?;

        // Mint basket tokens to user
        let seeds = &[b"basket".as_ref(), &[basket.bump]];
        let cpi_accounts = token::MintTo {
            mint: ctx.accounts.basket_mint.to_account_info(),
            to: ctx.accounts.user_basket_token.to_account_info(),
            authority: basket.to_account_info(),
        };

        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts,
                &[seeds],
            ),
            amount,
        )?;

        // Execute Jupiter swaps
        let mut initial_lamports = basket.to_account_info().lamports();

        for (i, token_info) in basket.tokens.iter().enumerate() {
            // Get token account from remaining accounts
            let token_account = &ctx.remaining_accounts[i * 12 + 1];
            let token_acc_data = Account::<TokenAccount>::try_from(&token_account)?;
            require!(
                token_acc_data.mint == token_info.mint,
                BasketError::InvalidTokenMint
            );
            require!(
                token_acc_data.owner == basket.key(),
                BasketError::InvalidTokenOwner
            );

            let token_amount = token_acc_data.amount;

            // Create Jupiter swap instruction
            let swap_ix = Instruction {
                program_id: jupiter::JUPITER_V6_ID,
                accounts: ctx.remaining_accounts[i * 12..(i + 1) * 12]
                    .iter()
                    .map(|acc| AccountMeta {
                        pubkey: *acc.key,
                        is_signer: acc.is_signer,
                        is_writable: acc.is_writable,
                    })
                    .collect(),
                data: AnchorSerialize::try_to_vec(&(
                    4u8,
                    jupiter::RouteSwapParams {
                        in_amount: token_amount,
                        quote_id: jupiter_quote,
                        slippage_bps,
                    },
                ))
                .unwrap(),
            };

            // Execute swap
            anchor_lang::solana_program::program::invoke(
                &swap_ix,
                &ctx.remaining_accounts[i * 12..(i + 1) * 12],
            )?;

            // Verify minimum received based on lamports difference
            let current_lamports = basket.to_account_info().lamports();
            let lamports_spent = current_lamports
                .checked_sub(initial_lamports)
                .ok_or(BasketError::MathOverflow)?;

            require!(
                lamports_spent >= minimum_token_amounts[i],
                BasketError::SlippageExceeded
            );

            initial_lamports = current_lamports;
        }

        // Clear reentrancy guard
        basket.reentrancy_guard = false;

        Ok(())
    }

    pub fn redeem(
        ctx: Context<Redeem>,
        amount: u64,
        jupiter_quote: [u8; 32],
        slippage_bps: u16,
        minimum_sol_amount: u64,
    ) -> Result<()> {
        let basket = &mut ctx.accounts.basket;
        require!(!basket.paused, BasketError::ProgramPaused);
        require!(!basket.reentrancy_guard, BasketError::ReentrancyDetected);

        // Set reentrancy guard
        basket.reentrancy_guard = true;

        // Validate remaining accounts count
        let remaining_account_count = ctx.remaining_accounts.len();
        require!(
            remaining_account_count == basket.tokens.len() * 12,
            BasketError::InvalidAccountCount
        );

        // Calculate redemption ratio with higher precision
        let redemption_ratio = (amount as u128)
            .checked_mul(MAGNIFIER)
            .ok_or(BasketError::MathOverflow)?
            .checked_div(basket.total_supply as u128)
            .ok_or(BasketError::MathOverflow)?;

        // Update state before external calls
        basket.total_supply = basket
            .total_supply
            .checked_sub(amount)
            .ok_or(BasketError::MathOverflow)?;

        // Burn basket tokens
        let seeds = &[b"basket".as_ref(), &[basket.bump]];
        token::burn(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token::Burn {
                    mint: ctx.accounts.basket_mint.to_account_info(),
                    from: ctx.accounts.user_basket_token.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
                &[seeds],
            ),
            amount,
        )?;

        // Sell tokens back to SOL
        let initial_basket_lamports = basket.to_account_info().lamports();
        let mut total_sol_received = 0;

        for (i, token_info) in basket.tokens.iter().enumerate() {
            // Verify token account mint and owner
            let token_account = Account::<TokenAccount>::try_from(
                &ctx.accounts.user_basket_token.to_account_info(),
            )?;
            let token_acc_data = Account::<TokenAccount>::try_from(
                &ctx.accounts.user_basket_token.to_account_info(),
            )?;
            require!(
                token_acc_data.mint == token_info.mint,
                BasketError::InvalidTokenMint
            );
            require!(
                token_acc_data.owner == basket.key(),
                BasketError::InvalidTokenOwner
            );

            let token_amount = token_acc_data.amount;
            let redeem_amount = (token_amount as u128)
                .checked_mul(redemption_ratio)
                .ok_or(BasketError::MathOverflow)?
                .checked_div(MAGNIFIER)
                .ok_or(BasketError::MathOverflow)? as u64;

            // Execute Jupiter swap
            let swap_ix = Instruction {
                program_id: jupiter::JUPITER_V6_ID,
                accounts: ctx.remaining_accounts[i * 12..(i + 1) * 12]
                    .iter()
                    .map(|acc| AccountMeta {
                        pubkey: *acc.key,
                        is_signer: acc.is_signer,
                        is_writable: acc.is_writable,
                    })
                    .collect(),
                data: AnchorSerialize::try_to_vec(&(
                    4u8,
                    jupiter::RouteSwapParams {
                        in_amount: redeem_amount,
                        quote_id: jupiter_quote,
                        slippage_bps,
                    },
                ))
                .unwrap(),
            };

            anchor_lang::solana_program::program::invoke(
                &swap_ix,
                &ctx.remaining_accounts[i * 12..(i + 1) * 12],
            )?;

            // Track SOL received
            let current_lamports = basket.to_account_info().lamports();
            let sol_received = current_lamports
                .checked_sub(initial_basket_lamports + total_sol_received)
                .ok_or(BasketError::MathOverflow)?;
            total_sol_received += sol_received;
        }

        require!(
            total_sol_received >= minimum_sol_amount,
            BasketError::SlippageExceeded
        );

        // Transfer SOL to user
        **basket.to_account_info().try_borrow_mut_lamports()? -= total_sol_received;
        **ctx
            .accounts
            .user
            .to_account_info()
            .try_borrow_mut_lamports()? += total_sol_received;

        // Clear reentrancy guard
        basket.reentrancy_guard = false;

        Ok(())
    }

    pub fn withdraw_authority_sol(ctx: Context<WithdrawAuthoritySol>, amount: u64) -> Result<()> {
        let basket = &ctx.accounts.basket;
        require!(
            basket.authority == ctx.accounts.authority.key(),
            BasketError::Unauthorized
        );

        let basket_lamports = basket.to_account_info().lamports();
        require!(basket_lamports >= amount, BasketError::InsufficientBalance);

        **basket.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx
            .accounts
            .authority
            .to_account_info()
            .try_borrow_mut_lamports()? += amount;

        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = authority,
        space = BasketState::required_space(MAX_TOKENS),  // Use this instead of manual calculation
        seeds = [b"basket"],
        bump
    )]
    pub basket: Account<'info, BasketState>,

    #[account(
        init,
        payer = authority,
        mint::decimals = 9,
        mint::authority = basket,
        seeds = [b"basket_mint"],
        bump
    )]
    pub basket_mint: Account<'info, Mint>,

    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct AddToken<'info> {
    #[account(mut, has_one = authority @ BasketError::Unauthorized)]
    pub basket: Account<'info, BasketState>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct RemoveToken<'info> {
    #[account(mut, has_one = authority @ BasketError::Unauthorized)]
    pub basket: Account<'info, BasketState>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct SetPaused<'info> {
    #[account(mut, has_one = authority @ BasketError::Unauthorized)]
    pub basket: Account<'info, BasketState>,
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub basket: Account<'info, BasketState>,

    #[account(mut)]
    pub basket_mint: Account<'info, Mint>,

    #[account(
        init_if_needed,
        payer = user,
        associated_token::mint = basket_mint,
        associated_token::authority = user
    )]
    pub user_basket_token: Account<'info, TokenAccount>,

    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(mut)]
    pub basket: Account<'info, BasketState>,

    #[account(mut)]
    pub basket_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_basket_token.mint == basket_mint.key(),
        constraint = user_basket_token.owner == user.key()
    )]
    pub user_basket_token: Account<'info, TokenAccount>,

    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct WithdrawAuthoritySol<'info> {
    #[account(mut, has_one = authority @ BasketError::Unauthorized)]
    pub basket: Account<'info, BasketState>,

    #[account(mut)]
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct EmergencyWithdraw<'info> {
    #[account(mut, has_one = authority @ BasketError::Unauthorized)]
    pub basket: Account<'info, BasketState>,

    #[account(mut)]
    pub basket_token: Account<'info, TokenAccount>,

    #[account(mut)]
    pub authority_token: Account<'info, TokenAccount>,

    #[account(mut)]
    pub authority: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[error_code]
pub enum BasketError {
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Insufficient deposit")]
    InsufficientDeposit,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Weight overflow")]
    WeightOverflow,
    #[msg("Invalid token count")]
    InvalidTokenCount,
    #[msg("Invalid account count")]
    InvalidAccountCount,
    #[msg("Invalid token mint")]
    InvalidTokenMint,
    #[msg("Invalid token owner")]
    InvalidTokenOwner,
    #[msg("Slippage exceeded")]
    SlippageExceeded,
    #[msg("Too many tokens")]
    TooManyTokens,
    #[msg("Token not found")]
    TokenNotFound,
    #[msg("Duplicate token")]
    DuplicateToken,
    #[msg("Program paused")]
    ProgramPaused,
    #[msg("Reentrancy detected")]
    ReentrancyDetected,
    #[msg("Insufficient balance")]
    InsufficientBalance,
}
