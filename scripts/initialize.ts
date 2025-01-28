import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { Program, AnchorProvider, web3 } from "@project-serum/anchor";
import { SystemProgram } from "@solana/web3.js";
import { IDL } from "../target/types/basket_token";

const provider = AnchorProvider.env();
const program = new Program(IDL, "Your_Program_ID", provider);

(async () => {
  const [basketPda, basketBump] = await web3.PublicKey.findProgramAddress(
    [Buffer.from("basket")],
    program.programId
  );

  const [basketMintPda, basketMintBump] = await web3.PublicKey.findProgramAddress(
    [Buffer.from("basket_mint")],
    program.programId
  );

  try {
    await program.methods
      .initialize(5) // 5 is the `max_tokens` argument
      .accounts({
        basket: basketPda,
        basketMint: basketMintPda,
        authority: provider.wallet.publicKey,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: web3.SYSVAR_RENT_PUBKEY,
      })
      .rpc();
    console.log(`Basket initialized at ${basketPda.toBase58()}`);
  } catch (err) {
    console.error("Error initializing basket:", err);
  }
})();