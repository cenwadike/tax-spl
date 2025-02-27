use anchor_lang::prelude::*;
use anchor_spl::{
    token::{Mint as TokenMint, Token, TokenAccount},
    token_2022::ID as TOKEN_2022_ID,
};

use spl_token_swap::state::SwapVersion;

mod instructions;
use instructions::*;

declare_id!("6wgDw4z2yv7eqJnuvZFgyGE3m4pVGnd77pGsjPdc6z8B");

const EPOCH_DURATION: i64 = 60 * 60; // 1 hour in seconds
const TAX_BASIS_POINT: u16 = 600; // 6%
const POOL_TOKEN_DECIMALS: u8 = 9;
const MAX_BATCH_SIZE: usize = 25; // Maximum number of recipients in a batch

#[program]
pub mod tax_token {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        params: InitTokenParams,
        tax_treasury: Pubkey,
        reward_treasury: Pubkey,
    ) -> Result<()> {
        process_initialize(ctx, params, tax_treasury, reward_treasury)
    }

    pub fn transfer(ctx: Context<Transfer>, amount: u64) -> Result<()> {
        process_transfer(ctx, amount)
    }
    
    pub fn harvest<'info>(ctx: Context<'_, '_, 'info, 'info, Harvest<'info>>) -> Result<()> {
        process_harvest(ctx)
    }

    pub fn withdraw(ctx: Context<Withdraw>) -> Result<()> {
        process_withdraw(ctx)
    }

    pub fn update_fee(
        ctx: Context<UpdateFee>,
        transfer_fee_basis_points: u16,
        maximum_fee: u64,
    ) -> Result<()> {
        process_update_fee(ctx, transfer_fee_basis_points, maximum_fee)
    }
}

#[derive(Accounts)]
pub struct CreatePool<'info> {
    #[account(
        mut,
        seeds = [b"program_state"],
        bump
    )]
    pub state: Account<'info, ProgramState>,

    /// The token mint
    pub token_mint: Account<'info, TokenMint>,

    /// The reward token mint
    pub reward_mint: Account<'info, TokenMint>,

    /// Empty account for swap pool state
    /// CHECK:
    #[account(
        init,
        payer = authority,
        space = SwapVersion::LATEST_LEN,
    )]
    pub swap_pool: AccountInfo<'info>,

    /// PDA that will act as the authority for the pool
    /// Will be derived in the instruction
    /// CHECK:
    pub swap_authority: AccountInfo<'info>,

    /// Token A account (SPL token)
    #[account(
        init,
        payer = authority,
        token::mint = token_mint,
        token::authority = swap_authority,
    )]
    pub token_a_account: Box<Account<'info, TokenAccount>>,

    /// Token B account (Reward token)
    #[account(
        init,
        payer = authority,
        token::mint = reward_mint,
        token::authority = swap_authority,
    )]
    pub token_b_account: Box<Account<'info, TokenAccount>>,

    /// Pool token mint
    #[account(
        init,
        payer = authority,
        mint::decimals = POOL_TOKEN_DECIMALS,
        mint::authority = swap_authority,
    )]
    pub pool_token_mint: Box<Account<'info, TokenMint>>,

    /// Account to collect trading fees
    #[account(
        init,
        payer = authority,
        token::mint = pool_token_mint,
        token::authority = authority,
    )]
    pub pool_token_fee_account: Box<Account<'info, TokenAccount>>,

    /// Account to receive pool tokens
    #[account(
        init,
        payer = authority,
        token::mint = pool_token_mint,
        token::authority = authority,
    )]
    pub pool_token_recipient: Box<Account<'info, TokenAccount>>,

    /// The authority (admin) of the program
    #[account(mut)]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,

    /// CHECK:
    pub token_swap_program: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct SwapTaxForReward<'info> {
    #[account(
        mut,
        seeds = [b"program_state"],
        bump
    )]
    pub state: Account<'info, ProgramState>,

    /// The token mint
    pub token_mint: Account<'info, TokenMint>,

    /// The reward token mint
    pub reward_mint: Account<'info, TokenMint>,

    /// Treasury that holds the collected tax
    #[account(
        mut,
        constraint = treasury.mint == token_mint.key(),
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    /// Treasury that will hold the reward tokens
    #[account(
        mut,
        constraint = reward_treasury.mint == reward_mint.key(),
    )]
    pub reward_treasury: Box<Account<'info, TokenAccount>>,

    /// The swap pool
    /// CHECK:
    #[account(mut)]
    pub swap_pool: AccountInfo<'info>,

    /// The swap authority
    /// This is a PDA that will be verified in the instruction
    /// CHECK:
    pub swap_authority: AccountInfo<'info>,

    /// Token A account in the swap pool
    #[account(
        mut,
        constraint = token_a_account.key() == state.token_mint,
    )]
    pub token_a_account: Box<Account<'info, TokenAccount>>,

    /// Token B account in the swap pool
    #[account(
        mut,
        constraint = token_b_account.key() == state.reward_mint,
    )]
    pub token_b_account: Box<Account<'info, TokenAccount>>,

    /// Pool token mint
    #[account(mut)]
    pub pool_token_mint: Box<Account<'info, TokenMint>>,

    /// Pool token fee account
    #[account(mut)]
    pub pool_token_fee_account: Box<Account<'info, TokenAccount>>,

    /// The authority (admin) of the program
    #[account(
        mut,
        constraint = authority.key() == state.authority,
    )]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,

    /// CHECK:
    pub token_swap_program: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct DistributeRewardsBatch<'info> {
    #[account(
        mut,
        seeds = [b"program_state"],
        bump
    )]
    pub state: Account<'info, ProgramState>,

    /// The token mint
    pub token_mint: Account<'info, TokenMint>,

    /// The reward token mint
    pub reward_mint: Account<'info, TokenMint>,

    /// Treasury that holds the reward tokens
    #[account(
        mut,
        constraint = reward_treasury.mint == reward_mint.key(),
        constraint = reward_treasury.key() == state.reward_treasury,
    )]
    pub reward_treasury: Box<Account<'info, TokenAccount>>,

    /// The authority (admin) of the program
    #[account(
        constraint = authority.key() == state.authority,
    )]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct UpdateTransferFee<'info> {
    #[account(
        seeds = [b"program_state"],
        bump
    )]
    pub state: Account<'info, ProgramState>,

    /// The token mint with transfer fee extension
    #[account(
        mut,
        owner = TOKEN_2022_ID,
    )]
    pub token_mint: Account<'info, TokenMint>,

    /// The authority (admin) of the program
    #[account(
        constraint = authority.key() == state.authority,
    )]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

#[account]
pub struct ProgramState {
    pub authority: Pubkey,
    pub token_mint: Pubkey,
    pub reward_mint: Pubkey,
    pub treasury: Pubkey,
    pub reward_treasury: Pubkey,
    pub last_distribution_time: i64,
    pub total_tax_collected: u64,
}

impl ProgramState {
    pub const LEN: usize = 8 + // discriminator
        32 + // authority
        32 + // token_mint
        32 + // reward_mint
        32 + // treasury
        32 + // reward_treasury
        8 +  // last_distribution_time
        8; // total_tax_collected
}

#[derive(AnchorSerialize, AnchorDeserialize, Debug, Clone)]
pub struct InitTokenParams {
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub decimals: u8,
    pub total_supply: u128,
}

#[error_code]
pub enum ErrorCode {
    #[msg("Unauthorized access")]
    UnauthorizedAccess,

    #[msg("Insufficient tax collected for swap")]
    InsufficientTaxCollected,

    #[msg("Insufficient rewards for distribution")]
    InsufficientRewards,

    #[msg("Invalid token supply")]
    InvalidTokenSupply,

    #[msg("Distribution too early, must wait for next epoch")]
    DistributionTooEarly,

    #[msg("Invalid batch data: recipients and balances arrays must have the same length")]
    InvalidBatchData,

    #[msg("Batch size exceeds maximum allowed")]
    BatchTooLarge,

    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
}
