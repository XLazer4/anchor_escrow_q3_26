use anchor_lang::prelude::*;

#[error_code]
pub enum EscrowError {
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Mint A and mint B must be different")]
    SameMint,
    #[msg("Receive amount changed since the taker last saw the escrow")]
    ReceiveChanged,
    #[msg("Expiry must be in the future")]
    InvalidExpiry,
    #[msg("Escrow has expired")]
    EscrowExpired,
    #[msg("Only the maker can refund before the escrow expires")]
    NotExpired,
    #[msg("This escrow is reserved for a different taker")]
    WrongTaker,
    #[msg("Nothing to update")]
    NothingToUpdate,
}
