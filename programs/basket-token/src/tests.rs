#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::solana_program::{system_program, system_instruction};
    use anchor_spl::associated_token;
    use solana_program_test::*;
    use solana_sdk::{signature::Keypair, signer::Signer};

    // Mock Jupiter Program
    pub fn process_jupiter_instruction(
        _program_id: &Pubkey,
        accounts: &[AccountInfo],
        instruction_data: &[u8],
    ) -> Result<()> {
        // Deserialize instruction data to get swap amount
        let (_discriminator, params) = AnchorSerialize::try_to_vec(&(
            4u8,
            jupiter::RouteSwapParams::try_from_slice(&instruction_data[1..]).unwrap()
        )).unwrap();

        let source_account = &accounts[0];
        let dest_account = &accounts[1];
        
        // Mock Jupiter's swap behavior with 1:1 rate for simplicity
        let amount = params.in_amount;

        **dest_account.try_borrow_mut_lamports()? += amount;
        **source_account.try_borrow_mut_lamports()? -= amount;

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
        context.banks_client
            .process_transaction(Transaction::new_signed_with_payer(
                &[system_instruction::transfer(
                    &context.payer.pubkey(),
                    &payer.pubkey(),
                    10_000_000_000, // 10 SOL
                )],
                Some(&context.payer.pubkey()),
                &[&context.payer],
                context.last_blockhash,
            ))
            .await
            .unwrap();

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

    async fn add_token_to_basket(
        context: &mut ProgramTestContext,
        payer: &Keypair,
        basket_pda: &Pubkey,
        token_mint: &Pubkey,
        weight: u8,
    ) -> Result<()> {
        let accounts = AddToken {
            basket: *basket_pda,
            authority: payer.pubkey(),
        };

        let ix = Instruction::new_with_bytes(
            id(),
            &anchor_lang::InstructionData::data(&basket_token::instruction::AddToken {
                token_mint: *token_mint,
                weight,
            }),
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

        // Verify mint state
        let mint_account = context.banks_client
            .get_account(mint_pda)
            .await
            .unwrap()
            .unwrap();
        let mint = Mint::unpack(&mint_account.data[..]).unwrap();
        assert_eq!(mint.decimals, 9);
    }

    #[tokio::test]
    async fn test_add_token() {
        let (mut context, payer, basket_pda, mint_pda) = setup().await;
        initialize_basket(&mut context, &payer, &basket_pda, &mint_pda)
            .await
            .unwrap();

        let token_mint = Keypair::new();
        add_token_to_basket(&mut context, &payer, &basket_pda, &token_mint.pubkey(), 50)
            .await
            .unwrap();

        // Verify token was added
        let basket_account = context.banks_client
            .get_account(basket_pda)
            .await
            .unwrap()
            .unwrap();

        let basket_state = BasketState::try_deserialize(&mut &basket_account.data[..]).unwrap();
        assert_eq!(basket_state.tokens.len(), 1);
        assert_eq!(basket_state.tokens[0].mint, token_mint.pubkey());
        assert_eq!(basket_state.tokens[0].weight, 50);
    }

    #[tokio::test]
    async fn test_weight_overflow() {
        let (mut context, payer, basket_pda, mint_pda) = setup().await;
        initialize_basket(&mut context, &payer, &basket_pda, &mint_pda)
            .await
            .unwrap();

        // Add first token with 60% weight
        let token1 = Keypair::new();
        add_token_to_basket(&mut context, &payer, &basket_pda, &token1.pubkey(), 60)
            .await
            .unwrap();

        // Try to add second token with 50% weight (should fail)
        let token2 = Keypair::new();
        let result = add_token_to_basket(&mut context, &payer, &basket_pda, &token2.pubkey(), 50)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_minimum_deposit() {
        let (mut context, payer, basket_pda, mint_pda) = setup().await;
        initialize_basket(&mut context, &payer, &basket_pda, &mint_pda)
            .await
            .unwrap();

        let token_mint = Keypair::new();
        add_token_to_basket(&mut context, &payer, &basket_pda, &token_mint.pubkey(), 100)
            .await
            .unwrap();

        let user = Keypair::new();
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

        // Try to deposit less than minimum
        let deposit_amount = MINIMUM_DEPOSIT - 1;
        let jupiter_quote = [0u8; 32];
        let slippage_bps = 100;
        let minimum_token_amounts = vec![100_000];

        let ix = Instruction::new_with_bytes(
            id(),
            &anchor_lang::InstructionData::data(&basket_token::instruction::Deposit {
                amount: deposit_amount,
                jupiter_quote,
                slippage_bps,
                minimum_token_amounts,
            }),
            accounts.to_account_metas(None),
        );

        let transaction = Transaction::new_signed_with_payer(
            &[ix],
            Some(&user.pubkey()),
            &[&user],
            context.last_blockhash,
        );

        let err = context.banks_client.process_transaction(transaction).await.unwrap_err();
        assert!(matches!(err, BanksClientError::TransactionError(_)));
    }

    #[tokio::test]
    async fn test_multi_token_deposit() {
        let (mut context, payer, basket_pda, mint_pda) = setup().await;
        initialize_basket(&mut context, &payer, &basket_pda, &mint_pda)
            .await
            .unwrap();

        // Add two tokens with equal weights
        let token1 = Keypair::new();
        let token2 = Keypair::new();
        add_token_to_basket(&mut context, &payer, &basket_pda, &token1.pubkey(), 50)
            .await
            .unwrap();
        add_token_to_basket(&mut context, &payer, &basket_pda, &token2.pubkey(), 50)
            .await
            .unwrap();

        let user = Keypair::new();
        // Airdrop SOL to user
        context.banks_client
            .process_transaction(Transaction::new_signed_with_payer(
                &[system_instruction::transfer(
                    &context.payer.pubkey(),
                    &user.pubkey(),
                    2_000_000_000, // 2 SOL
                )],
                Some(&context.payer.pubkey()),
                &[&context.payer],
                context.last_blockhash,
            ))
            .await
            .unwrap();

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

    #[tokio::test]
    async fn test_multi_token_redeem() {
        let (mut context, payer, basket_pda, mint_pda) = setup().await;
        initialize_basket(&mut context, &payer, &basket_pda, &mint_pda)
            .await
            .unwrap();

        // Add tokens and deposit first
        let token1 = Keypair::new();
        let token2 = Keypair::new();
        add_token_to_basket(&mut context, &payer, &basket_pda, &token1.pubkey(), 50)
            .await
            .unwrap();
        add_token_to_basket(&mut context, &payer, &basket_pda, &token2.pubkey(), 50)
            .await
            .unwrap();

        // ... (Previous deposit setup)

        let redeem_amount = 500_000_000; // Redeem half
        let jupiter_quote = [0u8; 32];
        let slippage_bps = 100;
        let minimum_sol_amount = 450_000_000;

        let accounts = Redeem {
            basket: basket_pda,
            basket_mint: mint_pda,
            user_basket_token: get_associated_token_address(&payer.pubkey(), &mint_pda),
            user: payer.pubkey(),
            system_program: system_program::ID,
            token_program: token::ID,
        };

        // Create mock Jupiter accounts
        let mock_accounts = create_mock_jupiter_accounts(&mut context, 2).await;
        let mut all_accounts = accounts.to_account_metas(None);
        all_accounts.extend(mock_accounts);

        let ix = Instruction::new_with_bytes(
            id(),
            &anchor_lang::InstructionData::data(&basket_token::instruction::Redeem {
                amount: redeem_amount,
                jupiter_quote,
                slippage_bps,
                minimum_sol_amount,
            }),
            all_accounts,
        );

        let transaction = Transaction::new_signed_with_payer(
            &[ix],
            Some(&payer.pubkey()),
            &[&payer],
            context.last_blockhash,
        );

        context.banks_client.process_transaction(transaction).await.unwrap();

        // Verify SOL balance increased
        let user_account = context.banks_client
            .get_account(payer.pubkey())
            .await
            .unwrap()
            .unwrap();
        assert!(user_account.lamports >= minimum_sol_amount);
    }

    #[tokio::test]
    async fn test_slippage_protection() {
        let (mut context, payer, basket_pda, mint_pda) = setup().await;
        initialize_basket(&mut context, &payer, &basket_pda, &mint_pda)
            .await
            .unwrap();

        // Add a token
        let token_mint = Keypair::new();
        add_token_to_basket(&mut context, &payer, &basket_pda, &token_mint.pubkey(), 100)
            .await
            .unwrap();

        // Set up deposit with very high minimum token amount
        let deposit_amount = 1_000_000_000;
        let jupiter_quote = [0u8; 32];
        let slippage_bps = 100;
        let minimum_token_amounts = vec![deposit_amount * 2]; // Impossible to meet this minimum

        let user_basket_token = get_associated_token_address(&payer.pubkey(), &mint_pda);

        let accounts = Deposit {
            basket: basket_pda,
            basket_mint: mint_pda,
            user_basket_token,
            user: payer.pubkey(),
            system_program: system_program::ID,
            token_program: token::ID,
            associated_token_program: associated_token::ID,
        };

        // Create mock Jupiter accounts
        let mock_accounts = create_mock_jupiter_accounts(&mut context, 1).await;
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
            Some(&payer.pubkey()),
            &[&payer],
            context.last_blockhash,
        );

        let err = context.banks_client.process_transaction(transaction).await.unwrap_err();
        assert!(matches!(err, BanksClientError::TransactionError(_)));
    }

    #[tokio::test]
    async fn test_insufficient_sol() {
        let (mut context, _, basket_pda, mint_pda) = setup().await;
        initialize_basket(&mut context, &context.payer, &basket_pda, &mint_pda)
            .await
            .unwrap();

        // Create user with very little SOL
        let poor_user = Keypair::new();
        context.banks_client
            .process_transaction(Transaction::new_signed_with_payer(
                &[system_instruction::transfer(
                    &context.payer.pubkey(),
                    &poor_user.pubkey(),
                    1_000_000, // Only 0.001 SOL
                )],
                Some(&context.payer.pubkey()),
                &[&context.payer],
                context.last_blockhash,
            ))
            .await
            .unwrap();

        let token_mint = Keypair::new();
        add_token_to_basket(&mut context, &context.payer, &basket_pda, &token_mint.pubkey(), 100)
            .await
            .unwrap();

        let user_basket_token = get_associated_token_address(&poor_user.pubkey(), &mint_pda);

        let accounts = Deposit {
            basket: basket_pda,
            basket_mint: mint_pda,
            user_basket_token,
            user: poor_user.pubkey(),
            system_program: system_program::ID,
            token_program: token::ID,
            associated_token_program: associated_token::ID,
        };

        let deposit_amount = 1_000_000_000; // Try to deposit 1 SOL
        let minimum_token_amounts = vec![100_000_000];

        let ix = Instruction::new_with_bytes(
            id(),
            &anchor_lang::InstructionData::data(&basket_token::instruction::Deposit {
                amount: deposit_amount,
                jupiter_quote: [0u8; 32],
                slippage_bps: 100,
                minimum_token_amounts,
            }),
            accounts.to_account_metas(None),
        );

        let transaction = Transaction::new_signed_with_payer(
            &[ix],
            Some(&poor_user.pubkey()),
            &[&poor_user],
            context.last_blockhash,
        );

        let err = context.banks_client.process_transaction(transaction).await.unwrap_err();
        assert!(matches!(err, BanksClientError::TransactionError(_)));
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

            // Fund the token account
            context.banks_client
                .process_transaction(Transaction::new_signed_with_payer(
                    &[system_instruction::transfer(
                        &context.payer.pubkey(),
                        &token_account.pubkey(),
                        1_000_000_000,
                    )],
                    Some(&context.payer.pubkey()),
                    &[&context.payer],
                    context.last_blockhash,
                ))
                .await
                .unwrap();

            // Add accounts needed by Jupiter (simplified for testing)
            accounts.push(AccountMeta::new(token_account.pubkey(), false));
            accounts.push(AccountMeta::new(destination.pubkey(), false));
            // Add other required Jupiter accounts (simplified)
            for _ in 0..10 {
                accounts.push(AccountMeta::new(Keypair::new().pubkey(), false));
            }
        }
        accounts
    }
}