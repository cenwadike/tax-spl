use reqwest::Client;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;

#[derive(Serialize)]
struct RpcRequest {
    id: String,
    jsonrpc: String,
    method: String,
    params: Params,
}

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

#[derive(Serialize)]
struct Options {
    #[serde(rename = "showZeroBalance")]
    show_zero_balance: bool,
}

#[derive(Deserialize, Debug)]
struct TokenAccount {
    address: String, // ATA address
    owner: String,   // Wallet address
    amount: u64,     // Raw balance
                     // decimals not provided, assume 9 for WSOL
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct ResultData {
    total: u32, // Total accounts returned in this page
    limit: u32, // Max accounts per page
    page: u32,  // Current page
    token_accounts: Vec<TokenAccount>,
}

#[derive(Deserialize, Debug)]
struct RpcResponse {
    result: ResultData,
}

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
    let api_key = "d1e0db78-2d9b-411a-b40e-c879d96bf3e4";
    let url = format!("https://mainnet.helius-rpc.com/?api-key={}", api_key);
    let client = Client::new();

    let mut page = page;

    let mut accounts = HashMap::new();

    loop {
        let request = RpcRequest {
            id: "text".to_string(),
            jsonrpc: "2.0".to_string(),
            method: "getTokenAccounts".to_string(),
            params: Params {
                mint: mint.to_string(),
                owner: owner.clone(),
                page,
                limit,
                cursor: cursor.clone(),
                before: before.clone(),
                after: after.clone(),
                options: Options { show_zero_balance },
            },
        };

        // println!("Sending request: {:?}", serde_json::to_string(&request)?);

        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            // let error_text = ;
            return Err(format!(
                "Request failed: {} - {}",
                response.status(),
                response.text().await?
            )
            .into());
        }

        let raw_response = response.text().await?;
        // println!("Raw response: {}", raw_response);

        let data: RpcResponse = serde_json::from_str(&raw_response)?;

        let num_accounts = data.result.token_accounts.len();
        for account in data.result.token_accounts {
            let balance = account.amount as f64 / 1_000_000_000.0; // WSOL has 9 decimals
            accounts.insert(account.address, (balance, account.owner));
        }

        // println!("Acc: {:?}", accounts);

        // Stop if fewer accounts than limit are returned (indicates last page)
        if num_accounts < limit as usize {
            println!("Fetched all accounts. Total pages: {}", page);
            break;
        }

        page += 1;
    }

    Ok(accounts)
}

pub fn get_discriminant(namespace: &str, name: &str) -> [u8; 8] {
    let preimage = format!("{}:{}", namespace, name);

    let mut sighash = [0u8; 8];
    sighash.copy_from_slice(
        &anchor_client::anchor_lang::solana_program::hash::hash(preimage.as_bytes()).to_bytes()
            [..8],
    );
    sighash
}
