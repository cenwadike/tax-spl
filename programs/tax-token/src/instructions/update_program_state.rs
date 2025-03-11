use anchor_lang::prelude::*;

use crate::ProgramState;

#[derive(Accounts)]
pub struct UpdateProgramState<'info> {
    #[account(mut)]
    pub state: Account<'info, ProgramState>,

    pub authority: Signer<'info>,
}

pub fn process_update_program_state(
    ctx: Context<UpdateProgramState>,
    authority: Option<Pubkey>,
    reward_mint: Option<Pubkey>,
) -> Result<()> {
    let state: &mut Account<'_, ProgramState> = &mut ctx.accounts.state;

    assert_eq!(state.authority, ctx.accounts.authority.key());

    if authority.is_some() {
        state.authority = authority.unwrap();
    }

    if reward_mint.is_some() {
        state.reward_mint = reward_mint.unwrap();
    }

    Ok(())
}
