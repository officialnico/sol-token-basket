import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { BasketToken } from "../target/types/basket_token";

describe("basket_token", () => {
  // Configure the client to use the local cluster.
  anchor.setProvider(anchor.AnchorProvider.env());
  const program = anchor.workspace.BasketToken as Program<BasketToken>;
  console.log(program)
  it("Is initialized!", async () => {
    // Add your test here.
    const tx = await program.methods.initialize(100000)
    console.log("Your transaction signature", tx);
  });
});
