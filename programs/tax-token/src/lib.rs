use anchor_lang::prelude::*;
use anchor_lang::system_program::{create_account, CreateAccount};
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{Mint as TokenMint, Token, TokenAccount},
    token_2022::ID as TOKEN_2022_ID,
};
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
use spl_token_swap::state::SwapVersion;

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
        msg!("Initializing SPL token with 6% tax");

        // Initialize the program state
        let state = &mut ctx.accounts.state;
        state.authority = ctx.accounts.authority.key();
        state.token_mint = ctx.accounts.token_mint.key();
        state.reward_mint = ctx.accounts.reward_mint.key();
        state.treasury = tax_treasury;
        state.reward_treasury = reward_treasury;
        state.last_distribution_time = Clock::get()?.unix_timestamp;
        state.total_tax_collected = 0;

        // Calculate space required for mint and extension data
        let mint_size = ExtensionType::try_calculate_account_len::<PodMint>(&[
            ExtensionType::TransferFeeConfig,
        ])?;

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
            (params.total_supply / 10) as u64, // maximum fee (maximum units of token per transfer)
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
