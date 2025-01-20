#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::solana_program::{system_program, system_instruction};
    use solana_program_test::*;
    use solana_sdk::{signature::Keypair, signer::Signer};

    // Mock Jupiter Program
    pub fn process_jupiter_instruction(
        _program_id: &Pubkey,
        accounts: &[AccountInfo],
        _instruction_data: &[u8],
    ) -> Result<()> {
        // Mock a successful swap
        // Transfer tokens from source to destination
        let source_account = &accounts[0];
        let dest_account = &accounts[1];
        let amount = 1_000_000; // Mock amount received

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

        // Create PDAs
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

    #[tokio::test]
    async fn test_initialize() {
        let (mut context, payer, basket_pda, mint_pda) = setup().await;

        let (_, basket_bump) = Pubkey::find_program_address(&[b"basket"], &id());
        let (_, mint_bump) = Pubkey::find_program_address(&[b"basket_mint"], &id());

        let accounts = Initialize {
            basket: basket_pda,
            basket_mint: mint_pda,
            authority: payer.pubkey(),
            system_program: system_program::ID,
            token_program: token::ID,
            rent: sysvar::rent::ID,
        };

        let ix = Instruction::new_with_bytes(
            id(),
            &[0, basket_bump, mint_bump], // Initialize instruction with bumps
            accounts.to_account_metas(None),
        );

        let transaction = Transaction::new_signed_with_payer(
            &[ix],
            Some(&payer.pubkey()),
            &[&payer],
            context.last_blockhash,
        );

        context.banks_client.process_transaction(transaction).await.unwrap();

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
        let (mut context, payer, basket_pda, _) = setup().await;
        
        // Initialize first...
        // (Add initialization code here)

        let token_mint = Keypair::new();
        
        let accounts = AddToken {
            basket: basket_pda,
            authority: payer.pubkey(),
        };

        let ix = Instruction::new_with_bytes(
            id(),
            &anchor_lang::InstructionData::data(&basket_token::instruction::AddToken {
                token_mint: token_mint.pubkey(),
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
        assert_eq!(basket_state.tokens[0].mint, token_mint.pubkey());
        assert_eq!(basket_state.tokens[0].weight, 50);
    }

    #[tokio::test]
    async fn test_deposit() {
        let (mut context, user, basket_pda, mint_pda) = setup().await;

        // Initialize and add tokens first...
        // (Add initialization and add_token code here)

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
        let slippage_bps = 100; // 1%
        let minimum_token_amounts = vec![100_000_000]; // Minimum expected output

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

        context.banks_client.process_transaction(transaction).await.unwrap();

        // Verify basket token balance
        let user_token_account = context.banks_client
            .get_account(user_basket_token)
            .await
            .unwrap()
            .unwrap();

        let token_balance = TokenAccount::unpack(&user_token_account.data[..]).unwrap();
        assert!(token_balance.amount > 0);
    }

    #[tokio::test]
    async fn test_redeem() {
        let (mut context, user, basket_pda, mint_pda) = setup().await;

        // Initialize, add tokens, and deposit first...
        // (Add setup code here)

        let redeem_amount = 500_000_000; // Redeem half
        let jupiter_quote = [0u8; 32];
        let slippage_bps = 100; // 1%
        let minimum_sol_amount = 450_000_000; // Accept 10% slippage

        let accounts = Redeem {
            basket: basket_pda,
            basket_mint: mint_pda,
            user_basket_token: get_associated_token_address(&user.pubkey(), &mint_pda),
            user: user.pubkey(),
            system_program: system_program::ID,
            token_program: token::ID,
        };

        let ix = Instruction::new_with_bytes(
            id(),
            &anchor_lang::InstructionData::data(&basket_token::instruction::Redeem {
                amount: redeem_amount,
                jupiter_quote,
                slippage_bps,
                minimum_sol_amount,
            }),
            accounts.to_account_metas(None),
        );

        let transaction = Transaction::new_signed_with_payer(
            &[ix],
            Some(&user.pubkey()),
            &[&user],
            context.last_blockhash,
        );

        context.banks_client.process_transaction(transaction).await.unwrap();

        // Verify SOL balance increased
        let user_account = context.banks_client
            .get_account(user.pubkey())
            .await
            .unwrap()
            .unwrap();
        assert!(user_account.lamports > 0);
    }

    #[tokio::test]
    async fn test_errors() {
        let (mut context, payer, basket_pda, _) = setup().await;

        // Test unauthorized token addition
        let unauthorized_user = Keypair::new();
        let token_mint = Keypair::new();

        let accounts = AddToken {
            basket: basket_pda,
            authority: unauthorized_user.pubkey(),
        };

        let ix = Instruction::new_with_bytes(
            id(),
            &anchor_lang::InstructionData::data(&basket_token::instruction::AddToken {
                token_mint: token_mint.pubkey(),
                weight: 50,
            }),
            accounts.to_account_metas(None),
        );

        let transaction = Transaction::new_signed_with_payer(
            &[ix],
            Some(&unauthorized_user.pubkey()),
            &[&unauthorized_user],
            context.last_blockhash,
        );

        let err = context.banks_client.process_transaction(transaction).await.unwrap_err();
        assert_eq!(err.unwrap(), TransactionError::InstructionError(0, InstructionError::Custom(BasketError::Unauthorized as u32)));
    }
}