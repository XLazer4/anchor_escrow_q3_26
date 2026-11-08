use anchor_lang::prelude::*;

use crate::{constants::ESCROW_SEED, error::EscrowError, state::Escrow};

#[derive(Accounts)]
pub struct Update<'info> {
    pub maker: Signer<'info>,

    #[account(
        mut,
        has_one = maker,
        seeds = [ESCROW_SEED, maker.key().as_ref(), escrow.seed.to_le_bytes().as_ref()],
        bump = escrow.bump
    )]
    pub escrow: Account<'info, Escrow>,
}

impl<'info> Update<'info> {
    /// Either field can be left as None to keep it. Setting a new expiry on
    /// an already expired escrow puts it back on the market, which is fine,
    /// it's the maker's deposit.
    pub fn update(&mut self, new_receive: Option<u64>, new_expires_at: Option<i64>) -> Result<()> {
        require!(
            new_receive.is_some() || new_expires_at.is_some(),
            EscrowError::NothingToUpdate
        );

        if let Some(receive) = new_receive {
            require!(receive > 0, EscrowError::InvalidAmount);
            self.escrow.receive = receive;
        }

        if let Some(expires_at) = new_expires_at {
            let now = Clock::get()?.unix_timestamp;
            require!(expires_at > now, EscrowError::InvalidExpiry);
            self.escrow.expires_at = expires_at;
        }

        Ok(())
    }
}
