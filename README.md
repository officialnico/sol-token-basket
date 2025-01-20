# Installation

```bash
#Install Solana CLI
sh -c "$(curl -sSfL https://release.solana.com/stable/install)"

# Install Anchor CLI
cargo install --git https://github.com/coral-xyz/anchor avm --locked
avm install latest
avm use latest
```

- make sure solana is in your path

Run solana test validator in separate terminal 

`solana-test-validator`

Run tests 

`cargo test -- --nocapture`
