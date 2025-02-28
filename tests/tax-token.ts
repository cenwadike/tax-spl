import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { TaxToken } from "../target/types/tax_token";
import { PublicKey, Keypair, SystemProgram, Signer, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { 
  TOKEN_2022_PROGRAM_ID, 
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createMint,
  TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  getOrCreateAssociatedTokenAccount,
  mintTo
} from "@solana/spl-token";
import { assert } from "chai";
import { ASSOCIATED_PROGRAM_ID } from "@coral-xyz/anchor/dist/cjs/utils/token";
import { readFileSync } from "fs";
import path from 'path';
import os from 'os';

function loadSecretKey(filePath: string): Uint8Array {
  try {
    const resolvedPath = filePath.startsWith('~') ? path.join(os.homedir(), filePath.slice(1)) : filePath;
    const keyData = readFileSync(resolvedPath, 'utf-8');
    const keyArray = JSON.parse(keyData);
    if (!Array.isArray(keyArray) || keyArray.length !== 64) {
      throw new Error('Invalid secret key format or size');
    }
    return Uint8Array.from(keyArray);
  } catch (error) {
    throw new Error(`Failed to load secret key: ${error.message}`);
  }
}

describe("tax-token", () => {
  // Configure the client to use the local cluster
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.TaxToken as Program<TaxToken>;
  
  // Generate a new keypair for the reward mint
  const tokenMintKeypair = Keypair.generate();
  const rewardMintKeypair = Keypair.generate();
  
  // We'll need some test wallets
  // const authority = provider.wallet;
  // const authority = Keypair.generate();
  const secretKeyFilePath = process.env.PAYER_SECRET_KEY || '~/.config/solana/id.json';
  const payerSecretKey = loadSecretKey(secretKeyFilePath);
  const authority = Keypair.fromSecretKey(payerSecretKey);
  
  // Test data
  const tokenName = "Tax Token";
  const tokenSymbol = "TAX";
  const tokenUri = "https://pbs.twimg.com/profile_images/1577719586040025131/p1zeCklU_400x400.jpg";
  const tokenDecimals = 9;
  const tokenTotalSupply = 1_000_000_000 * 10 ** tokenDecimals; // 1 billion tokens

  // Find PDA addresses
  const [statePda] = PublicKey.findProgramAddressSync(
    [Buffer.from("program_state")],
    program.programId
  );

  // const admin = Keypair.generate();
  // console.log("Admin: ", admin.publicKey.toString());
  const sig: Signer = {
    publicKey: authority.publicKey,
    secretKey: authority.secretKey
  }
  const mintSig: Signer = {
    publicKey: tokenMintKeypair.publicKey,
    secretKey: tokenMintKeypair.secretKey
  }

  it("Initializes the tax token", async () => {
    console.log("Starting tax token initialization test...");

    // await program.provider.connection.confirmTransaction(
    //   await program.provider.connection.requestAirdrop(
    //     admin.publicKey,
    //     3 * LAMPORTS_PER_SOL
    //   ),
    //   "confirmed"
    // );

    // await program.provider.connection.confirmTransaction(
    //   await program.provider.connection.requestAirdrop(
    //     authority.publicKey,
    //     3 * LAMPORTS_PER_SOL
    //   ),
    //   "confirmed"
    // );

    // await program.provider.connection.confirmTransaction(
    //   await program.provider.connection.requestAirdrop(
    //     program.provider.publicKey,
    //     3 * LAMPORTS_PER_SOL
    //   ),
    //   "confirmed"
    // );

    const tokenMint =  tokenMintKeypair.publicKey;   
    
    console.log("Creating reward mint...");
    // Create the reward mint first (this would typically be an existing token in a real scenario)
    const rewardMint = await createMint(
      provider.connection,
      authority,
      authority.publicKey,
      null,
      tokenDecimals,
      rewardMintKeypair,
      undefined,
      TOKEN_PROGRAM_ID
    );
    const rewardTokenAccountAddress = getAssociatedTokenAddressSync(rewardMint, authority.publicKey, false, TOKEN_PROGRAM_ID);

    mintTo(
      provider.connection,
      authority,
      rewardMint,
      rewardTokenAccountAddress,
      authority,
      tokenTotalSupply * 10 ** 9
    )

    console.log("Initializing tax token...");
    try {
      // Call the initialize function
      const initCtx = {
        state: statePda,
        tokenMint: tokenMintKeypair.publicKey,
        authority: authority.publicKey,
        rewardMint: rewardMint,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_2022_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      };

      const tx = await program.methods
        .initialize(
          {
            name: tokenName,
            symbol: tokenSymbol,
            uri: tokenUri,
            decimals: tokenDecimals,
            totalSupply: new anchor.BN(tokenTotalSupply.toString()),
          }
      )
        .accounts(initCtx)
        .signers([mintSig, sig])
        .rpc({ commitment: "confirmed" });
      
      console.log("Transaction signature:", tx);
      
      // Fetch the program state to verify initialization
      const state = await program.account.programState.fetch(statePda);
      const senderTokenAccountAddress = getAssociatedTokenAddressSync(tokenMintKeypair.publicKey, authority.publicKey, false, TOKEN_2022_PROGRAM_ID);
      
      // Assert the state was properly initialized
      assert.equal(state.authority.toString(), authority.publicKey.toString());
      assert.equal(state.tokenMint.toString(), tokenMint.toString());
      assert.equal(state.rewardMint.toString(), rewardMint.toString());

      console.log("Admin: ", authority.publicKey);
      console.log("Tax Program ID: ", program.programId);
      console.log("Tax token Mint: ", tokenMint);
      console.log("TaxTokenATA: ", senderTokenAccountAddress);
      console.log("Reward Mint: ", rewardMint);
      console.log("Reward ATA: ", rewardTokenAccountAddress);
      
      console.log("✅ Test passed - Tax token initialized successfully!");
    } catch (err) {
      console.error("Error in initialization:", err);
      throw err;
    }
  });

  it('Mint Tokens', async () => {
    try {
      const senderTokenAccountAddress = getAssociatedTokenAddressSync(tokenMintKeypair.publicKey, authority.publicKey, false, TOKEN_2022_PROGRAM_ID);

      await getOrCreateAssociatedTokenAccount(
        provider.connection,
        authority,
        tokenMintKeypair.publicKey,
        authority.publicKey,
        false,
        null,
        null,
        TOKEN_2022_PROGRAM_ID,
        ASSOCIATED_PROGRAM_ID,
      );

      const mintRes = await mintTo(provider.connection, authority, tokenMintKeypair.publicKey, senderTokenAccountAddress, authority, tokenTotalSupply, [], null, TOKEN_2022_PROGRAM_ID);  
      console.log("Transaction Signature: ", mintRes)
      console.log("✅ Mint passed - Token minted successfully!");
    } catch (error) {
      console.error("Error minting tokens");
      throw error
    }  
  });

  // it('Transfer', async () => {
  //   const recipient = Keypair.generate();

  //   const transactionSignature = await program.methods
  //     .transfer(new anchor.BN(100))
  //     .accounts({
  //       sender: authority.publicKey,
  //       recipient: recipient.publicKey,
  //       mintAccount: tokenMintKeypair.publicKey,
  //     })
  //     .signers([sig])
  //     .rpc({ skipPreflight: true });
  //   console.log('Your transaction signature', transactionSignature);
  //   console.log("✅ Transfer passed - Token transferred successfully!");
  // });

  // it('Harvest Transfer Fees to Mint Account', async () => {
  //   const recipientTokenAccountAddress = getAssociatedTokenAddressSync(tokenMintKeypair.publicKey, authority.publicKey, false, TOKEN_2022_PROGRAM_ID);
  //   const transactionSignature = await program.methods
  //     .harvest()
  //     .accounts({ mintAccount: tokenMintKeypair.publicKey })
  //     .remainingAccounts([
  //       {
  //         pubkey: recipientTokenAccountAddress,
  //         isSigner: false,
  //         isWritable: true,
  //       },
  //     ])
  //     .rpc({ skipPreflight: true });
  //   console.log('Your transaction signature', transactionSignature);
  //   console.log("✅ Harvest passed - Token harvested successfully!");
  // });

  // it('Withdraw Transfer Fees from Mint Account', async () => {
  //     const senderTokenAccountAddress = getAssociatedTokenAddressSync(tokenMintKeypair.publicKey, authority.publicKey, false, TOKEN_2022_PROGRAM_ID);

  //   const transactionSignature = await program.methods
  //     .withdraw()
  //     .accounts({
  //       authority: authority.publicKey,
  //       mintAccount: tokenMintKeypair.publicKey,
  //       tokenAccount: senderTokenAccountAddress,
  //     })
  //     .signers([sig])
  //     .rpc({ skipPreflight: true });
  //   console.log('Your transaction signature', transactionSignature);
  //   console.log("✅ Withdraw passed - Token tax withdraw successfully!");
  // });

  // it('Update Transfer Fee', async () => {
  //   const transferFeeBasisPoints = 0;
  //   const maximumFee = 0;

  //   const transactionSignature = await program.methods
  //     .updateFee(transferFeeBasisPoints, new anchor.BN(maximumFee))
  //     .accounts(
  //       { 
  //         authority: authority.publicKey,
  //         mintAccount: tokenMintKeypair.publicKey,
  //       }
  //     )
  //     .signers([sig])
  //     .rpc({ skipPreflight: true });
  //   console.log('Your transaction signature', transactionSignature);
  //   console.log("✅ Update passed - Token tax fee updated successfully!");
  // });

  // it('Update Program state', async () => {
  //   const newAuthority = Keypair.generate().publicKey;
  //   const rewardMint = Keypair.generate().publicKey;

  //   const transactionSignature = await program.methods
  //     .updateProgramState(newAuthority, rewardMint)
  //     .accounts(
  //       { 
  //         state: statePda,
  //         authority: authority.publicKey,
  //       }
  //     )
  //     .signers([sig])
  //     .rpc({ skipPreflight: true });
  //   console.log('Your transaction signature', transactionSignature);
  //   console.log("✅ Update passed - Program state updated successfully!");
  // });
});
