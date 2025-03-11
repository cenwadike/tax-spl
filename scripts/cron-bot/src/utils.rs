use log::{debug, error, info, LevelFilter};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::{collections::HashMap, env};

/// Initializes logging with colorful output and timestamps, reading level from .env
pub fn setup_logging() {
    let log_level = env::var("LOG_LEVEL")
        .unwrap_or_else(|_| "INFO".to_string())
        .to_uppercase();

    let level_filter = match log_level.as_str() {
        "DEBUG" => LevelFilter::Debug,
        "INFO" => LevelFilter::Info,
        "WARN" => LevelFilter::Warn,
        "ERROR" => LevelFilter::Error,
        "OFF" => LevelFilter::Off,
        _ => {
            println!("Invalid LOG_LEVEL '{}', defaulting to INFO", log_level);
            LevelFilter::Info
        }
    };

    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "\x1B[{}m[{}] [{}] {}\x1B[0m",
                match record.level() {
                    log::Level::Error => 31, // Red
                    log::Level::Warn => 33,  // Yellow
                    log::Level::Info => 32,  // Green
                    log::Level::Debug => 34, // Blue
                    _ => 0,                  // Default
                },
                chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                message
            ))
        })
        .level(level_filter)
        .chain(std::io::stdout())
        .apply()
        .expect("Failed to initialize logging");
    info!("Logging initialized with level: {}", log_level);
}

/// RPC request structure for Helius API
#[derive(Serialize)]
struct RpcRequest {
    id: String,
    jsonrpc: String,
    method: String,
    params: Params,
}

/// Parameters for the RPC request
#[derive(Serialize)]
struct Params {
    mint: String,
    owner: Option<String>,
    page: u32,
    limit: u32,
    cursor: Option<String>,
    before: Option<String>,
    after: Option<String>,
    options: Options,
}

/// Options for the RPC request
#[derive(Serialize)]
struct Options {
    #[serde(rename = "showZeroBalance")]
    show_zero_balance: bool,
}

/// Structure representing a token account from the API response
#[derive(Deserialize, Debug)]
struct TokenAccount {
    address: String, // ATA address
    owner: String,   // Wallet address
    amount: u64,     // Raw balance (assumes 9 decimals for WSOL)
}

/// Result data from the RPC response
#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct ResultData {
    total: u32, // Total accounts returned in this page
    limit: u32, // Max accounts per page
    page: u32,  // Current page
    token_accounts: Vec<TokenAccount>,
}

/// RPC response structure
#[derive(Deserialize, Debug)]
struct RpcResponse {
    result: ResultData,
}

/// Fetches token accounts for a given mint from the Helius RPC API
///
/// # Arguments
/// * `mint` - The token mint public key
/// * `owner` - Optional owner filter
/// * `page` - Starting page number
/// * `limit` - Number of accounts per page
/// * `cursor` - Optional cursor for pagination
/// * `before` - Optional before cursor
/// * `after` - Optional after cursor
/// * `show_zero_balance` - Whether to include accounts with zero balance
///
/// # Returns
/// A HashMap mapping ATA addresses to tuples of (balance as f64, owner address)
pub async fn get_token_accounts(
    mint: &Pubkey,
    owner: Option<String>,
    page: u32,
    limit: u32,
    cursor: Option<String>,
    before: Option<String>,
    after: Option<String>,
    show_zero_balance: bool,
) -> Result<HashMap<String, (f64, String)>, Box<dyn std::error::Error>> {
    info!("🚀 Fetching token accounts for mint: {}", mint);

    let url = env::var("HELIUS_RPC").expect("HELIUS_RPC must be set in environment variables");
    debug!("🌐 Using Helius RPC URL: {}", url);

    let client = Client::new();
    let mut current_page = page;
    let mut accounts = HashMap::new();

    loop {
        info!("📄 Requesting page {} with limit {}", current_page, limit);
        let request = RpcRequest {
            id: "text".to_string(),
            jsonrpc: "2.0".to_string(),
            method: "getTokenAccounts".to_string(),
            params: Params {
                mint: mint.to_string(),
                owner: owner.clone(),
                page: current_page,
                limit,
                cursor: cursor.clone(),
                before: before.clone(),
                after: after.clone(),
                options: Options { show_zero_balance },
            },
        };

        debug!(
            "📤 Sending RPC request: {:?}",
            serde_json::to_string(&request)?
        );

        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            error!("❌ Request failed: {} - {}", status, error_text);
            return Err(format!("Request failed: {} - {}", status, error_text).into());
        }

        let raw_response = response.text().await?;
        debug!("📥 Raw response received: {}", raw_response);

        let data: RpcResponse = serde_json::from_str(&raw_response)?;
        let num_accounts = data.result.token_accounts.len();
        info!(
            "✅ Received {} accounts for page {}",
            num_accounts, current_page
        );

        for account in data.result.token_accounts {
            let balance = account.amount as f64 / 1_000_000_000.0; // Assuming 9 decimals (WSOL standard)
            debug!(
                "💰 Account {}: balance={}, owner={}",
                account.address, balance, account.owner
            );
            accounts.insert(account.address, (balance, account.owner));
        }

        // Check if this is the last page
        if num_accounts < limit as usize {
            info!("🏁 Fetched all accounts. Total pages: {}", current_page);
            break;
        }

        current_page += 1;
        debug!("⏩ Advancing to next page: {}", current_page);
    }

    info!("📊 Total accounts fetched: {}", accounts.len());
    Ok(accounts)
}

/// Computes the Solana Anchor instruction discriminant (8-byte signature hash)
///
/// # Arguments
/// * `namespace` - The namespace of the instruction (e.g., "global")
/// * `name` - The name of the instruction (e.g., "swap_v2")
///
/// # Returns
/// An 8-byte array representing the instruction discriminant
pub fn get_discriminant(namespace: &str, name: &str) -> [u8; 8] {
    let preimage = format!("{}:{}", namespace, name);
    debug!("🔍 Computing discriminant for: {}", preimage);

    let mut sighash = [0u8; 8];
    sighash.copy_from_slice(
        &anchor_client::anchor_lang::solana_program::hash::hash(preimage.as_bytes()).to_bytes()
            [..8],
    );
    debug!("✅ Discriminant computed: {:?}", sighash);
    sighash
}
