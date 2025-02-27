import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { TaxToken } from "../target/types/tax_token";
import { PublicKey, Keypair, SystemProgram, Signer, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { 
  TOKEN_2022_PROGRAM_ID, 
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddress,
  createMint,
  TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  getMint,
  getOrCreateAssociatedTokenAccount,
  mintTo
} from "@solana/spl-token";
import { assert } from "chai";
import { ASSOCIATED_PROGRAM_ID } from "@coral-xyz/anchor/dist/cjs/utils/token";

describe("tax-token", () => {
  // Configure the client to use the local cluster
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.TaxToken as Program<TaxToken>;
  const tokenMetadataProgramId = new PublicKey("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");
  
  // Generate a new keypair for the reward mint
  const tokenMintKeypair = Keypair.generate();
  const rewardMintKeypair = Keypair.generate();
  
  // We'll need some test wallets
  // const authority = provider.wallet;
  const authority = Keypair.generate();
  
  // Test data
  const tokenName = "Tax Token";
  const tokenSymbol = "TAX";
  const tokenUri = "https://example.com/metadata.json";
  const tokenDecimals = 9;
  const tokenTotalSupply = 1_000_000_000 * 10 ** tokenDecimals; // 1 billion tokens

  it("Initializes the tax token", async () => {
    console.log("Starting tax token initialization test...");

    const admin = Keypair.generate();
    console.log("Admin: ", admin.publicKey.toString());
    const sig: Signer = {
      publicKey: authority.publicKey,
      secretKey: authority.secretKey
    }
    const mintSig: Signer = {
      publicKey: tokenMintKeypair.publicKey,
      secretKey: tokenMintKeypair.secretKey
    }

    await program.provider.connection.confirmTransaction(
      await program.provider.connection.requestAirdrop(
        admin.publicKey,
        3 * LAMPORTS_PER_SOL
      ),
      "confirmed"
    );

    await program.provider.connection.confirmTransaction(
      await program.provider.connection.requestAirdrop(
        authority.publicKey,
        3 * LAMPORTS_PER_SOL
      ),
      "confirmed"
    );

    await program.provider.connection.confirmTransaction(
      await program.provider.connection.requestAirdrop(
        program.provider.publicKey,
        3 * LAMPORTS_PER_SOL
      ),
      "confirmed"
    );

    // Find PDA addresses
    const [statePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("program_state")],
      program.programId
    );
    
    const tokenMint =  tokenMintKeypair.publicKey;   
    
    console.log("Creating reward mint...");
    // Create the reward mint first (this would typically be an existing token in a real scenario)
    const rewardMint = await createMint(
      provider.connection,
      admin,
      admin.publicKey,
      null,
      tokenDecimals,
      rewardMintKeypair,
      undefined,
      TOKEN_PROGRAM_ID
    );
    
    const treasuryAccount = getAssociatedTokenAddressSync(
      tokenMint,
      statePda,
      true,
      TOKEN_2022_PROGRAM_ID
    );
    
    const rewardTreasuryAccount = await getAssociatedTokenAddress(
      rewardMint,
      statePda,
      true,
      TOKEN_PROGRAM_ID
    );
    
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
          },
          treasuryAccount,
          rewardTreasuryAccount
      )
        .accounts(initCtx)
        .signers([mintSig, sig])
        .rpc({ commitment: "confirmed" });
      
      console.log("Transaction signature:", tx);
      
      // Fetch the program state to verify initialization
      const state = await program.account.programState.fetch(statePda);
      
      // Assert the state was properly initialized
      assert.equal(state.authority.toString(), authority.publicKey.toString());
      assert.equal(state.tokenMint.toString(), tokenMint.toString());
      assert.equal(state.rewardMint.toString(), rewardMint.toString());
      assert.equal(state.treasury.toString(), treasuryAccount.toString());
      assert.equal(state.rewardTreasury.toString(), rewardTreasuryAccount.toString());
      
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
      console.log("✅ Mint passed - Token minted successfully!");
      console.log("Transaction Signature: ", mintRes)
    } catch (error) {
      console.error("Error minting tokens");
      throw error
    }  
  });
});
