use anchor_client::solana_client;
use anchor_client::solana_client::rpc_config::RpcSendTransactionConfig;
use anchor_client::solana_client::rpc_request::RpcResponseErrorData;
use anchor_client::{
    solana_client::nonblocking::rpc_client::RpcClient,
    solana_sdk::{
        commitment_config::{CommitmentConfig, CommitmentLevel},
        instruction::Instruction,
        pubkey::Pubkey,
        signature::{Keypair, Signature},
        signer::Signer,
    },
    Client, Cluster, Program,
};
use anchor_lang::prelude::AccountMeta;
use anyhow::anyhow;
use borsh::{BorshDeserialize, BorshSerialize};
use dotenv::dotenv;
use log::{debug, error, info, warn};
use solana_sdk::message::Message;
use solana_sdk::transaction::Transaction;
use std::sync::Arc;
use std::{env, str::FromStr, thread, time::Duration};
use tokio;

mod utils;
use utils::{get_discriminant, get_token_accounts, setup_logging};

/// Main entry point for the token tax and distribution cron bot
#[tokio::main]
async fn main() {
    dotenv().ok();
    setup_logging();
    info!("🚀 Starting Token Tax and Distribution Bot...");

    // Load environment variables with error handling
    let helius_rpc_endpoint =
        env::var("HELIUS_RPC").expect("HELIUS_RPC must be set in environment variables");
    let sol_admin_private_key =
        env::var("SOLANA_ADMIN_PRIVATE_KEY").expect("SOLANA_ADMIN_PRIVATE_KEY must be set");
    let tax_program_id =
        env::var("TAX_PROGRAM_ID").expect("TAX_PROGRAM_ID must be set in environment variables");
    let mint_address = env::var("TOKEN_MINT").expect("TOKEN_MINT must be set");
    let reward_token_mint_address = env::var("REWARD_TOKEN_MINT")
        .expect("REWARD_TOKEN_MINT must be set in environment variables");
    let pool_id = env::var("POOL_ID").expect("POOL_ID must be set");
    let base_vault = env::var("BASE_VAULT").expect("BASE_VAULT must be set");
    let quote_vault = env::var("QUOTE_VAULT").expect("QUOTE_VAULT must be set");
    let observation_state = env::var("OBSERVATION_STATE").expect("OBSERVATION_STATE must be set");
    let amm_config = env::var("AMM_CONFIG").expect("AMM_CONFIG must be set");

    let cluster = env::var("SOLANA_NETWORK")
        .unwrap_or("mainnet".to_string())
        .to_lowercase();
    let rpc_url = match cluster.as_str() {
        "devnet" => "https://api.devnet.solana.com".to_string(),
        "mainnet" => helius_rpc_endpoint,
        custom => custom.to_string(),
    };
    info!("🌐 Connected to cluster: {} (RPC: {})", cluster, rpc_url);

    let interval_secs = env::var("INTERVAL")
        .unwrap_or("3600".to_string())
        .parse::<u64>()
        .expect("Failed to parse INTERVAL");
    info!("⏰ Job interval set to {} seconds", interval_secs);

    loop {
        info!("🏃 Starting new job cycle...");
        match process_job(
            &rpc_url,
            &sol_admin_private_key,
            &tax_program_id,
            &mint_address,
            &reward_token_mint_address,
            &base_vault,
            &quote_vault,
            &observation_state,
            &pool_id,
            &amm_config,
        )
        .await
        {
            Ok(()) => info!("✅ Job completed successfully at {}", chrono::Utc::now()),
            Err(e) => error!("❌ Job failed at {}: {:?}", chrono::Utc::now(), e),
        }
        debug!("⏳ Sleeping for {} seconds...", interval_secs);
        thread::sleep(Duration::from_secs(interval_secs));
    }
}

/// Processes the main job: harvests taxes, swaps tokens, and distributes rewards
///
/// # Arguments
/// * `sol_rpc_endpoint` - Solana RPC endpoint URL
/// * `sol_admin_private_key` - Admin's private key in base58
/// * `tax_program_id` - ID of the tax program
/// * `token_mint_address` - Mint address of the taxed token
/// * `reward_token_mint_address` - Mint address of the reward token
/// * `base_vault` - Base vault pubkey
/// * `quote_vault` - Quote vault pubkey
/// * `pool_id` - Pool identifier
/// * `amm_config` - AMM configuration pubkey
async fn process_job(
    sol_rpc_endpoint: &str,
    sol_admin_private_key: &str,
    tax_program_id: &str,
    token_mint_address: &str,
    reward_token_mint_address: &str,
    base_vault: &str,
    quote_vault: &str,
    observation_state: &str,
    pool_id: &str,
    amm_config: &str,
) -> Result<(), anyhow::Error> {
    info!("🔧 Initializing job processor...");
    let payer = Keypair::from_base58_string(sol_admin_private_key);
    let client = Client::new(
        Cluster::Custom(sol_rpc_endpoint.to_string(), "".to_string()),
        Arc::new(payer.insecure_clone()),
    );
    let token_mint = Pubkey::from_str(token_mint_address)?;
    let reward_token_mint = Pubkey::from_str(reward_token_mint_address)?;
    let tax_program_id = Pubkey::from_str(tax_program_id)?;

    // Define program IDs
    let raydium_clmm_id = Pubkey::from_str("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK")?;
    let token_program_id = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")?;
    let token_2022_program_id = Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")?;
    let ata_program_id = Pubkey::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL")?;
    let system_program_id = Pubkey::from_str("11111111111111111111111111111111")?;

    let tax_program = client.program(tax_program_id)?;
    let clmm_program = client.program(raydium_clmm_id)?;

    let base_vault = Pubkey::from_str(base_vault)?;
    let quote_vault = Pubkey::from_str(quote_vault)?;
    let observation_state = Pubkey::from_str(observation_state)?;
    let pool_id = Pubkey::from_str(pool_id)?;
    let amm_config = Pubkey::from_str(amm_config)?;

    let (admin_ata, _) = Pubkey::find_program_address(
        &[
            payer.pubkey().as_ref(),
            token_2022_program_id.as_ref(),
            token_mint.as_ref(),
        ],
        &ata_program_id,
    );

    let rpc_client =
        RpcClient::new_with_commitment(sol_rpc_endpoint.to_string(), CommitmentConfig::confirmed());

    let pre_harvested_balance = rpc_client
        .get_token_account_balance(&admin_ata)
        .await?
        .ui_amount
        .expect("Failed to parse balance") as u64;
    info!("💰 Pre-harvest balance: {}", pre_harvested_balance);

    debug!("📋 Fetching token holders...");
    let holders = get_token_accounts(&token_mint, None, 1, 1000, None, None, None, false).await;
    if holders.is_err() {
        error!("❌ Failed to fetch holders");
        return Err(anyhow!("Failed to get holders for harvesting"));
    }
    let holders = holders.unwrap();
    let token_accounts: Vec<Pubkey> = holders
        .into_iter()
        .map(|(account, _)| Pubkey::from_str(&account))
        .collect::<Result<Vec<_>, _>>()?;

    info!(
        "🌾 Harvesting taxes from {} accounts...",
        token_accounts.len()
    );
    for chunk in token_accounts.chunks(20) {
        harvest(
            &tax_program,
            &token_mint,
            chunk.to_vec(),
            &token_2022_program_id,
            &payer,
        )
        .await?;
    }

    info!("💸 Withdrawing harvested taxes...");
    withdraw(
        &tax_program,
        &token_mint,
        &token_2022_program_id,
        &payer,
        &payer.pubkey(),
        &admin_ata,
    )
    .await?;

    let post_harvested_balance = rpc_client
        .get_token_account_balance(&admin_ata)
        .await?
        .ui_amount
        .expect("Failed to parse balance") as u64;
    info!("💰 Post-harvest balance: {}", post_harvested_balance);

    let harvested_amount = post_harvested_balance - pre_harvested_balance;
    info!("📈 Harvested amount: {}", harvested_amount);

    if harvested_amount == 0 {
        warn!("⚠️ No tokens harvested, skipping swap and distribution");
        return Ok(());
    }

    // Derive reward token ATA
    let (output_ata, _) = Pubkey::find_program_address(
        &[
            payer.pubkey().as_ref(),
            token_program_id.as_ref(),
            reward_token_mint.as_ref(),
        ],
        &ata_program_id,
    );

    // Check and create destination ATA if needed
    debug!("🔍 Checking reward token ATA...");
    let output_account = rpc_client.get_account(&output_ata).await;
    if output_account.is_err() {
        info!("🆕 Creating reward token ATA...");
        let recent_blockhash = rpc_client.get_latest_blockhash().await?;
        let create_destination_ata_ix = Instruction {
            program_id: ata_program_id,
            accounts: vec![
                AccountMeta::new(payer.pubkey(), true),
                AccountMeta::new(output_ata, false),
                AccountMeta::new_readonly(payer.pubkey(), false),
                AccountMeta::new_readonly(reward_token_mint, false),
                AccountMeta::new_readonly(system_program_id, false),
                AccountMeta::new_readonly(token_program_id, false),
            ],
            data: vec![0],
        };
        let mut transaction =
            Transaction::new_with_payer(&[create_destination_ata_ix], Some(&payer.pubkey()));
        transaction.sign(&[&payer], recent_blockhash);
        match rpc_client.send_and_confirm_transaction(&transaction).await {
            Ok(signature) => info!("✅ Created ATA with tx: {}", signature),
            Err(err) => error!("❌ Failed to create ATA: {}", err),
        }
    }

    let amount_in = harvested_amount;
    info!("🔄 Swapping {} tokens...", amount_in);
    swap_clmm(
        &rpc_client,
        &clmm_program.id(),
        &payer,
        pool_id,
        amm_config,
        admin_ata,
        output_ata,
        base_vault,
        quote_vault,
        observation_state,
        token_mint,
        reward_token_mint,
        token_2022_program_id,
        token_program_id,
        amount_in,
    )
    .await?;

    let reward_balance = rpc_client
        .get_token_account_balance(&output_ata)
        .await?
        .ui_amount
        .unwrap_or(0.0) as u64;
    info!("🎁 Reward balance after swap: {}", reward_balance);

    info!("📤 Distributing rewards to holders...");
    distribute_rewards(
        rpc_client,
        client,
        &token_mint,
        &reward_token_mint,
        reward_balance,
        &payer,
        token_program_id,
        token_2022_program_id,
        ata_program_id,
    )
    .await?;

    info!("🏁 Job processing completed successfully");
    Ok(())
}

/// Harvests taxes from specified token accounts
///
/// # Arguments
/// * `program` - Tax program instance
/// * `mint_account` - Token mint address
/// * `token_accounts` - List of token accounts to harvest from
/// * `token_2022_program_id` - Token 2022 program ID
/// * `keypair` - Signer's keypair
async fn harvest(
    program: &Program<Arc<Keypair>>,
    mint_account: &Pubkey,
    token_accounts: Vec<Pubkey>,
    token_2022_program_id: &Pubkey,
    keypair: &Keypair,
) -> Result<Signature, anyhow::Error> {
    info!(
        "🌾 Starting harvest for {} accounts...",
        token_accounts.len()
    );
    let remaining_accounts: Vec<AccountMeta> = token_accounts
        .into_iter()
        .map(|pubkey| AccountMeta {
            pubkey,
            is_signer: false,
            is_writable: true,
        })
        .collect();

    debug!("📝 Building harvest transaction...");
    let tx_hash = program
        .request()
        .accounts(tax_token::accounts::Harvest {
            mint_account: *mint_account,
            token_program: *token_2022_program_id,
        })
        .accounts(remaining_accounts)
        .args(tax_token::instruction::Harvest {})
        .signer(keypair)
        .send()
        .await?;

    info!("✅ Harvest completed with tx: {}", tx_hash);
    Ok(tx_hash)
}

/// Withdraws harvested taxes to the admin's associated token account (ATA)
///
/// # Arguments
/// * `program` - Tax program instance
/// * `mint_account` - Token mint address
/// * `token_2022_program_id` - Token 2022 program ID
/// * `keypair` - Signer's keypair
/// * `authority` - Authority pubkey
/// * `authority_ata` - Authority's ATA pubkey
async fn withdraw(
    program: &Program<Arc<Keypair>>,
    mint_account: &Pubkey,
    token_2022_program_id: &Pubkey,
    keypair: &Keypair,
    authority: &Pubkey,
    authority_ata: &Pubkey,
) -> Result<Signature, anyhow::Error> {
    info!("💸 Initiating withdrawal...");
    let tx_hash = program
        .request()
        .accounts(tax_token::accounts::Withdraw {
            authority: *authority,
            mint_account: *mint_account,
            token_account: *authority_ata,
            token_program: *token_2022_program_id,
        })
        .args(tax_token::instruction::Withdraw)
        .signer(keypair)
        .send()
        .await?;

    info!("✅ Withdrawal completed with tx: {}", tx_hash);
    Ok(tx_hash)
}

#[derive(BorshSerialize, BorshDeserialize)]
#[borsh(crate = "borsh")]
pub struct SwapV2 {
    amount: u64,
    other_amount_threshold: u64,
    sqrt_price_limit_x64: u128,
    is_base_input: bool,
}

/// Executes a swap on the Raydium Concentrated Liquidity Market Maker (CLMM)
///
/// # Arguments
/// * `rpc_client` - Solana RPC client
/// * `program_id` - CLMM program ID
/// * `payer` - Transaction signer
/// * `pool_state` - Pool state pubkey
/// * `[...]` - Various account pubkeys and parameters
async fn swap_clmm(
    rpc_client: &RpcClient,
    program_id: &Pubkey,
    payer: &Keypair,
    pool_state: Pubkey,
    amm_config: Pubkey,
    input_token_account: Pubkey,
    output_token_account: Pubkey,
    input_vault: Pubkey,
    output_vault: Pubkey,
    observation_state: Pubkey,
    input_token_mint: Pubkey,
    output_token_mint: Pubkey,
    input_token_program: Pubkey,
    output_token_program: Pubkey,
    amount_in: u64,
) -> Result<Signature, anyhow::Error> {
    info!("🔄 Starting CLMM swap of {} tokens...", amount_in);

    let accounts = vec![
        AccountMeta::new(payer.pubkey(), true),
        AccountMeta::new(amm_config, false),
        AccountMeta::new(pool_state, false),
        AccountMeta::new(input_token_account, false),
        AccountMeta::new(output_token_account, false),
        AccountMeta::new(input_vault, false),
        AccountMeta::new(output_vault, false),
        AccountMeta::new(observation_state, false),
        AccountMeta::new(output_token_program, false),
        AccountMeta::new(input_token_program, false),
        AccountMeta::new(
            Pubkey::from_str("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr")?,
            false,
        ),
        AccountMeta::new(input_token_mint, false),
        AccountMeta::new(output_token_mint, false),
        AccountMeta::new(
            Pubkey::from_str("4xsACyKdzNbpE7QteqyGvXvuvQCrYmho1W1HPbWS6QWN")?,
            false,
        ),
        AccountMeta::new(
            Pubkey::from_str("8xoo3dW62Fa6jpnwY3Rv3EiuhBqaVryHRoNzVcUoogrx")?,
            false,
        ),
        AccountMeta::new(
            Pubkey::from_str("47Wkp1HLuHhYkiVhVK32JeWofK7Ur65hW2JwDNjWJyJG")?,
            false,
        ),
    ];

    let instruction_data = SwapV2 {
        amount: amount_in,
        other_amount_threshold: 0,
        sqrt_price_limit_x64: 0,
        is_base_input: true,
    };

    debug!("📝 Preparing swap instruction...");
    let discriminant = get_discriminant("global", "swap_v2");
    let ix = Instruction::new_with_borsh(
        program_id.clone(),
        &(discriminant, instruction_data),
        accounts,
    );

    let blockhash = rpc_client.get_latest_blockhash().await?;
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &blockhash);
    let mut tx = Transaction::new_unsigned(msg);
    tx.sign(&[payer], tx.message.recent_blockhash);

    debug!("📤 Sending swap transaction...");
    let tx_hash = rpc_client
        .send_transaction_with_config(
            &tx,
            RpcSendTransactionConfig {
                skip_preflight: false,
                preflight_commitment: Some(CommitmentLevel::Confirmed),
                ..Default::default()
            },
        )
        .await;

    match tx_hash {
        Ok(hash) => {
            info!("✅ Swap completed with tx: {}", hash);
            Ok(hash)
        }
        Err(e) => {
            error!("❌ Swap failed: {:?}", e);
            if let solana_client::client_error::ClientErrorKind::RpcError(
                solana_client::rpc_request::RpcError::RpcResponseError { data, .. },
            ) = &e.kind
            {
                if let RpcResponseErrorData::SendTransactionPreflightFailure(preflight) = data {
                    warn!("Preflight failure logs:");
                    if let Some(logs) = &preflight.logs {
                        for (i, log) in logs.iter().enumerate() {
                            warn!("Log {}: {}", i, log);
                        }
                    }
                }
            }
            Err(anyhow!("Transaction failed: {:?}", e))
        }
    }
}

/// Distributes rewards to token holders proportionally
///
/// # Arguments
/// * `rpc_client` - Solana RPC client
/// * `client` - Anchor client instance
/// * `tax_token_mint` - Taxed token mint
/// * `reward_token_mint` - Reward token mint
/// * `total_rewards` - Total reward amount to distribute
/// * `payer` - Transaction signer
/// * `[...]` - Program IDs
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
    info!(
        "🎁 Starting reward distribution of {} tokens...",
        total_rewards
    );
    let mint_info = rpc_client.get_token_supply(tax_token_mint).await?;
    let reward_info = rpc_client.get_token_supply(reward_token_mint).await?;
    let total_supply = mint_info.ui_amount.unwrap_or(0.0) as u64;

    debug!("📋 Fetching token accounts for distribution...");
    let accounts =
        get_token_accounts(&tax_token_mint, None, 1, 1000, None, None, None, false).await;
    if accounts.is_err() {
        error!("❌ Failed to fetch holders");
        return Err(anyhow!("Failed to get holders for harvesting"));
    }
    let accounts = accounts.unwrap();
    let mut distribution_data = Vec::new();
    for (_, (balance, wallet)) in accounts {
        if balance > 0.0 {
            let reward = (balance as u128 * total_rewards as u128 / total_supply as u128) as u64
                * 10u64.pow(reward_info.decimals.into());
            if reward > 0 {
                distribution_data.push((wallet, reward));
            }
        }
    }

    let (admin_reward_ata, _) = Pubkey::find_program_address(
        &[
            payer.pubkey().as_ref(),
            token_program_id.as_ref(),
            reward_token_mint.as_ref(),
        ],
        &ata_program_id,
    );
    let program = client.program(token_program_id)?;

    info!("📤 Distributing to {} holders...", distribution_data.len());
    for (owner, reward) in distribution_data.iter() {
        let owner = Pubkey::from_str(owner)?;
        let (holder_ata, _) = Pubkey::find_program_address(
            &[
                owner.as_ref(),
                token_program_id.as_ref(),
                reward_token_mint.as_ref(),
            ],
            &ata_program_id,
        );

        if rpc_client.get_account(&holder_ata).await.is_err() {
            debug!("🆕 Creating ATA for holder {}", owner);
            let ix = spl_associated_token_account::instruction::create_associated_token_account(
                &payer.pubkey(),
                &owner,
                reward_token_mint,
                &token_program_id,
            );
            program
                .request()
                .instruction(ix)
                .signer(payer)
                .send()
                .await?;
        }

        debug!("💸 Transferring {} rewards to {}", reward, owner);
        let ix = spl_token::instruction::transfer(
            &token_program_id,
            &admin_reward_ata,
            &holder_ata,
            &payer.pubkey(),
            &[&payer.pubkey()],
            *reward,
        )?;
        program
            .request()
            .instruction(ix)
            .signer(payer)
            .send()
            .await?;
    }

    info!(
        "✅ Distributed {} rewards to {} holders",
        total_rewards,
        distribution_data.len()
    );
    Ok(())
}

/// Distributes tokens to holders (alternative distribution method)
///
/// # Arguments
/// * `rpc_client` - Solana RPC client
/// * `client` - Anchor client instance
/// * `tax_token_mint` - Taxed token mint
/// * `total_rewards` - Total reward amount to distribute
/// * `payer` - Transaction signer
/// * `admin_ata` - Admin's ATA
/// * `token_2022_program_id` - Token 2022 program ID
#[allow(unused_variables)]
#[allow(dead_code)]
async fn distribute_tokens(
    rpc_client: RpcClient,
    client: Client<Arc<Keypair>>,
    tax_token_mint: &Pubkey,
    total_rewards: u64,
    payer: &Keypair,
    admin_ata: &Pubkey,
    token_2022_program_id: Pubkey,
) -> Result<(), anyhow::Error> {
    //     info!(
    //         "🎁 Starting token distribution of {} tokens...",
    //         total_rewards
    //     );
    //     let accounts =
    //         get_token_accounts(&tax_token_mint, None, 1, 1000, None, None, None, false).await;
    //     if accounts.is_err() {
    //         error!("❌ Failed to get holders");
    //         return Err(anyhow!("Failed to get holders"));
    //     }
    //     let accounts = accounts.unwrap();

    //     debug!("📋 Admin ATA: {}", admin_ata);
    //     let admin_account_info = rpc_client.get_account(admin_ata).await?;
    //     if admin_account_info.owner != token_2022_program_id {
    //         error!("❌ Admin ATA is not owned by the expected program id");
    //         return Err(anyhow!("Admin ATA is not owned by the expected program id"));
    //     }

    //     let mint_info = rpc_client.get_token_supply(tax_token_mint).await?;
    //     let mint_account_info = rpc_client.get_account(tax_token_mint).await?;
    //     if mint_account_info.owner != token_2022_program_id {
    //         error!("❌ Mint is not owned by the expected program id");
    //         return Err(anyhow!("Mint is not owned by the expected program id"));
    //     }

    //     let total_supply = mint_info.ui_amount.unwrap_or(0.0) as u64;
    //     debug!("📊 Total supply: {}", total_supply);

    //     let program = client.program(token_2022_program_id)?;
    //     info!("📤 Distributing to {} holders...", accounts.len());

    //     for (acc, (bal, _)) in accounts {
    //         let holder_ata = Pubkey::from_str(&acc)?;
    //         debug!("👤 Holder ATA: {}", holder_ata);
    //         let reward = (bal as u128 * total_rewards as u128 / total_supply as u128) as u64;

    //         let ix = spl_token_2022::instruction::transfer(
    //             &token_2022_program_id,
    //             admin_ata,
    //             &holder_ata,
    //             &payer.pubkey(),
    //             &[&payer.pubkey()],
    //             reward,
    //         )?;
    //         match program.request().instruction(ix).signer(payer).send().await {
    //             Ok(_) => debug!("✅ Transferred {} to {}", reward, holder_ata),
    //             Err(e) => error!("❌ Error distributing to {}: {}", holder_ata, e),
    //         }
    //     }

    //     info!("✅ Distributed reward successfully");
    Ok(())
}
