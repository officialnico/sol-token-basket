import { Connection, Keypair, VersionedTransaction } from '@solana/web3.js';
import fetch from 'cross-fetch';
import { Wallet } from '@project-serum/anchor';
import bs58 from 'bs58';
/*
* This program will convert a bit of SOL to USDC
*/
const connection = new Connection("https://api.mainnet-beta.solana.com");

const devKey = '<YOUR-PRIVATE-KEY-HERE>'
const keyPair = Keypair.fromSecretKey(bs58.decode(devKey|| ''));
const wallet = new Wallet(keyPair);

const quote = async () => {
    const req = await fetch(
        `https://quote-api.jup.ag/v6/quote?inputMint=So11111111111111111111111111111111111111112&outputMint=EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v&amount=5000000&slippageBps=50`
    );

    const quoteResponse = await req.json()
    console.log("Quote Response:", quoteResponse);
    return quoteResponse
}


const swapTransaction = async (payload: any) => {
    const swapReq = await fetch("https://quote-api.jup.ag/v6/swap", {
        method: "POST",
        headers: {
            "Content-Type": "application/json"
        },
        body: JSON.stringify({
            userPublicKey: wallet.publicKey,
            quoteResponse: payload,
            wrapAndUnwrapSol: true
        })
    });

    const swapRes = await swapReq.json();
    console.log("Swap Response:", swapRes);
    return swapRes as {
        swapTransaction: string;
    }
};

const signAndSendSwap = async (base64Transaction) => {
    try {
        // Decode Base64 Transaction
        const transactionBuffer = Buffer.from(base64Transaction, "base64");
        const transaction = VersionedTransaction.deserialize(transactionBuffer);

        // Sign transaction
        console.log("signing tx")
        transaction.sign([keyPair]);

        // Send transaction to Solana
        const txid = await connection.sendTransaction(transaction, { skipPreflight: false });
        console.log("Transaction ID:", txid);

        // Confirm transaction
        await connection.confirmTransaction(txid, "confirmed");
        console.log("Swap successful!");
    } catch (error) {
        console.error("Swap failed:", error);
    }
};



const main = async () => {
    const responseData = await quote();
    const swapData = await swapTransaction(responseData)
    const response = await signAndSendSwap(swapData.swapTransaction)
    console.log(response);
}


main();