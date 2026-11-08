pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("CX1gEY431CXgVKiWMLdo94QPN49BtKW7fPmrByFZrJ5p");

#[program]
pub mod escrow {
    use super::*;

    /// `duration` is in seconds from now. `taker` locks the offer to one
    /// wallet, None means anyone can take it.
    pub fn make(
        ctx: Context<Make>,
        seed: u64,
        deposit: u64,
        receive: u64,
        duration: i64,
        taker: Option<Pubkey>,
    ) -> Result<()> {
        ctx.accounts
            .init_escrow(seed, receive, duration, taker, &ctx.bumps)?;
        ctx.accounts.deposit(deposit)
    }

    pub fn take(ctx: Context<Take>, expected_receive: u64) -> Result<()> {
        ctx.accounts.check_terms(expected_receive)?;
        ctx.accounts.pay_maker()?;
        ctx.accounts.withdraw_and_close_vault()
    }

    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        ctx.accounts.check_caller()?;
        ctx.accounts.refund_and_close_vault()
    }

    pub fn update(
        ctx: Context<Update>,
        new_receive: Option<u64>,
        new_expires_at: Option<i64>,
    ) -> Result<()> {
        ctx.accounts.update(new_receive, new_expires_at)
    }
}
