use anchor_client::{
    solana_client::{
        nonblocking::rpc_client::RpcClient,
        rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
        rpc_filter::{Memcmp, RpcFilterType},
    },
    solana_sdk::{
        commitment_config::CommitmentConfig,
        instruction::Instruction,
        program_pack::Pack,
        pubkey::Pubkey,
        signature::{Keypair, Signature},
        signer::Signer,
    },
    Client, Cluster, Program,
};
use anchor_lang::prelude::AccountMeta;
use dotenv::dotenv;
use futures::future::join_all;
use serde_json::Value;
use std::sync::Arc;
use std::{env, str::FromStr, time::Duration};
use tokio::time::interval;

// Constants
/// Metaplex Token Metadata Program ID for fetching token metadata
const METAPLEX_METADATA_PROGRAM_ID: &str = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";

// Structs
/// Represents token metadata fetched from the Metaplex Metadata Program
#[derive(Debug)]
struct TokenMetadata {
    _name: String,  // Full name of the token
    symbol: String, // Token ticker symbol
}

/// Fetches token metadata from the Metaplex Metadata Program
///
/// # Arguments
/// * `rpc_client` - The RPC client for interacting with the Solana network
/// * `mint` - The public key of the token mint to fetch metadata for
///
/// # Returns
/// A `Result` containing `TokenMetadata` with the token's name and symbol, or an error if fetching fails
///
/// # Errors
/// Returns an error if the account data cannot be fetched or parsed correctly
async fn fetch_token_metadata(
    rpc_client: &RpcClient,
    mint: &Pubkey,
) -> Result<TokenMetadata, anyhow::Error> {
    let metadata_program = Pubkey::from_str(METAPLEX_METADATA_PROGRAM_ID)?;

    // Derive the Program Derived Address (PDA) for the token's metadata
    let (metadata_pda, _) = Pubkey::find_program_address(
        &[b"metadata", metadata_program.as_ref(), mint.as_ref()],
        &metadata_program,
    );

    let account = rpc_client.get_account(&metadata_pda).await?;

    // Metadata layout: 1 byte (update auth flag) + 32 bytes (update auth) + 32 bytes (mint) + data
    let data = &account.data[65..]; // Offset where variable-length data starts

    // Extract name: 4 bytes (length) + string
    let name_length = u32::from_le_bytes(data[0..4].try_into()?) as usize;
    let _name = String::from_utf8(data[4..4 + name_length].to_vec())?
        .trim_end_matches(char::from(0))
        .to_string();

    // Extract symbol: 4 bytes (length) + string
    let symbol_offset = 4 + name_length;
    let symbol_length =
        u32::from_le_bytes(data[symbol_offset..symbol_offset + 4].try_into()?) as usize;
    let symbol =
        String::from_utf8(data[symbol_offset + 4..symbol_offset + 4 + symbol_length].to_vec())?
            .trim_end_matches(char::from(0))
            .to_string();

    Ok(TokenMetadata { _name, symbol })
}

/// Fetches a swap quote from Raydium pools using pool data and RPC client
///
/// # Arguments
/// * `rpc_client` - The Solana RPC client
/// * `pools` - Raydium pool data from the SDK
/// * `in_token` - Public key of the input token mint
/// * `out_token` - Public key of the output token mint
/// * `in_amount` - Amount of input tokens to swap
/// * `slippage` - Acceptable slippage percentage (e.g., 0.25 for 25%)
///
/// # Returns
/// A `Result` containing the estimated output amount in tokens, or an error if the calculation fails
async fn fetch_raydium_quote(
    rpc_client: &RpcClient,
    pools: &Value,
    in_token: &Pubkey,
    out_token: &Pubkey,
    in_amount: u64,
    slippage: f64,
) -> Result<u64, anyhow::Error> {
    let default_vec = Vec::<Value>::new();
    let pool = pools["official"]
        .as_array()
        .unwrap_or(&default_vec)
        .iter()
        .find(|pool| {
            let base_mint = pool["baseMint"].as_str();
            let quote_mint = pool["quoteMint"].as_str();
            (base_mint == Some(&in_token.to_string()) && quote_mint == Some(&out_token.to_string()))
                || (base_mint == Some(&out_token.to_string())
                    && quote_mint == Some(&in_token.to_string()))
        })
        .ok_or(anyhow::anyhow!("No suitable pool found for tokens"))?;

    // Simplified estimation using pool reserves (this assumes CLMM pool structure)
    let base_vault = Pubkey::from_str(pool["baseVault"].as_str().unwrap_or(""))?;
    let quote_vault = Pubkey::from_str(pool["quoteVault"].as_str().unwrap_or(""))?;

    let base_balance = rpc_client
        .get_token_account_balance(&base_vault)
        .await?
        .amount
        .parse::<u64>()?;
    let quote_balance = rpc_client
        .get_token_account_balance(&quote_vault)
        .await?
        .amount
        .parse::<u64>()?;

    // Determine direction and calculate quote
    let (reserve_in, reserve_out) = if pool["baseMint"].as_str() == Some(&in_token.to_string()) {
        (base_balance, quote_balance)
    } else {
        (quote_balance, base_balance)
    };

    // Simple constant product formula (x * y = k) without fees for estimation
    // Actual Raydium CLMM uses more complex logic with ticks, but this is a basic approximation
    let amount_out =
        (in_amount as u128 * reserve_out as u128 / (reserve_in as u128 + in_amount as u128)) as u64;

    // Apply slippage
    let amount_out_with_slippage = (amount_out as f64 * (1.0 - slippage)) as u64;

    Ok(amount_out_with_slippage)
}

/// Main entry point for the token tax and distribution cron bot
///
/// This function sets up the environment, initializes the Solana client, and runs a periodic job
/// to harvest taxes, swap tokens, and distribute rewards to token holders.
///
/// # Panics
/// Panics if required environment variables are missing or invalid
#[tokio::main]
async fn main() {
    dotenv().ok();

    let sol_admin_private_key =
        env::var("SOLANA_ADMIN_PRIVATE_KEY").expect("SOLANA_ADMIN_PRIVATE_KEY must be set");
    let mint_address = env::var("TOKEN_MINT").expect("TOKEN_MINT must be set");

    let cluster = env::var("SOLANA_NETWORK")
        .unwrap_or("mainnet".to_string())
        .to_lowercase();
    let (rpc_url, raydium_endpoint) = match cluster.as_str() {
        "devnet" => (
            "https://api.devnet.solana.com".to_string(),
            "https://api.raydium.io/v2/sdk/liquidity/devnet.json".to_string(),
        ),
        "mainnet" => (
            "https://api.mainnet-beta.solana.com".to_string(),
            "https://api.raydium.io/v2/sdk/liquidity/mainnet.json".to_string(),
        ),
        custom => (
            custom.to_string(),
            "https://api.raydium.io/v2/sdk/liquidity/mainnet.json".to_string(),
        ),
    };

    let raydium_data = fetch_raydium_pools(&raydium_endpoint)
        .await
        .expect("Failed to fetch Raydium pools");

    let interval_secs = env::var("INTERVAL")
        .unwrap_or("3600".to_string())
        .parse::<u64>()
        .expect("Failed to parse INTERVAL");
    let mut interval = interval(Duration::from_secs(interval_secs));

    println!("Raydium data: {}", raydium_data);
    loop {
        interval.tick().await;
        match process_job(
            &rpc_url,
            &sol_admin_private_key,
            &mint_address,
            &raydium_data,
        )
        .await
        {
            Ok(()) => println!("Job completed at {}", chrono::Utc::now()),
            Err(e) => eprintln!("Job failed at {}: {:?}", chrono::Utc::now(), e),
        }
    }
}

/// Fetches Raydium pool data from the specified endpoint
///
/// # Arguments
/// * `endpoint` - The URL of the Raydium liquidity endpoint
///
/// # Returns
/// A `Result` containing the JSON data of Raydium pools, or an error if the request fails
async fn fetch_raydium_pools(endpoint: &str) -> Result<Value, anyhow::Error> {
    let client = reqwest::Client::new();
    let response = client
        .get(endpoint)
        .timeout(Duration::from_secs(10))
        .send()
        .await?
        .json::<Value>()
        .await?;
    Ok(response)
}

/// Processes the main job: harvests taxes, swaps tokens, and distributes rewards
///
/// # Arguments
/// * `sol_rpc_endpoint` - The Solana RPC endpoint URL
/// * `sol_admin_private_key` - The admin's private key in base58 format
/// * `token_mint_address` - The address of the token mint
/// * `raydium_data` - Raydium pool data fetched from the API
///
/// # Returns
/// A `Result` indicating success or an error if any step fails
async fn process_job(
    sol_rpc_endpoint: &String,
    sol_admin_private_key: &String,
    token_mint_address: &String,
    raydium_data: &Value,
) -> Result<(), anyhow::Error> {
    let payer = Keypair::from_base58_string(sol_admin_private_key);
    let client = Client::new(
        Cluster::Custom(sol_rpc_endpoint.to_string(), "".to_string()),
        Arc::new(payer.insecure_clone()),
    );
    let token_mint = Pubkey::from_str(token_mint_address)?;

    let tax_program_id = Pubkey::from_str("YOUR_TAX_PROGRAM_ID_HERE")?;
    let raydium_clmm_id = Pubkey::from_str("CLMM9tUoggJu2wam25TCwC6eWkw1mnbn7nryKyswgTNB")?;
    let token_program_id = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")?;
    let token_2022_program_id = Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")?;
    let ata_program_id = Pubkey::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL")?;
    let memo_program_id = Pubkey::from_str("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr")?;

    let tax_program = client.program(tax_program_id)?;
    let clmm_program = client.program(raydium_clmm_id)?;

    let (admin_ata, _) = Pubkey::find_program_address(
        &[
            payer.pubkey().as_ref(),
            token_program_id.as_ref(),
            token_mint.as_ref(),
        ],
        &ata_program_id,
    );

    let rpc_client =
        RpcClient::new_with_commitment(sol_rpc_endpoint.to_string(), CommitmentConfig::confirmed());

    let token_account = rpc_client.get_account(&token_mint).await?;
    let is_token_2022 = token_account.owner == token_2022_program_id;

    harvest(&tax_program, &token_mint, &token_2022_program_id, &payer).await?;
    withdraw(
        &tax_program,
        &token_mint,
        &token_2022_program_id,
        &payer,
        &payer.pubkey(),
        &admin_ata,
    )
    .await?;

    let harvested_amount = rpc_client
        .get_token_account_balance(&admin_ata)
        .await?
        .ui_amount
        .unwrap_or(0.0) as u64;

    if harvested_amount == 0 {
        println!("No tokens harvested, skipping swap and distribution");
        return Ok(());
    }

    let default_vec = Vec::<Value>::new();
    let pools = raydium_data["official"]
        .as_array()
        .unwrap_or(&default_vec)
        .iter()
        .find(|pool| {
            pool["baseMint"].as_str() == Some(token_mint_address)
                || pool["quoteMint"].as_str() == Some(token_mint_address)
        })
        .ok_or(anyhow::anyhow!(
            "No suitable pool found for token: {}",
            token_mint_address
        ))?;

    let pool_id = Pubkey::from_str(pools["id"].as_str().unwrap_or(""))?;
    let base_vault = Pubkey::from_str(pools["baseVault"].as_str().unwrap_or(""))?;
    let quote_vault = Pubkey::from_str(pools["quoteVault"].as_str().unwrap_or(""))?;
    let tick_array_lower = Pubkey::from_str(pools["tickArrayLower"].as_str().unwrap_or(""))?;
    let base_mint = Pubkey::from_str(pools["baseMint"].as_str().unwrap_or(""))?;
    let quote_mint = Pubkey::from_str(pools["quoteMint"].as_str().unwrap_or(""))?;
    let amm_config = Pubkey::from_str(
        pools["configId"]
            .as_str()
            .unwrap_or("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1"),
    )?;

    let base_metadata = fetch_token_metadata(&rpc_client, &base_mint).await?;
    let quote_metadata = fetch_token_metadata(&rpc_client, &quote_mint).await?;

    let (input_token, output_token, _is_input_2022, _in_token_symbol, _out_token_symbol) =
        if pools["baseMint"].as_str() == Some(token_mint_address) {
            (
                token_mint,
                quote_mint,
                is_token_2022,
                base_metadata.symbol.clone(),
                quote_metadata.symbol.clone(),
            )
        } else {
            (
                base_mint,
                token_mint,
                false,
                base_metadata.symbol.clone(),
                quote_metadata.symbol.clone(),
            )
        };

    let (output_ata, _) = Pubkey::find_program_address(
        &[
            payer.pubkey().as_ref(),
            token_program_id.as_ref(),
            output_token.as_ref(),
        ],
        &ata_program_id,
    );

    let amount_in = harvested_amount;
    let slippage_tolerance = 0.25; // 25% slippage tolerance

    let quoted_amount_out = fetch_raydium_quote(
        &rpc_client,
        raydium_data,
        &input_token,
        &output_token,
        amount_in,
        slippage_tolerance,
    )
    .await?;

    let amount_out_minimum = (quoted_amount_out as f64 * (1.0 - slippage_tolerance)) as u64;

    swap_clmm(
        &clmm_program,
        &payer,
        pool_id,
        amm_config,
        admin_ata,
        output_ata,
        base_vault,
        quote_vault,
        tick_array_lower,
        base_mint,
        quote_mint,
        token_program_id,
        token_2022_program_id,
        memo_program_id,
        amount_in,
        amount_out_minimum,
    )
    .await?;

    let reward_balance = rpc_client
        .get_token_account_balance(&output_ata)
        .await?
        .ui_amount
        .unwrap_or(0.0) as u64;

    distribute_rewards(
        rpc_client, // Pass owned value
        client,     // Pass owned value
        &token_mint,
        &output_token,
        reward_balance,
        &payer,
        token_program_id,
        token_2022_program_id,
        ata_program_id,
    )
    .await?;

    Ok(())
}

/// Harvests accumulated taxes from the token program
///
/// # Arguments
/// * `program` - The tax program instance
/// * `mint_account` - The token mint address
/// * `token_2022_program_id` - The Token-2022 program ID
/// * `keypair` - The signing keypair
///
/// # Returns
/// A `Result` containing the transaction signature, or an error if the harvest fails
async fn harvest(
    program: &Program<Arc<Keypair>>,
    mint_account: &Pubkey,
    token_2022_program_id: &Pubkey,
    keypair: &Keypair,
) -> Result<Signature, anyhow::Error> {
    let tx_hash = program
        .request()
        .accounts(tax_token::accounts::Harvest {
            mint_account: *mint_account,
            token_program: *token_2022_program_id,
        })
        .signer(keypair)
        .send()?;
    Ok(tx_hash)
}

/// Withdraws harvested taxes to the admin's associated token account (ATA)
///
/// # Arguments
/// * `program` - The tax program instance
/// * `mint_account` - The token mint address
/// * `token_2022_program_id` - The Token-2022 program ID
/// * `keypair` - The signing keypair
/// * `authority` - The authority's public key
/// * `authority_ata` - The authority's associated token account
///
/// # Returns
/// A `Result` containing the transaction signature, or an error if the withdrawal fails
async fn withdraw(
    program: &Program<Arc<Keypair>>,
    mint_account: &Pubkey,
    token_2022_program_id: &Pubkey,
    keypair: &Keypair,
    authority: &Pubkey,
    authority_ata: &Pubkey,
) -> Result<Signature, anyhow::Error> {
    let tx_hash = program
        .request()
        .accounts(tax_token::accounts::Withdraw {
            authority: *authority,
            mint_account: *mint_account,
            token_account: *authority_ata,
            token_program: *token_2022_program_id,
        })
        .signer(keypair)
        .send()?;
    Ok(tx_hash)
}

/// Executes a swap on the Raydium Concentrated Liquidity Market Maker (CLMM)
///
/// # Arguments
/// * `program` - The Raydium CLMM program instance
/// * `payer` - The payer keypair
/// * `pool_id` - The pool identifier
/// * `amm_config` - The AMM configuration address
/// * `input_token_account` - The input token account
/// * `output_token_account` - The output token account
/// * `input_vault` - The input vault address
/// * `output_vault` - The output vault address
/// * `tick_array` - The tick array address
/// * `input_vault_mint` - The input vault mint address
/// * `output_vault_mint` - The output vault mint address
/// * `token_program_id` - The token program ID
/// * `token_2022_program_id` - The Token-2022 program ID
/// * `memo_program_id` - The memo program ID
/// * `amount_in` - The input amount
/// * `amount_out_minimum` - The minimum output amount
///
/// # Returns
/// A `Result` containing the transaction signature, or an error if the swap fails
async fn swap_clmm(
    program: &Program<Arc<Keypair>>,
    payer: &Keypair,
    pool_id: Pubkey,
    amm_config: Pubkey,
    input_token_account: Pubkey,
    output_token_account: Pubkey,
    input_vault: Pubkey,
    output_vault: Pubkey,
    tick_array: Pubkey,
    input_vault_mint: Pubkey,
    output_vault_mint: Pubkey,
    token_program_id: Pubkey,
    token_2022_program_id: Pubkey,
    memo_program_id: Pubkey,
    amount_in: u64,
    amount_out_minimum: u64,
) -> Result<Signature, anyhow::Error> {
    let (observation, _) =
        Pubkey::find_program_address(&[b"observation", pool_id.as_ref()], &program.id());

    let mut data = vec![9]; // Instruction discriminator for swap
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&amount_out_minimum.to_le_bytes());
    data.extend_from_slice(&0u128.to_le_bytes()); // sqrt_price_limit (set to 0 for no limit)
    data.push(1); // direction (1 for base-to-quote)

    let accounts = vec![
        AccountMeta::new(payer.pubkey(), true),
        AccountMeta::new_readonly(amm_config, false),
        AccountMeta::new(pool_id, false),
        AccountMeta::new(input_token_account, false),
        AccountMeta::new(output_token_account, false),
        AccountMeta::new(input_vault, false),
        AccountMeta::new(output_vault, false),
        AccountMeta::new(observation, false),
        AccountMeta::new_readonly(token_program_id, false),
        AccountMeta::new_readonly(token_2022_program_id, false),
        AccountMeta::new_readonly(memo_program_id, false),
        AccountMeta::new_readonly(input_vault_mint, false),
        AccountMeta::new_readonly(output_vault_mint, false),
        AccountMeta::new(tick_array, false),
    ];

    let instruction = Instruction {
        program_id: program.id(),
        accounts,
        data,
    };

    let tx_hash = program
        .request()
        .instruction(instruction)
        .signer(payer)
        .send()?;
    Ok(tx_hash)
}

/// Distributes rewards proportionally to token holders
///
/// # Arguments
/// * `rpc_client` - The RPC client for network queries
/// * `client` - The Anchor client instance
/// * `tax_token_mint` - The tax token mint address
/// * `reward_token_mint` - The reward token mint address
/// * `total_rewards` - The total rewards to distribute
/// * `payer` - The payer keypair
/// * `token_program_id` - The token program ID
/// * `token_2022_program_id` - The Token-2022 program ID
/// * `ata_program_id` - The associated token account program ID
///
/// # Returns
/// A `Result` indicating success or an error if distribution fails
async fn distribute_rewards(
    rpc_client: RpcClient,
    client: Client<Arc<Keypair>>,
    tax_token_mint: &Pubkey,
    reward_token_mint: &Pubkey,
    total_rewards: u64,
    payer: &Keypair,
    token_program_id: Pubkey,
    _token_2022_program_id: Pubkey,
    ata_program_id: Pubkey,
) -> Result<(), anyhow::Error> {
    // Get token supply
    let mint_info = rpc_client.get_token_supply(tax_token_mint).await?;
    let total_supply = mint_info.ui_amount.unwrap_or(0.0) as u64;

    // Create config for fetching token accounts
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![
            RpcFilterType::DataSize(spl_token::state::Account::LEN as u64),
            RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                0,
                tax_token_mint.to_string().as_bytes(),
            )),
        ]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(solana_account_decoder::UiAccountEncoding::Base64),
            ..RpcAccountInfoConfig::default()
        },
        with_context: Some(false),
    };

    // Get all token accounts
    let accounts = rpc_client
        .get_program_accounts_with_config(&token_program_id, config)
        .await?;

    // Process token accounts and create distribution data
    let mut distribution_data = Vec::new();
    for (_pubkey, account) in accounts {
        let token_account = spl_token::state::Account::unpack(&account.data)?;
        let balance = token_account.amount;

        if balance > 0 {
            let owner = token_account.owner;
            let reward = (balance as u128 * total_rewards as u128 / total_supply as u128) as u64;

            if reward > 0 {
                distribution_data.push((owner, reward));
            }
        }
    }

    // Get admin reward ATA
    let (admin_reward_ata, _) = Pubkey::find_program_address(
        &[
            payer.pubkey().as_ref(),
            token_program_id.as_ref(),
            reward_token_mint.as_ref(),
        ],
        &ata_program_id,
    );

    // Clone necessary data for concurrent tasks
    let client_arc = Arc::new(client); // Share the client across tasks
    let payer_arc = Arc::new(payer.insecure_clone());
    let rpc_client_arc = Arc::new(rpc_client);

    // Process distributions concurrently in batches
    const BATCH_SIZE: usize = 25;
    let mut tasks = Vec::new();

    for chunk in distribution_data.chunks(BATCH_SIZE) {
        let chunk_tasks: Vec<_> = chunk
            .iter()
            .map(|&(owner, reward)| {
                let client = Arc::clone(&client_arc);
                let payer = Arc::clone(&payer_arc);
                let rpc_client = Arc::clone(&rpc_client_arc);
                let admin_reward_ata = admin_reward_ata;
                let token_program_id = token_program_id;
                let ata_program_id = ata_program_id;
                let reward_token_mint = *reward_token_mint;

                tokio::spawn(async move {
                    // Derive the program inside the task
                    let program = client.program(token_program_id)?;

                    let (holder_ata, _) = Pubkey::find_program_address(
                        &[
                            owner.as_ref(),
                            token_program_id.as_ref(),
                            reward_token_mint.as_ref(),
                        ],
                        &ata_program_id,
                    );

                    // Create ATA if it doesn't exist
                    if rpc_client.get_account(&holder_ata).await.is_err() {
                        let ix = spl_associated_token_account::instruction::create_associated_token_account(
                            &payer.pubkey(),
                            &owner,
                            &reward_token_mint,
                            &token_program_id,
                        );
                        program
                            .request()
                            .instruction(ix)
                            .signer(&*payer)
                            .send()
                            ?;
                    }

                    // Transfer reward tokens
                    let ix = spl_token::instruction::transfer(
                        &token_program_id,
                        &admin_reward_ata,
                        &holder_ata,
                        &payer.pubkey(),
                        &[&payer.pubkey()],
                        reward,
                    )?;

                    program
                        .request()
                        .instruction(ix)
                        .signer(&*payer)
                        .send()
                        ?;

                    Ok::<(), anyhow::Error>(())
                })
            })
            .collect();

        tasks.extend(chunk_tasks);
    }

    // Await all tasks and collect results
    let results = join_all(tasks).await;
    for result in results {
        result??; // Propagate any JoinError or anyhow::Error
    }

    println!(
        "Distributed {} rewards to {} holders",
        total_rewards,
        distribution_data.len()
    );
    Ok(())
}
