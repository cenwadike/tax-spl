import { readFileSync, writeFileSync } from "fs";
import path from 'path';
import os from 'os';
import {  Keypair } from '@solana/web3.js';
import bs58 from 'bs58';

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

const secretKeyFilePath = process.env.PAYER_SECRET_KEY || '~/.config/solana/ephemeral.json';
const payerSecretKey = loadSecretKey(secretKeyFilePath);
const keypair = Keypair.fromSecretKey(payerSecretKey);
console.log("Secret key: ", bs58.encode(keypair.secretKey));

// Replace 'obviously-not-my-private-key' with your actual Base58-encoded private key
const base58PrivateKey = 'obviously-not-my-private-key';

// Step 1: Decode Base58 private key to bytes
const decodedBytes = bs58.decode(base58PrivateKey);

// Step 2: Create a Keypair from the decoded bytes (assumes 64-byte key)
const keypair2 = Keypair.fromSecretKey(decodedBytes);

// Step 3: Get the full keypair (64-byte private + 32-byte public) as Uint8Array
const fullKeypair = keypair2.secretKey; // 64 bytes (includes public key derivation)

// Step 4: Convert Uint8Array to a plain array for JSON serialization
const keyArray = Array.from(fullKeypair);

// Step 5: Write to file as a properly formatted JSON array
writeFileSync('mykey.json', JSON.stringify(keyArray));

// Optional: Verify the public key
console.log('Public Key:', keypair2.publicKey.toBase58());
