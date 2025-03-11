use anchor_lang::prelude::*;
use anchor_lang::solana_program::rent::{
    DEFAULT_EXEMPTION_THRESHOLD, DEFAULT_LAMPORTS_PER_BYTE_YEAR,
};
use anchor_lang::system_program::{create_account, CreateAccount};
use anchor_lang::system_program::{transfer, Transfer};
use anchor_spl::token_interface::{
    metadata_pointer_initialize, token_metadata_initialize, MetadataPointerInitialize, Token2022,
    TokenMetadataInitialize,
};
use anchor_spl::{associated_token::AssociatedToken, token::Mint as TokenMint};
use anchor_spl::{
    metadata::Metadata as Metaplex,
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
        spl_pod::optional_keys::OptionalNonZeroPubkey, transfer_fee_initialize,
        TransferFeeInitialize,
    },
};
use spl_token_metadata_interface::state::TokenMetadata;

use crate::{InitTokenParams, ProgramState, TAX_BASIS_POINT};

pub fn process_initialize(ctx: Context<Initialize>, params: InitTokenParams) -> Result<()> {
    msg!("Initializing SPL token with 10% tax");

    // Initialize the program state
    let state = &mut ctx.accounts.state;
    state.authority = ctx.accounts.authority.key();
    state.token_mint = ctx.accounts.token_mint.key();
    state.reward_mint = ctx.accounts.reward_mint.key();

    // Calculate space required for mint with both TransferFeeConfig and MetadataPointer extensions
    let mint_size = ExtensionType::try_calculate_account_len::<PodMint>(&[
        ExtensionType::TransferFeeConfig,
        ExtensionType::MetadataPointer,
    ])?;

    // Calculate minimum lamports required for size of mint account with extensions
    let lamports = (Rent::get()?).minimum_balance(mint_size);

    // Create the mint account if it doesn't exist
    if ctx.accounts.token_mint.to_account_info().data_is_empty() {
        create_account(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                CreateAccount {
                    from: ctx.accounts.authority.to_account_info(),
                    to: ctx.accounts.token_mint.to_account_info(),
                },
            ),
            lamports,
            mint_size as u64,
            &ctx.accounts.token_program.key(),
        )?;
    }

    // Initialize the TransferFeeConfig extension
    transfer_fee_initialize(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            TransferFeeInitialize {
                token_program_id: ctx.accounts.token_program.to_account_info(),
                mint: ctx.accounts.token_mint.to_account_info(),
            },
        ),
        Some(&ctx.accounts.authority.key()), // Transfer fee config authority
        Some(&ctx.accounts.authority.key()), // Withdraw authority
        TAX_BASIS_POINT,                     // Transfer fee basis points
        (params.total_supply / 10) as u64,   // Maximum fee
    )?;

    // Initialize the MetadataPointer extension BEFORE initializing the mint
    metadata_pointer_initialize(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            MetadataPointerInitialize {
                token_program_id: ctx.accounts.token_program.to_account_info(),
                mint: ctx.accounts.token_mint.to_account_info(),
            },
        ),
        Some(ctx.accounts.authority.key()), // Authority for metadata updates
        Some(ctx.accounts.token_mint.key()), // Metadata stored in the mint account itself
    )?;

    // Initialize the mint data (AFTER all extensions are set up)
    initialize_mint2(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            InitializeMint2 {
                mint: ctx.accounts.token_mint.to_account_info(),
            },
        ),
        params.decimals,
        &ctx.accounts.authority.key(),
        Some(&ctx.accounts.authority.key()),
    )?;

    ctx.accounts.check_mint_data()?;

    // Define token metadata
    let token_metadata = TokenMetadata {
        name: params.name.clone(),
        symbol: params.symbol.clone(),
        uri: params.uri.clone(),
        ..Default::default()
    };

    // Calculate additional space for metadata
    let data_len = 4 + token_metadata.tlv_size_of()?;
    let additional_lamports =
        data_len as u64 * DEFAULT_LAMPORTS_PER_BYTE_YEAR * DEFAULT_EXEMPTION_THRESHOLD as u64;

    // Transfer additional lamports to mint account for metadata
    transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            Transfer {
                from: ctx.accounts.authority.to_account_info(),
                to: ctx.accounts.token_mint.to_account_info(),
            },
        ),
        additional_lamports,
    )?;

    // Initialize token metadata
    token_metadata_initialize(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            TokenMetadataInitialize {
                token_program_id: ctx.accounts.token_program.to_account_info(),
                mint: ctx.accounts.token_mint.to_account_info(),
                metadata: ctx.accounts.token_mint.to_account_info(),
                mint_authority: ctx.accounts.authority.to_account_info(),
                update_authority: ctx.accounts.authority.to_account_info(),
            },
        ),
        params.name,
        params.symbol,
        params.uri,
    )?;

    msg!("SPL token with 10% tax initialized successfully");
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

    #[account(mut)]
    /// CHECK: UncheckedAccount
    pub metadata: UncheckedAccount<'info>,

    pub token_metadata_program: Program<'info, Metaplex>,
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
