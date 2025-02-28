import * as dotenv from 'dotenv';
import BN from 'bn.js';
import { Decimal } from 'decimal.js';
import {
  Clmm,
  ClmmConfigInfo,
  Token,
  TokenAccount,
  fetchMultipleMintInfos,
  SplAccount,
} from '@raydium-io/raydium-sdk';
import {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  sendAndConfirmTransaction,
  LAMPORTS_PER_SOL,
} from '@solana/web3.js';
import {
  TOKEN_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  getAccount,
} from '@solana/spl-token';
import { readFileSync } from 'fs';
import path from 'path';
import os from 'os';

dotenv.config();

const NETWORK = process.env.NETWORK || 'devnet';
const ENDPOINTS = {
  devnet: 'https://api.devnet.solana.com',
  mainnet: 'https://api.mainnet-beta.solana.com',
};
const connection = new Connection(ENDPOINTS[NETWORK], 'confirmed');

const PROGRAMIDS = {
  CLMM: new PublicKey('devi51mZmdwUJGU9hjN27vEz64Gps7uUefqxg27EAtH'), // Devnet CLMM program ID
};
const makeTxVersion = 0; // LEGACY

const secretKeyFilePath = process.env.PAYER_SECRET_KEY || '~/.config/solana/id.json';
const payerSecretKey = loadSecretKey(secretKeyFilePath);
const wallet = Keypair.fromSecretKey(payerSecretKey);

const DECIMALS = 9;   // Matching test script decimals

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

async function buildAndSendTx(innerTransactions: { instructions: any[] }[]) {
  const tx = new Transaction();
  innerTransactions.forEach((innerTx) => {
    tx.add(...innerTx.instructions);
  });
  const txid = await sendAndConfirmTransaction(connection, tx, [wallet], {
    commitment: 'confirmed',
  });
  return [txid];
}

async function getWalletTokenAccount(connection: Connection, wallet: PublicKey): Promise<TokenAccount[]> {
  const [accounts, accounts2022] = await Promise.all([
    connection.getTokenAccountsByOwner(wallet, { programId: TOKEN_PROGRAM_ID }),
    connection.getTokenAccountsByOwner(wallet, { programId: TOKEN_2022_PROGRAM_ID }),
  ]);

  const tokenAccounts: TokenAccount[] = await Promise.all([
    ...accounts.value.map(async ({ pubkey }) => {
      const account = await getAccount(connection, pubkey, 'confirmed', TOKEN_PROGRAM_ID);
      const accountInfo: SplAccount = {
        mint: account.mint,
        owner: account.owner,
        amount: new BN(account.amount.toString()),
        delegate: account.delegate || null,
        state: account.isInitialized ? 1 : account.isFrozen ? 2 : 0,
        isNative: account.isNative ? new BN(1) : new BN(0),
        delegatedAmount: new BN(account.delegatedAmount.toString()),
        closeAuthority: account.closeAuthority || null,
        delegateOption: 0,
        isNativeOption: 0,
        closeAuthorityOption: 0,
      };
      return {
        pubkey,
        programId: TOKEN_PROGRAM_ID,
        accountInfo,
      };
    }),
    ...accounts2022.value.map(async ({ pubkey }) => {
      const account = await getAccount(connection, pubkey, 'confirmed', TOKEN_2022_PROGRAM_ID);
      const accountInfo: SplAccount = {
        mint: account.mint,
        owner: account.owner,
        amount: new BN(account.amount.toString()),
        delegate: account.delegate || null,
        state: account.isInitialized ? 1 : account.isFrozen ? 2 : 0,
        isNative: account.isNative ? new BN(1) : new BN(0),
        delegatedAmount: new BN(account.delegatedAmount.toString()),
        closeAuthority: account.closeAuthority || null,
        delegateOption: 0,
        isNativeOption: 0,
        closeAuthorityOption: 0,
      };
      return {
        pubkey,
        programId: TOKEN_2022_PROGRAM_ID,
        accountInfo,
      };
    }),
  ]);

  return tokenAccounts;
}

async function createRegularToken(): Promise<{ mint: PublicKey; ata: PublicKey }> {
  console.log("Start creating regular token");
  const mint = new PublicKey(process.env.REWARD_MINT || "7hmxme69fHBXXKmPKAayPwtFkwbieaeS8i4i16bYQzKM");
  const ata = new PublicKey(process.env.REWARD_ATA || "82aJsd3btRu7dh2Zdzod8CQ1cnGpEF6zTxczv6SFX8Kx");
  console.log(`Complete creating regular token successfully. Mint: ${mint}, ATA: ${ata}`);
  return { mint, ata };
}

async function deployAndInitializeTaxToken(rewardMint: PublicKey): Promise<{ mint: PublicKey; ata: PublicKey; programId: PublicKey }> {
    console.log("Deploying and initializing Anchor tax token...");
  
    const mint = new PublicKey(process.env.TAX_TOKEN_MINT || "Aq5FknGHoXikxVpjPVAPbm6AcwtaXQAN1YRhkfng7yB7");
    const ata = new PublicKey(process.env.TAX_TOKEN_ATA || "BR5QsotaQ7R12AaDSTWHKVjXa5Wt79BitKLVNKzJyRoN");
    const programId = new PublicKey(process.env.TAX_PROGRAM_ID || "6wgDw4z2yv7eqJnuvZFgyGE3m4pVGnd77pGsjPdc6z8B");

    console.log(`Anchor tax token deployed successfully. Mint: ${mint}, ATA: ${ata}`);
    return { mint, ata, programId };
}

async function clmmCreatePool(input: {
  baseToken: Token;
  quoteToken: Token;
  clmmConfigId: string;
  wallet: Keypair;
  startPoolPrice: Decimal;
  startTime: BN;
}) {
  const ammConfig: ClmmConfigInfo = {
    id: new PublicKey(input.clmmConfigId),
    index: 0,
    protocolFeeRate: 120000,
    tradeFeeRate: 2500,
    tickSpacing: 60,
    fundFeeRate: 0,
    description: '',
    fundOwner: wallet.publicKey.toBase58(),
  };

  const makeCreatePoolInstruction = await Clmm.makeCreatePoolInstructionSimple({
    connection,
    programId: PROGRAMIDS.CLMM,
    owner: input.wallet.publicKey,
    mint1: input.baseToken,
    mint2: input.quoteToken,
    ammConfig,
    initialPrice: input.startPoolPrice,
    startTime: input.startTime,
    makeTxVersion,
    payer: wallet.publicKey,
  });

  const mockPoolInfo = Clmm.makeMockPoolInfo({
    programId: PROGRAMIDS.CLMM,
    mint1: input.baseToken,
    mint2: input.quoteToken,
    ammConfig,
    createPoolInstructionSimpleAddress: makeCreatePoolInstruction.address,
    owner: input.wallet.publicKey,
    initialPrice: input.startPoolPrice,
    startTime: input.startTime,
  });

  return {
    txids: await buildAndSendTx(makeCreatePoolInstruction.innerTransactions),
    poolId: mockPoolInfo.id.toBase58(),
  };
}

async function clmmCreatePosition(input: {
  poolId: string;
  inputTokenAmount: Decimal;
  inputTokenMint: 'mintA' | 'mintB';
  wallet: Keypair;
  startPrice: Decimal;
  endPrice: Decimal;
  slippage: number;
}) {
  const walletTokenAccounts = await getWalletTokenAccount(connection, input.wallet.publicKey);
  
  const initialPoolInfo = (await Clmm.fetchMultiplePoolInfos({
    connection,
    poolKeys: [{
      id: input.poolId,
      mintProgramIdA: '',
      mintProgramIdB: '',
      mintA: '',
      mintB: '',
      vaultA: '',
      vaultB: '',
      mintDecimalsA: 0,
      mintDecimalsB: 0,
      ammConfig: undefined,
      rewardInfos: [],
      tvl: 0,
      day: undefined,
      week: undefined,
      month: undefined,
      lookupTableAccount: ''
    }],
    chainTime: new Date().getTime() / 1000,
    ownerInfo: {
      wallet: input.wallet.publicKey,
      tokenAccounts: walletTokenAccounts,
    },
  }))[input.poolId].state;

  const poolInfo = (await Clmm.fetchMultiplePoolInfos({
    connection,
    poolKeys: [{
      id: input.poolId,
      mintProgramIdA: TOKEN_PROGRAM_ID.toBase58(),
      mintProgramIdB: TOKEN_2022_PROGRAM_ID.toBase58(),
      mintA: initialPoolInfo.mintA.mint.toBase58(),
      mintB: initialPoolInfo.mintB.mint.toBase58(),
      vaultA: "",
      vaultB: "",
      mintDecimalsA: DECIMALS,
      mintDecimalsB: DECIMALS,
      ammConfig: {
        id: '5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1',
        index: 0,
        protocolFeeRate: 120000,
        tradeFeeRate: 2500,
        tickSpacing: 60,
        fundFeeRate: 0,
        description: '',
        fundOwner: wallet.publicKey.toBase58(),
      },
      rewardInfos: [],
      tvl: 0,
      day: undefined,
      week: undefined,
      month: undefined,
      lookupTableAccount: '',
    }],
    chainTime: new Date().getTime() / 1000,
    ownerInfo: {
      wallet: input.wallet.publicKey,
      tokenAccounts: walletTokenAccounts,
    },
  }))[input.poolId].state;

  const { tick: tickLower } = Clmm.getPriceAndTick({
    poolInfo,
    baseIn: true,
    price: input.startPrice,
  });
  const { tick: tickUpper } = Clmm.getPriceAndTick({
    poolInfo,
    baseIn: true,
    price: input.endPrice,
  });

  const decimals = input.inputTokenMint === 'mintA' ? poolInfo.mintA.decimals : poolInfo.mintB.decimals;
  const { liquidity, amountSlippageA, amountSlippageB } = Clmm.getLiquidityAmountOutFromAmountIn({
    poolInfo,
    slippage: input.slippage,
    inputA: input.inputTokenMint === 'mintA',
    tickUpper,
    tickLower,
    amount: new BN(input.inputTokenAmount.mul(10 ** decimals).toFixed(0)),
    add: true,
    amountHasFee: true,
    token2022Infos: await fetchMultipleMintInfos({
      connection,
      mints: [poolInfo.mintA.mint, poolInfo.mintB.mint],
    }),
    epochInfo: await connection.getEpochInfo(),
  });

  const makeOpenPositionInstruction = await Clmm.makeOpenPositionFromLiquidityInstructionSimple({
    connection,
    poolInfo,
    ownerInfo: {
      feePayer: input.wallet.publicKey,
      wallet: input.wallet.publicKey,
      tokenAccounts: walletTokenAccounts,
    },
    tickLower,
    tickUpper,
    liquidity,
    makeTxVersion,
    amountMaxA: amountSlippageA.amount,
    amountMaxB: amountSlippageB.amount,
  });

  return { txids: await buildAndSendTx(makeOpenPositionInstruction.innerTransactions) };
}

async function main() {
  const balance = await connection.getBalance(wallet.publicKey);
  if (balance < 3 * LAMPORTS_PER_SOL) {
    await connection.requestAirdrop(wallet.publicKey, 3 * LAMPORTS_PER_SOL);
    await new Promise(resolve => setTimeout(resolve, 1000)); // Wait for confirmation
  }

  const { mint: baseMint } = await createRegularToken();
  const { mint: quoteMint } = await deployAndInitializeTaxToken(baseMint);
  const baseToken = new Token(TOKEN_PROGRAM_ID, baseMint, DECIMALS);
  const quoteToken = new Token(TOKEN_2022_PROGRAM_ID, quoteMint, DECIMALS);

  const { txids: createTxids, poolId } = await clmmCreatePool({
    baseToken,
    quoteToken,
    clmmConfigId: '5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1',
    wallet,
    startPoolPrice: new Decimal(1),
    startTime: new BN(Math.floor(new Date().getTime() / 1000)),
  });
  console.log('Pool created with txids:', createTxids);
  console.log('Pool ID:', poolId);

  const { txids: positionTxids } = await clmmCreatePosition({
    poolId: poolId,
    inputTokenAmount: new Decimal(100),
    inputTokenMint: 'mintA',
    wallet,
    startPrice: new Decimal(0.9),
    endPrice: new Decimal(1.1),
    slippage: 0.01,
  });
  console.log('Position created and liquidity added with txids:', positionTxids);

  console.log('Pool Summary:');
  console.log('Base Token (Regular):', baseMint.toBase58());
  console.log('Quote Token (Anchor Tax Token with 6% tax):', quoteMint.toBase58());
}

main()
  .then(() => console.log('Script completed successfully'))
  .catch((error) => console.error('Error:', error));