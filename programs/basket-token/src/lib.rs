use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Mint};
use anchor_spl::associated_token::AssociatedToken;
use solana_program::instruction::{Instruction, AccountMeta};

declare_id!("HEtbYTnFvGD6FePjzrGCyjZwtDR6ycRd26qPcPbNQoCN");

pub mod jupiter {
    use anchor_lang::prelude::*;
    
    pub static JUPITER_V6_ID: Pubkey = solana_program::pubkey!("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");
    
    #[derive(AnchorSerialize, AnchorDeserialize)]
    pub struct RouteSwapParams {
        pub in_amount: u64,
        pub quote_id: [u8; 32],
        pub slippage_bps: u16,
    }
}

#[program]
pub mod basket_token {
    use super::*;

    const MAGNIFIER: u64 = 10_000;
    const MINIMUM_DEPOSIT: u64 = 10_000_000; // 0.01 SOL

    pub fn initialize(
        ctx: Context<Initialize>,
    ) -> Result<()> {
        let basket = &mut ctx.accounts.basket;
        basket.authority = ctx.accounts.authority.key();
        basket.tokens = vec![];
        basket.total_supply = 0;
        Ok(())
    }

    pub fn add_token(
        ctx: Context<AddToken>,
        token_mint: Pubkey,
        weight: u8,
    ) -> Result<()> {
        let basket = &mut ctx.accounts.basket;
        require!(
            basket.authority == ctx.accounts.authority.key(),
            BasketError::Unauthorized
        );
        
        // Validate total weights
        let total_weight: u8 = basket.tokens.iter()
            .map(|t| t.weight)
            .sum::<u8>()
            .checked_add(weight)
            .ok_or(BasketError::WeightOverflow)?;
        require!(total_weight <= 100, BasketError::WeightOverflow);

        basket.tokens.push(TokenInfo {
            mint: token_mint,
            weight,
        });
        
        Ok(())
    }

    pub fn deposit(
        ctx: Context<Deposit>,
        amount: u64,
        jupiter_quote: [u8; 32],
        slippage_bps: u16,
        minimum_token_amounts: Vec<u64>
    ) -> Result<()> {
        require!(amount >= MINIMUM_DEPOSIT, BasketError::InsufficientDeposit);
        require!(
            minimum_token_amounts.len() == ctx.accounts.basket.tokens.len(),
            BasketError::InvalidTokenCount
        );

        // Transfer SOL from user to basket
        let transfer_ix = anchor_lang::solana_program::system_instruction::transfer(
            &ctx.accounts.user.key(),
            &ctx.accounts.basket.key(),
            amount
        );
        
        anchor_lang::solana_program::program::invoke(
            &transfer_ix,
            &[
                ctx.accounts.user.to_account_info(),
                ctx.accounts.basket.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;

        // Execute Jupiter swaps
        let basket = &ctx.accounts.basket;
        let initial_lamports = ctx.accounts.basket.to_account_info().lamports();

        for (i, token_info) in basket.tokens.iter().enumerate() {
            let token_amount = (amount as u128 * token_info.weight as u128 / 100) as u64;
            
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
                    }
                )).unwrap(),
            };
            
            // Execute swap
            anchor_lang::solana_program::program::invoke(
                &swap_ix,
                &ctx.remaining_accounts[i * 12..(i + 1) * 12],
            )?;

            // Verify minimum received
            let token_account = &ctx.remaining_accounts[i * 12 + 1];
            let token_balance = Account::<TokenAccount>::try_from(token_account)?.amount;
            require!(
                token_balance >= minimum_token_amounts[i],
                BasketError::SlippageExceeded
            );
        }

        // Mint basket tokens to user
        let seeds = &[b"basket".as_ref(), &[ctx.accounts.basket.bump]];

        let cpi_accounts = token::MintTo {
            mint: ctx.accounts.basket_mint.to_account_info(),
            to: ctx.accounts.user_basket_token.to_account_info(),
            authority: ctx.accounts.basket.to_account_info(),
        };

        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts,
                &[seeds]
            ),
            amount,
        )?;

        // Update state
        let basket = &mut ctx.accounts.basket;
        basket.total_supply += amount;
        
        Ok(())
    }

    pub fn redeem(
        ctx: Context<Redeem>,
        amount: u64,
        jupiter_quote: [u8; 32],
        slippage_bps: u16,
        minimum_sol_amount: u64
    ) -> Result<()> {
        let redemption_ratio = (amount as u128 * MAGNIFIER as u128) / ctx.accounts.basket.total_supply as u128;
        
        // Burn basket tokens first
        let seeds = &[b"basket".as_ref(), &[ctx.accounts.basket.bump]];
        
        token::burn(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token::Burn {
                    mint: ctx.accounts.basket_mint.to_account_info(),
                    from: ctx.accounts.user_basket_token.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
                &[seeds]
            ),
            amount
        )?;

        // Sell tokens back to SOL
        let mut total_sol_received = 0u64;
        let initial_basket_lamports = ctx.accounts.basket.to_account_info().lamports();
        
        for (i, _) in ctx.accounts.basket.tokens.iter().enumerate() {
            let token_account = &ctx.remaining_accounts[i * 12 + 1];
            let token_amount = Account::<TokenAccount>::try_from(token_account)?.amount;
            let redeem_amount = (token_amount as u128 * redemption_ratio / MAGNIFIER as u128) as u64;

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
                    }
                )).unwrap(),
            };
            
            anchor_lang::solana_program::program::invoke(
                &swap_ix,
                &ctx.remaining_accounts[i * 12..(i + 1) * 12],
            )?;

            // Track SOL received
            let current_lamports = ctx.accounts.basket.to_account_info().lamports();
            let sol_received = current_lamports
                .checked_sub(initial_basket_lamports)
                .ok_or(BasketError::MathOverflow)?;
            total_sol_received += sol_received;
        }

        require!(
            total_sol_received >= minimum_sol_amount,
            BasketError::SlippageExceeded
        );

        // Transfer SOL to user
        **ctx.accounts.basket.to_account_info().try_borrow_mut_lamports()? -= total_sol_received;
        **ctx.accounts.user.to_account_info().try_borrow_mut_lamports()? += total_sol_received;

        // Update state at the end
        let basket = &mut ctx.accounts.basket;
        basket.total_supply -= amount;
        
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + 32 + 32 + 4 + 1000 + 1, // Added space for token info
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

#[account]
#[derive(Default)]
pub struct BasketState {
    pub authority: Pubkey,
    pub tokens: Vec<TokenInfo>,
    pub total_supply: u64,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct TokenInfo {
    pub mint: Pubkey,
    pub weight: u8,  // Percentage weight in basket
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
    #[msg("Slippage exceeded")]
    SlippageExceeded,
}