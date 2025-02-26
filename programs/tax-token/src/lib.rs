use anchor_lang::prelude::*;

declare_id!("6wgDw4z2yv7eqJnuvZFgyGE3m4pVGnd77pGsjPdc6z8B");

#[program]
pub mod tax_token {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        msg!("Greetings from: {:?}", ctx.program_id);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
