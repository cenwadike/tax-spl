use anchor_lang::prelude::*;
use anchor_lang::system_program::{create_account, CreateAccount};
use anchor_spl::{associated_token::AssociatedToken, token::Mint as TokenMint};
use anchor_spl::{
    token_2022::{
        initialize_mint2,
        spl_token_2022::{
            extension::{
                transfer_fee::TransferFeeConfig, BaseStateWithExtensions, ExtensionType,
                StateWithExtensions,
            },
            pod::PodMint,
            state::Mint as MintState,
        },
        InitializeMint2,
    },
    token_interface::{
        spl_pod::optional_keys::OptionalNonZeroPubkey, transfer_fee_initialize, Token2022,
        TransferFeeInitialize,
    },
};

use crate::{InitTokenParams, ProgramState, TAX_BASIS_POINT};

pub fn process_initialize(ctx: Context<Initialize>, params: InitTokenParams) -> Result<()> {
    msg!("Initializing SPL token with 6% tax");

    // Initialize the program state
    let state = &mut ctx.accounts.state;
    state.authority = ctx.accounts.authority.key();
    state.token_mint = ctx.accounts.token_mint.key();
    state.reward_mint = ctx.accounts.reward_mint.key();
    state.last_distribution_time = Clock::get()?.unix_timestamp;

    // Calculate space required for mint and extension data
    let mint_size =
        ExtensionType::try_calculate_account_len::<PodMint>(&[ExtensionType::TransferFeeConfig])?;

    // Calculate minimum lamports required for size of mint account with extensions
    let lamports = (Rent::get()?).minimum_balance(mint_size);

    // Invoke System Program to create new account with space for mint and extension data
    create_account(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            CreateAccount {
                from: ctx.accounts.authority.to_account_info(),
                to: ctx.accounts.token_mint.to_account_info(),
            },
        ),
        lamports,                          // Lamports
        mint_size as u64,                  // Space
        &ctx.accounts.token_program.key(), // Owner Program
    )?;

    // Initialize the transfer fee extension data
    // This instruction must come before the instruction to initialize the mint data
    transfer_fee_initialize(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            TransferFeeInitialize {
                token_program_id: ctx.accounts.token_program.to_account_info(),
                mint: ctx.accounts.token_mint.to_account_info(),
            },
        ),
        Some(&ctx.accounts.authority.key()), // transfer fee config authority (update fee)
        Some(&ctx.accounts.authority.key()), // withdraw authority (withdraw fees)
        TAX_BASIS_POINT,                     // transfer fee basis points (% fee per transfer)
        (params.total_supply / 10) as u64,   // maximum fee (maximum units of token per transfer)
    )?;

    // Initialize the standard mint account data
    initialize_mint2(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            InitializeMint2 {
                mint: ctx.accounts.token_mint.to_account_info(),
            },
        ),
        params.decimals,                     // decimals
        &ctx.accounts.authority.key(),       // mint authority
        Some(&ctx.accounts.authority.key()), // freeze authority
    )?;

    ctx.accounts.check_mint_data()?;

    msg!("SPL token with 6% tax initialized successfully");

    Ok(())
}

#[derive(Accounts)]
#[instruction(params: InitTokenParams)]
pub struct Initialize<'info> {
    #[account(
    init,
    payer = authority,
    space = ProgramState::LEN,
    seeds = [b"program_state"],
    bump
)]
    pub state: Account<'info, ProgramState>,

    #[account(mut)]
    pub token_mint: Signer<'info>,

    #[account(mut)]
    pub authority: Signer<'info>,

    /// The reward token mint
    #[account()]
    pub reward_mint: Account<'info, TokenMint>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}

// helper to demonstrate how to read mint extension data within a program
impl<'info> Initialize<'info> {
    pub fn check_mint_data(&self) -> Result<()> {
        let mint = &self.token_mint.to_account_info();
        let mint_data = mint.data.borrow();
        let mint_with_extension = StateWithExtensions::<MintState>::unpack(&mint_data)?;
        let extension_data = mint_with_extension.get_extension::<TransferFeeConfig>()?;

        assert_eq!(
            extension_data.transfer_fee_config_authority,
            OptionalNonZeroPubkey::try_from(Some(self.authority.key()))?
        );

        assert_eq!(
            extension_data.withdraw_withheld_authority,
            OptionalNonZeroPubkey::try_from(Some(self.authority.key()))?
        );

        msg!("Extension Data: {:?}", extension_data);
        Ok(())
    }
}
