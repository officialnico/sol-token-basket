import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { BasketToken } from "../target/types/basket_token";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { expect } from "chai";

describe("BasketToken Tests", () => {
  anchor.setProvider(anchor.AnchorProvider.env());

  const program = anchor.workspace.BasketToken as Program<BasketToken>;
  const provider = anchor.getProvider();
  const payer = provider.wallet as anchor.Wallet;

  let basketState: anchor.web3.PublicKey;
  let basketMint: anchor.web3.PublicKey;
  let tokenMint: anchor.web3.PublicKey;
  let userTokenAccount: anchor.web3.PublicKey;
  let userBasketToken: anchor.web3.PublicKey;
  let basketBump: number;

  before(async () => {
    // Derive Basket PDA
    [basketState, basketBump] = await PublicKey.findProgramAddressSync(
      [Buffer.from("basket")],
      program.programId
    );

    // Derive Basket Mint PDA
    [basketMint] = await PublicKey.findProgramAddressSync(
      [Buffer.from("basket_mint")],
      program.programId
    );

    // Create a dummy token mint
    tokenMint = anchor.web3.Keypair.generate().publicKey;

    // Derive user's associated basket token account
    [userBasketToken] = await PublicKey.findProgramAddressSync(
      [payer.publicKey.toBuffer(), tokenMint.toBuffer()],
      program.programId
    );

    console.log("Basket State:", basketState.toBase58());
    console.log("Basket Mint:", basketMint.toBase58());
    console.log("User Basket Token:", userBasketToken.toBase58());
  });

  it("Initializes BasketToken with max_tokens = 5", async () => {
    const tx = await program.methods
      .initialize(5)
      .accounts({
        basket: basketState,
        basketMint,
        authority: payer.publicKey,
        systemProgram: SystemProgram.programId,
        tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    console.log("✅ BasketToken initialized with tx:", tx);
  });

  it("Adds a token to the basket", async () => {
    const tx = await program.methods
      .addToken(tokenMint, 10)
      .accounts({
        basket: basketState,
        authority: payer.publicKey,
      })
      .rpc();

    console.log("✅ Token added to the basket with tx:", tx);
  });

  it("Fails to add duplicate token", async () => {
    try {
      await program.methods
        .addToken(tokenMint, 15)
        .accounts({
          basket: basketState,
          authority: payer.publicKey,
        })
        .rpc();
    } catch (err) {
      console.log("✅ Expected failure: Cannot add duplicate token");
      expect(err.message).to.include("Duplicate token");
    }
  });

  it("Removes a token from the basket", async () => {
    const tx = await program.methods
      .removeToken(tokenMint)
      .accounts({
        basket: basketState,
        authority: payer.publicKey,
      })
      .rpc();

    console.log("✅ Token removed from the basket with tx:", tx);
  });

  it("Fails to remove a non-existent token", async () => {
    try {
      await program.methods
        .removeToken(tokenMint)
        .accounts({
          basket: basketState,
          authority: payer.publicKey,
        })
        .rpc();
    } catch (err) {
      console.log("✅ Expected failure: Cannot remove non-existent token");
      expect(err.message).to.include("Token not found");
    }
  });

  it("Pauses the program", async () => {
    const tx = await program.methods
      .setPaused(true)
      .accounts({
        basket: basketState,
        authority: payer.publicKey,
      })
      .rpc();

    console.log("✅ BasketToken program paused with tx:", tx);
  });

  it("Fails to add token when program is paused", async () => {
    try {
      await program.methods
        .addToken(tokenMint, 10)
        .accounts({
          basket: basketState,
          authority: payer.publicKey,
        })
        .rpc();
    } catch (err) {
      console.log("✅ Expected failure: Cannot add token when paused");
      expect(err.message).to.include("Program paused");
    }
  });

  it("Resumes the program", async () => {
    const tx = await program.methods
      .setPaused(false)
      .accounts({
        basket: basketState,
        authority: payer.publicKey,
      })
      .rpc();

    console.log("✅ BasketToken program resumed with tx:", tx);
  });

});