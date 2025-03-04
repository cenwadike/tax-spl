# Tax Token System Documentation

This document provides detailed instructions on how to deploy and run the `tax_token` system, 
which consists of an on-chain Solana Anchor program and an off-chain Rust script. The on-chain 
program implements a Token-2022 with a transfer fee extension (10% tax), while the off-chain 
script (`cron-bot`) periodically harvests the tax, swaps it using Raydium CLMM pools, and 
distributes rewards to token holders every hour.

## Design

```
graph TD
    A[User Transfers Token-2022] -->|10% Tax| B(Token Mint)
    B -->|Harvest Tax| C(Cron Bot)
    C -->|Swap on Raydium CLMM| D(Reward Token)
    D -->|Distribute Rewards| E(Token Holders)
    subgraph On-Chain
        A
        B
    end
    subgraph Off-Chain
        C
    end
```

- **On-Chain (tax_token)**: A Solana Anchor program that creates a Token-2022 with a 10% transfer fee. The tax is collected in the mint account and can be harvested/withdrawn by the authority.

- **Off-Chain (cron-bot)**: A Rust script running in a Docker container that:
Harvests the tax from the mint account.
Withdraws it to the admin’s ATA.
Swaps the harvested tokens for a reward token (e.g., USDC) via Raydium CLMM.
Distributes the reward tokens proportionally to token holders every hour.

## Prerequisites

### Software Requirements
- **Rust**: Version 1.82.0 or later (rustup install 1.82.0)
- **Node.js**: Version 16+ (for TypeScript script)
- **Yarn**: Package manager (npm install -g yarn)
- **Docker**: For running the off-chain script
- **Anchor CLI**: Version 0.30.1 (cargo install --git https://github.com/coral-xyz/anchor avm --locked --force)
- **Solana CLI**: Version 1.18+ (sh -c "$(curl -sSfL https://release.solana.com/stable/install)")

### Environment Setup
- A Solana keypair at ~/.config/solana/id.json (or specify via PAYER_SECRET_KEY in .env).
- Access to a Solana RPC endpoint (e.g., https://api.devnet.solana.com).

## On-Chain Deployment and Initialization

- Step 1: Build the Anchor Program
1. Navigate to Project Root:
```sh
    cd /path/to/tax-token
```

2. Build the Program:

```sh
    anchor build
```

This generates target/deploy/tax_token.so and target/idl/tax_token.json.

- Step 2: Deploy and initialize the Program on Devnet

1. Ensure SOL Balance:
    - Check your wallet balance:
    ```sh
        solana balance --url https://api.devnet.solana.com
    ```

    - If less than 3 SOL, request an airdrop:
    ```sh
        solana airdrop 3 --url https://api.devnet.solana.com
    ```
2. Deploy and initialize the Program:
    ```sh
        anchor test
    ```
    - If you see an error with a seed phrase (e.g., solve syrup fatigue...), recover the buffer keypair:
    ```sh
        solana-keygen recover -o buffer-keypair.json
        # Enter the seed phrase from the error
    ```
    
    - Resume deployment:
    ```sh
        solana program deploy --url https://api.devnet.solana.com \
            --keypair ~/.config/solana/id.json \
            --buffer buffer-keypair.json \
            target/deploy/tax_token.so
    ```

## Off-Chain Cron Bot Setup

- Step 1: Configure Environment Variables

1. Update .env for Cron Bot:

    - Add the following to your .env file:

    ```sh
        SOLANA_ADMIN_PRIVATE_KEY=<BASE58_ENCODED_PRIVATE_KEY>
        TOKEN_MINT=<QUOTE_MINT>
        REWARD_TOKEN_MINT=<REWARD_MINT>
        TAX_PROGRAM_ID_HERE=<YOUR_PROGRAM_ID>
        SOLANA_NETWORK=devnet
        INTERVAL=3600  # Run every hour (in seconds)
    ```

    - How to Get Values:
        - SOLANA_ADMIN_PRIVATE_KEY: Run solana-keygen pubkey ~/.config/solana/id.json to get the public key, then use a tool like solana-keygen grind to convert to base58 if needed (or use the keypair file directly in base58 format).
        - TOKEN_MINT: From the create-clmm-pool.ts output (<QUOTE_MINT>).
        - REWARD_TOKEN_MINT: From the initialization logs or a known reward token (e.g., Devnet USDC: Gh9ZwEmdLJ8DscKNTkTqPbNwLNNBjuSzaG1NfTamcPump).
        - TAX_PROGRAM_ID_HERE: Your deployed program ID.

- Step 2: Build and Run the Cron Bot in Docker

1. Navigate to Project Root:

    ```sh
        cd /path/to/tax-token
    ```

2. Build the Docker Image:

    ```sh
        docker build -t cron-bot .
    ```

3. Run the Docker Container:
    ```sh
        docker run cron-bot
    ```
