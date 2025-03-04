#![allow(unexpected_cfgs)]

use anchor_lang::prelude::*;

mod instructions;
use instructions::*;

declare_id!("C4ZgZJSwHg65gZsLoa9gt7nitzeMFRMD6eK6xMEgdyPg");

const TAX_BASIS_POINT: u16 = 1000; // 10%

#[program]
pub mod tax_token {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, params: InitTokenParams) -> Result<()> {
        process_initialize(ctx, params)
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

    pub fn update_program_state(
        ctx: Context<UpdateProgramState>,
        authority: Option<Pubkey>,
        reward_mint: Option<Pubkey>,
    ) -> Result<()> {
        process_update_program_state(ctx, authority, reward_mint)
    }
}

#[account]
pub struct ProgramState {
    pub authority: Pubkey,
    pub token_mint: Pubkey,
    pub reward_mint: Pubkey,
}

impl ProgramState {
    pub const LEN: usize = 8 + // discriminator
        32 + // authority
        32 + // token_mint
        32; // reward_mint
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

    #[msg("Invalid batch data: recipients and balances can not be empty")]
    EmptyTransferList,

    #[msg("Invalid batch data: recipients and balances arrays must have the same length")]
    InvalidBatchData,

    #[msg("Batch size exceeds maximum allowed")]
    BatchTooLarge,

    #[msg("Batch percentage exceeds 100% allowed")]
    PercentageSumExceeds100,

    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
}
