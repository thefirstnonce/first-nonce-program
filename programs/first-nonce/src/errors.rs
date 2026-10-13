use anchor_lang::prelude::*;

#[error_code]
pub enum NonceError {
    #[msg("New launches are paused")]
    Paused,
    #[msg("Dig length must be 5, 10 or 15 minutes")]
    InvalidDigLength,
    #[msg("Name is empty or longer than 32 bytes")]
    InvalidName,
    #[msg("Symbol is empty or longer than 10 bytes")]
    InvalidSymbol,
    #[msg("Uri is longer than 200 bytes")]
    InvalidUri,
    #[msg("Opening market cap is outside the allowed range")]
    InvalidOpenFdv,
    #[msg("Fee split must be at most 10000 bps")]
    InvalidFeeBps,
    #[msg("Invalid pool fee schedule")]
    InvalidFeeSchedule,
    #[msg("Share target must be between 1 and 10000")]
    InvalidShareTarget,
    #[msg("Start target must be non-zero")]
    InvalidStartTarget,
    #[msg("Opening market cap range is invalid")]
    InvalidFdvRange,
    #[msg("No nonces in this submission")]
    NoNonces,
    #[msg("At most 8 nonces per submission")]
    TooManyNonces,
    #[msg("Nonces must be strictly increasing per miner per block")]
    NonceNotIncreasing,
    #[msg("A submitted hash does not meet the share target")]
    ShareAboveTarget,
    #[msg("The dig is not over yet")]
    DigNotOver,
    #[msg("Coin is not open for trading yet")]
    NotOpen,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("SlotHashes sysvar data is malformed")]
    InvalidSlotHashes,
    #[msg("Sqrt price is outside the Meteora range")]
    InvalidSqrtPrice,
    #[msg("Some mined tokens are still unclaimed")]
    UnclaimedTokens,
    #[msg("Serialization failed")]
    Serialization,
    #[msg("Account is not a valid token account")]
    InvalidTokenAccount,
    #[msg("Signer is not allowed to do this")]
    Unauthorized,
    #[msg("This block is closed: its shares no longer count")]
    BlockClosed,
    #[msg("This block is not open yet")]
    BlockNotOpen,
    #[msg("Wrong token program for this coin's mint")]
    InvalidTokenProgram,
    #[msg("Account is not this coin's launcher")]
    InvalidLauncher,
}
