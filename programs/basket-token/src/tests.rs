#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::solana_program::{system_program, system_instruction};
    use anchor_spl::{token, associated_token};
    use solana_program_test::*;
    use solana_sdk::{signature::Keypair, signer::Signer};
    use anchor_spl::token::{Mint, Token};
    use spl_associated_token_account::get_associated_token_address;

    // Mock Jupiter Program
    pub fn process_jupiter_instruction(
        _program_id: &Pubkey,
        accounts: &[AccountInfo],
        instruction_data: &[u8],
    ) -> Result<()> {
        let source_info = accounts[0];
        let destination_info = accounts[1];
        
        let (_disc, params) = jupiter::RouteSwapParams::try_from_slice(&instruction_data[1..]).unwrap();
        
        if source_info.is_writable() && destination_info.is_writable() {
            let amount = params.in_amount;
            **destination_info.try_borrow_mut_lamports()? += amount;
            **source_info.try_borrow_mut_lamports()? = source_info
                .lamports()
                .checked_sub(amount)
                .ok_or(BasketError::MathOverflow)?;
        }
        
        Ok(())
    }

    async fn setup() -> (ProgramTestContext, Keypair, Pubkey, Pubkey) {
        let program_id = id();
        let mut program_test = ProgramTest::new(
            "basket_token",
            program_id,
            processor!(basket_token::entry),
        );

        // Add mock Jupiter program
        program_test.add_program(
            "jupiter",
            jupiter::JUPITER_V6_ID,
            processor!(process_jupiter_instruction),
        );

        let mut context = program_test.start_with_context().await;
        let payer = Keypair::new();

        // Airdrop SOL to payer
        airdrop_sol(&mut context, &payer.pubkey(), 10_000_000_000).await;

        let (basket_pda, _) = Pubkey::find_program_address(
            &[b"basket"],
            &program_id,
        );

        let (mint_pda, _) = Pubkey::find_program_address(
            &[b"basket_mint"],
            &program_id,
        );

        (context, payer, basket_pda, mint_pda)
    }

    async fn create_mint(
        context: &mut ProgramTestContext,
        payer: &Keypair
    ) -> Pubkey {
        let mint = Keypair::new();
        let rent = context.banks_client.get_rent().await.unwrap();
        
        let ix = system_instruction::create_account(
            &payer.pubkey(),
            &mint.pubkey(),
            rent.minimum_balance(Mint::LEN),
            Mint::LEN as u64,
            &token::ID,
        );
        
        let initialize_ix = token::instruction::initialize_mint(
            &token::ID,
            &mint.pubkey(),
            &payer.pubkey(),
            None,
            9,
        ).unwrap();

        let transaction = Transaction::new_signed_with_payer(
            &[ix, initialize_ix],
            Some(&payer.pubkey()),
            &[payer, &mint],
            context.last_blockhash,
        );

        context.banks_client.process_transaction(transaction).await.unwrap();
        mint.pubkey()
    }

    async fn create_token_account(
        context: &mut ProgramTestContext,
        mint: &Pubkey,
        owner: &Pubkey,
    ) -> Pubkey {
        let account = get_associated_token_address(owner, mint);
        let create_ix = associated_token::instruction::create_associated_token_account(
            &context.payer.pubkey(),
            owner,
            mint,
        );

        let transaction = Transaction::new_signed_with_payer(
            &[create_ix],
            Some(&context.payer.pubkey()),
            &[&context.payer],
            context.last_blockhash,
        );

        context.banks_client.process_transaction(transaction).await.unwrap();
        account
    }

    async fn airdrop_sol(
        context: &mut ProgramTestContext,
        to: &Pubkey,
        amount: u64,
    ) {
        let transfer_ix = system_instruction::transfer(
            &context.payer.pubkey(),
            to,
            amount,
        );

        let transaction = Transaction::new_signed_with_payer(
            &[transfer_ix],
            Some(&context.payer.pubkey()),
            &[&context.payer],
            context.last_blockhash,
        );

        context.banks_client.process_transaction(transaction).await.unwrap();
    }

    async fn initialize_basket(
        context: &mut ProgramTestContext,
        payer: &Keypair,
        basket_pda: &Pubkey,
        mint_pda: &Pubkey,
    ) -> Result<()> {
        let accounts = Initialize {
            basket: *basket_pda,
            basket_mint: *mint_pda,
            authority: payer.pubkey(),
            system_program: system_program::ID,
            token_program: token::ID,
            rent: sysvar::rent::ID,
        };

        let ix = Instruction::new_with_bytes(
            id(),
            &[0], // Initialize instruction
            accounts.to_account_metas(None),
        );

        let transaction = Transaction::new_signed_with_payer(
            &[ix],
            Some(&payer.pubkey()),
            &[payer],
            context.last_blockhash,
        );

        context.banks_client.process_transaction(transaction).await?;
        Ok(())
    }

    // Helper function to create mock Jupiter accounts
    async fn create_mock_jupiter_accounts(
        context: &mut ProgramTestContext,
        num_pairs: usize,
    ) -> Vec<AccountMeta> {
        let mut accounts = Vec::new();
        for _ in 0..num_pairs {
            let token_account = Keypair::new();
            let destination = Keypair::new();

            // Fund accounts
            airdrop_sol(context, &token_account.pubkey(), 1_000_000_000).await;
            airdrop_sol(context, &destination.pubkey(), 1_000_000_000).await;

            // Add main accounts
            accounts.push(AccountMeta::new(token_account.pubkey(), false));
            accounts.push(AccountMeta::new(destination.pubkey(), false));
            
            // Add remaining required accounts (mocked)
            for _ in 0..10 {
                let mock_account = Keypair::new();
                accounts.push(AccountMeta::new(mock_account.pubkey(), false));
            }
        }
        accounts
    }

    #[tokio::test]
    async fn test_initialize() {
        let (mut context, payer, basket_pda, mint_pda) = setup().await;
        initialize_basket(&mut context, &payer, &basket_pda, &mint_pda)
            .await
            .unwrap();

        // Verify basket state
        let basket_account = context.banks_client
            .get_account(basket_pda)
            .await
            .unwrap()
            .unwrap();

        let basket_state = BasketState::try_deserialize(&mut &basket_account.data[..]).unwrap();
        assert_eq!(basket_state.authority, payer.pubkey());
        assert_eq!(basket_state.tokens.len(), 0);
        assert_eq!(basket_state.total_supply, 0);
    }

    #[tokio::test]
    async fn test_add_token() {
        let (mut context, payer, basket_pda, mint_pda) = setup().await;
        initialize_basket(&mut context, &payer, &basket_pda, &mint_pda)
            .await
            .unwrap();

        let token_mint = create_mint(&mut context, &payer).await;
        
        // Add token with 50% weight
        let accounts = AddToken {
            basket: basket_pda,
            authority: payer.pubkey(),
        };

        let ix = Instruction::new_with_bytes(
            id(),
            &anchor_lang::InstructionData::data(&basket_token::instruction::AddToken {
                token_mint,
                weight: 50,
            }),
            accounts.to_account_metas(None),
        );

        let transaction = Transaction::new_signed_with_payer(
            &[ix],
            Some(&payer.pubkey()),
            &[&payer],
            context.last_blockhash,
        );

        context.banks_client.process_transaction(transaction).await.unwrap();

        // Verify token was added
        let basket_account = context.banks_client
            .get_account(basket_pda)
            .await
            .unwrap()
            .unwrap();

        let basket_state = BasketState::try_deserialize(&mut &basket_account.data[..]).unwrap();
        assert_eq!(basket_state.tokens.len(), 1);
        assert_eq!(basket_state.tokens[0].mint, token_mint);
        assert_eq!(basket_state.tokens[0].weight, 50);
    }

    #[tokio::test]
    async fn test_deposit() {
        let (mut context, payer, basket_pda, mint_pda) = setup().await;
        initialize_basket(&mut context, &payer, &basket_pda, &mint_pda)
            .await
            .unwrap();

        // Create and add two tokens
        let token1 = create_mint(&mut context, &payer).await;
        let token2 = create_mint(&mut context, &payer).await;

        // Add tokens with equal weights
        let add_token_ix1 = Instruction::new_with_bytes(
            id(),
            &anchor_lang::InstructionData::data(&basket_token::instruction::AddToken {
                token_mint: token1,
                weight: 50,
            }),
            AddToken {
                basket: basket_pda,
                authority: payer.pubkey(),
            }.to_account_metas(None),
        );

        let add_token_ix2 = Instruction::new_with_bytes(
            id(),
            &anchor_lang::InstructionData::data(&basket_token::instruction::AddToken {
                token_mint: token2,
                weight: 50,
            }),
            AddToken {
                basket: basket_pda,
                authority: payer.pubkey(),
            }.to_account_metas(None),
        );

        let transaction = Transaction::new_signed_with_payer(
            &[add_token_ix1, add_token_ix2],
            Some(&payer.pubkey()),
            &[&payer],
            context.last_blockhash,
        );

        context.banks_client.process_transaction(transaction).await.unwrap();

        // Set up deposit
        let user = Keypair::new();
        airdrop_sol(&mut context, &user.pubkey(), 2_000_000_000).await;

        let user_basket_token = get_associated_token_address(&user.pubkey(), &mint_pda);

        let accounts = Deposit {
            basket: basket_pda,
            basket_mint: mint_pda,
            user_basket_token,
            user: user.pubkey(),
            system_program: system_program::ID,
            token_program: token::ID,
            associated_token_program: associated_token::ID,
        };

        let deposit_amount = 1_000_000_000; // 1 SOL
        let jupiter_quote = [0u8; 32];
        let slippage_bps = 100;
        let minimum_token_amounts = vec![100_000_000, 100_000_000];

        // Create mock Jupiter accounts
        let mock_accounts = create_mock_jupiter_accounts(&mut context, 2).await;
        let mut all_accounts = accounts.to_account_metas(None);
        all_accounts.extend(mock_accounts);

        let ix = Instruction::new_with_bytes(
            id(),
            &anchor_lang::InstructionData::data(&basket_token::instruction::Deposit {
                amount: deposit_amount,
                jupiter_quote,
                slippage_bps,
                minimum_token_amounts,
            }),
            all_accounts,
        );

        let transaction = Transaction::new_signed_with_payer(
            &[ix],
            Some(&user.pubkey()),
            &[&user],
            context.last_blockhash,
        );

        context.banks_client.process_transaction(transaction).await.unwrap();

        // Verify basket token balance
        let user_token_account = context.banks_client
            .get_account(user_basket_token)
            .await
            .unwrap()
            .unwrap();

        let token_balance = TokenAccount::unpack(&user_token_account.data[..]).unwrap();
        assert_eq!(token_balance.amount, deposit_amount);
    }
}