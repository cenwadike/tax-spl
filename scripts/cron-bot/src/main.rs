use anchor_client::solana_client;
use anchor_client::solana_client::rpc_config::RpcSendTransactionConfig;
use anchor_client::solana_client::rpc_request::RpcResponseErrorData;
use anchor_client::solana_sdk::hash::Hash;
use anchor_client::{
    solana_client::nonblocking::rpc_client::RpcClient,
    solana_sdk::{
        commitment_config::CommitmentConfig,
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
use solana_sdk::commitment_config::CommitmentLevel;
use solana_sdk::message::Message;
use solana_sdk::transaction::Transaction;
use std::sync::Arc;
use std::{env, str::FromStr, thread, time::Duration};
use tokio;
use utils::{get_discriminant, get_token_accounts};

mod utils;

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

    let tax_program_id =
        env::var("TAX_PROGRAM_ID").expect("TAX_PROGRAM_ID must be set in environment variables");
    let mint_address = env::var("TOKEN_MINT").expect("TOKEN_MINT must be set");
    let reward_token_mint_address = env::var("REWARD_TOKEN_MINT")
        .expect("REWARD_TOKEN_MINT must be set in environment variables");

    let pool_id = env::var("POOL_ID").expect("POOL_ID must be set");
    let base_vault = env::var("BASE_VAULT").expect("BASE_VAULT must be set");
    let quote_vault = env::var("QUOTE_VAULT").expect("QUOTE_VAULT must be set");
    let amm_config = env::var("AMM_CONFIG").expect("AMM_CONFIG must be set");

    let cluster = env::var("SOLANA_NETWORK")
        .unwrap_or("mainnet".to_string())
        .to_lowercase();
    let rpc_url = match cluster.as_str() {
        "devnet" => "https://api.devnet.solana.com".to_string(),
        "mainnet" => "https://api.mainnet-beta.solana.com".to_string(),
        custom => custom.to_string(),
    };

    let interval_secs = env::var("INTERVAL")
        .unwrap_or("3600".to_string())
        .parse::<u64>()
        .expect("Failed to parse INTERVAL");

    loop {
        match process_job(
            &rpc_url,
            &sol_admin_private_key,
            &tax_program_id,
            &mint_address,
            &reward_token_mint_address,
            &base_vault,
            &quote_vault,
            &pool_id,
            &amm_config,
        )
        .await
        {
            Ok(()) => println!("Job completed at {}", chrono::Utc::now()),
            Err(e) => eprintln!("Job failed at {}: {:?}", chrono::Utc::now(), e),
        }
        thread::sleep(Duration::from_secs(interval_secs));
    }
}

/// Processes the main job: harvests taxes, swaps tokens, and distributes rewards
///

/// # Returns
/// A `Result` indicating success or an error if any step fails
async fn process_job(
    sol_rpc_endpoint: &str,
    sol_admin_private_key: &str,
    tax_program_id: &str,
    token_mint_address: &str,
    reward_token_mint_address: &str,
    base_vault: &str,
    quote_vault: &str,
    pool_id: &str,
    amm_config: &str,
) -> Result<(), anyhow::Error> {
    let payer = Keypair::from_base58_string(sol_admin_private_key);
    let client = Client::new(
        Cluster::Custom(sol_rpc_endpoint.to_string(), "".to_string()),
        Arc::new(payer.insecure_clone()),
    );
    let token_mint = Pubkey::from_str(token_mint_address)?;

    let reward_token_mint = Pubkey::from_str(&reward_token_mint_address)?;

    let tax_program_id = Pubkey::from_str(&tax_program_id)?;

    let raydium_clmm_id = Pubkey::from_str("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK")?;
    let token_program_id = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")?;
    let token_2022_program_id = Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb")?;
    let ata_program_id = Pubkey::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL")?;
    let system_program_id = Pubkey::from_str("11111111111111111111111111111111")?;

    let tax_program = client.program(tax_program_id)?;
    let clmm_program = client.program(raydium_clmm_id)?;

    let base_vault = Pubkey::from_str(base_vault)?;
    let quote_vault = Pubkey::from_str(quote_vault)?;

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
    println!("Pre-harvest balance: {}", pre_harvested_balance);

    let holders = get_token_accounts(&token_mint, None, 1, 1000, None, None, None, false).await;
    if holders.is_err() {
        return Err(anyhow!("Failed to get holders for harvesting"));
    }
    let holders = holders.unwrap();
    let token_accounts: Vec<Pubkey> = holders
        .into_iter()
        .map(|(account, _)| Pubkey::from_str(&account))
        .collect::<Result<Vec<_>, _>>()?;
    for chunk in token_accounts.chunks(20) {
        // Batch into groups of 20
        harvest(
            &tax_program,
            &token_mint,
            chunk.to_vec(),
            &token_2022_program_id,
            &payer,
        )
        .await?;
    }

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

    println!("Post-harvest balance: {}", post_harvested_balance);

    let harvested_amount = post_harvested_balance - pre_harvested_balance;

    println!("Harvested amount: {}", harvested_amount);

    if harvested_amount == 0 {
        println!("No tokens harvested, skipping swap and distribution");
        return Ok(());
    }

    // derive reward token ata
    let (output_ata, _) = Pubkey::find_program_address(
        &[
            payer.pubkey().as_ref(),
            token_program_id.as_ref(),
            reward_token_mint.as_ref(),
        ],
        &ata_program_id,
    );

    // check if derived account has been created
    let output_account = rpc_client.get_account(&output_ata).await;

    // create destination ATA if it does not exist
    if output_account.is_err() {
        let recent_blockhash = rpc_client.get_latest_blockhash().await;
        let mut latest_blockhash = Hash::default();
        match recent_blockhash {
            Ok(hash) => {
                latest_blockhash = hash;
            }
            Err(err) => {
                eprintln!("Failed to Get Recent Hash. {}", err)
            }
        }

        // construct create destination ATA instruction
        let create_destination_ata_ix = Instruction {
            program_id: ata_program_id.clone(),
            accounts: vec![
                AccountMeta::new(payer.pubkey().clone(), true),
                AccountMeta::new(output_ata, false),
                AccountMeta::new_readonly(payer.pubkey().clone(), false),
                AccountMeta::new_readonly(reward_token_mint.clone(), false),
                AccountMeta::new_readonly(system_program_id.clone(), false),
                AccountMeta::new_readonly(token_program_id.clone(), false),
            ],
            data: vec![0],
        };

        // create create destination ATA transaction
        let mut transaction =
            Transaction::new_with_payer(&[create_destination_ata_ix], Some(&payer.pubkey()));

        // sign create destination ATA transaction
        transaction.sign(&[&payer], latest_blockhash);

        // send and confirm transaction
        match rpc_client.send_and_confirm_transaction(&transaction).await {
            Ok(signature) => {
                println!("Created associated token account. Tx Hash: {}", signature);
            }
            Err(err) => {
                eprintln!("Failed to create associated token account. {}", err);
            }
        }
    }

    let amount_in = harvested_amount;

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

    // distribute_tokens(
    //     rpc_client,
    //     client,
    //     &token_mint,
    //     reward_balance,
    //     &payer,
    //     &admin_ata,
    //     token_2022_program_id,
    // )
    // .await?;

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
    token_accounts: Vec<Pubkey>,
    token_2022_program_id: &Pubkey,
    keypair: &Keypair,
) -> Result<Signature, anyhow::Error> {
    println!("Starting Harvest...");

    let remaining_accounts: Vec<AccountMeta> = token_accounts
        .into_iter()
        .map(|pubkey| AccountMeta {
            pubkey,
            is_signer: false,
            is_writable: true,
        })
        .collect();

    // Send the transaction with remaining accounts
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

    println!("Completed Harvest...");
    Ok(tx_hash)
}

/// Withdraws harvested taxes to the admin's associated token account (ATA)
async fn withdraw(
    program: &Program<Arc<Keypair>>,
    mint_account: &Pubkey,
    token_2022_program_id: &Pubkey,
    keypair: &Keypair,
    authority: &Pubkey,
    authority_ata: &Pubkey,
) -> Result<Signature, anyhow::Error> {
    println!("Starting withdraw...");

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

    println!("Completed withdraw...");
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
    input_token_mint: Pubkey,
    output_token_mint: Pubkey,
    input_token_program: Pubkey,
    output_token_program: Pubkey,
    amount_in: u64,
) -> Result<Signature, anyhow::Error> {
    println!("Starting swap...");
    let observation_state = Pubkey::from_str("4ujXUVoCPsUtUyWjWAoBe7enohp3WZvReFGboG4vU3NF")?;

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

    let discriminant = get_discriminant("global", "swap_v2");
    let ix = Instruction::new_with_borsh(
        program_id.clone(),
        &(discriminant, instruction_data),
        accounts.clone(),
    );

    // get latest block hash
    let blockhash = rpc_client.get_latest_blockhash().await?;

    // construct message
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &blockhash);

    //construct transaction
    let mut tx = Transaction::new_unsigned(msg);

    // sign transaction
    tx.sign(&[&payer], tx.message.recent_blockhash);

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
            println!("Completed swap with tx: {}", hash);
            Ok(hash)
        }
        Err(e) => {
            if let solana_client::client_error::ClientErrorKind::RpcError(
                solana_client::rpc_request::RpcError::RpcResponseError { data, .. },
            ) = &e.kind
            {
                if let RpcResponseErrorData::SendTransactionPreflightFailure(preflight) = data {
                    println!("Preflight failure logs:");
                    if let Some(logs) = &preflight.logs {
                        for (i, log) in logs.iter().enumerate() {
                            println!("Log {}: {}", i, log);
                        }
                    }
                }
            }
            Err(anyhow::anyhow!("Transaction failed: {:?}", e))
        }
    }
}

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
    let mint_info = rpc_client.get_token_supply(tax_token_mint).await?;
    let total_supply = mint_info.ui_amount.unwrap_or(0.0) as u64;

    let accounts =
        get_token_accounts(&tax_token_mint, None, 1, 1000, None, None, None, false).await;
    if accounts.is_err() {
        return Err(anyhow!("Failed to get holders"));
    }
    let accounts = accounts.unwrap();

    let mut distribution_data = Vec::new();
    for (_, (balance, wallet)) in accounts {
        if balance > 0.0 {
            let reward = (balance as u128 * total_rewards as u128 / total_supply as u128) as u64;

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

    for (owner, reward) in distribution_data.iter() {
        let owner = Pubkey::from_str(&owner)?;
        let (holder_ata, _) = Pubkey::find_program_address(
            &[
                owner.as_ref(),
                token_program_id.as_ref(),
                reward_token_mint.as_ref(),
            ],
            &ata_program_id,
        );

        if rpc_client.get_account(&holder_ata).await.is_err() {
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

    println!(
        "Distributed {} rewards to {} holders",
        total_rewards,
        distribution_data.len()
    );
    Ok(())
}

// async fn distribute_tokens(
//     rpc_client: RpcClient,
//     client: Client<Arc<Keypair>>,
//     tax_token_mint: &Pubkey,
//     total_rewards: u64,
//     payer: &Keypair,
//     admin_ata: &Pubkey,
//     token_2022_program_id: Pubkey,
// ) -> Result<(), anyhow::Error> {
//     let accounts =
//         get_token_accounts(&tax_token_mint, None, 1, 1000, None, None, None, false).await;
//     if accounts.is_err() {
//         return Err(anyhow!("Failed to get holders"));
//     }
//     let accounts = accounts.unwrap();

//     println!("Admin ATA: {}", admin_ata);

//     let admin_account_info = rpc_client.get_account(admin_ata).await?;
//     if admin_account_info.owner != token_2022_program_id {
//         return Err(anyhow!(
//             "Admin ATA is not owned by the expected program id",
//         ));
//     }

//     let mint_info = rpc_client.get_token_supply(tax_token_mint).await?;
//     let mint_account_info = rpc_client.get_account(tax_token_mint).await?;
//     if mint_account_info.owner != token_2022_program_id {
//         return Err(anyhow!("Mint is not owned by the expected program id"));
//     }

//     let total_supply = mint_info.ui_amount.unwrap_or(0.0) as u64;

//     println!("Token 2022 program id: {}", token_2022_program_id);

//     let program = client.program(token_2022_program_id)?;

//     println!("Reached here");

//     for (acc, (bal, _)) in accounts {
//         let holder_ata = Pubkey::from_str(&acc)?;
//         println!("Holder ATA {}", holder_ata);

//         let reward = (bal as u128 * total_rewards as u128 / total_supply as u128) as u64;
//         let ix = spl_token_2022::instruction::transfer(
//             &token_2022_program_id,
//             &admin_ata,
//             &holder_ata,
//             &payer.pubkey(),
//             &[&payer.pubkey()],
//             reward,
//         )?;
//         match program.request().instruction(ix).signer(payer).send().await {
//             Ok(_) => {}
//             Err(e) => {
//                 eprintln!("\nError distributing to {}: {}", holder_ata, e);
//             }
//         }
//     }

//     println!("Distributed reward successfully.");

//     Ok(())
// }
