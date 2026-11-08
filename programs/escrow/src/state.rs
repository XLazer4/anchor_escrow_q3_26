use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct Escrow {
    pub seed: u64,
    pub maker: Pubkey,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    /// how much of mint_b the maker wants for the whole deposit
    pub receive: u64,
    /// if set, only this wallet can take the escrow
    pub taker: Option<Pubkey>,
    /// unix timestamp. take works while now < expires_at, from then on
    /// the offer is dead and anyone can push the refund back to the maker
    pub expires_at: i64,
    pub bump: u8,
}
