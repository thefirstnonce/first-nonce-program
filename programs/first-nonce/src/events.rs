use anchor_lang::prelude::*;

#[event]
pub struct CoinLaunched {
    pub coin: Pubkey,
    pub mint: Pubkey,
    pub launcher: Pubkey,
    pub dig_minutes: u8,
    pub total_blocks: u16,
    pub dig_start: i64,
    pub dig_end: i64,
    pub open_fdv_lamports: u64,
    pub start_target: u64,
}

#[event]
pub struct SharesSubmitted {
    pub coin: Pubkey,
    pub beneficiary: Pubkey,
    pub block: u32,
    /// Number of shares accepted (== nonces.len(); stale submissions fail instead).
    pub accepted: u8,
    /// Shares in this block so far, this submission included.
    pub block_shares: u64,
    /// The accepted nonces, so anyone can recompute each share hash.
    pub nonces: Vec<u64>,
    /// Challenge and target the shares were checked against.
    pub challenge: [u8; 32],
    pub target: u64,
}

#[event]
pub struct BlockAdvanced {
    pub coin: Pubkey,
    /// Index of the block that is now open (== total_blocks when the dig ended).
    pub current_block: u32,
    pub target: u64,
    pub challenge: [u8; 32],
    pub ended: bool,
}

#[event]
pub struct CoinOpened {
    pub coin: Pubkey,
    pub pool: Pubkey,
    pub position: Pubkey,
    pub sqrt_price: u128,
    pub liquidity: u128,
    pub pool_tokens: u64,
    pub burned: u64,
    pub allocated: u64,
    pub unmined: u64,
}

#[event]
pub struct Claimed {
    pub coin: Pubkey,
    pub beneficiary: Pubkey,
    pub amount: u64,
    pub total_shares: u64,
}

#[event]
pub struct FeesCollected {
    pub coin: Pubkey,
    pub amount: u64,
    pub to_launcher: u64,
    pub to_treasury: u64,
}

/// Unused open reserve returned from the coin's vault to its launcher (in `open` or `collect`).
#[event]
pub struct ReserveRefunded {
    pub coin: Pubkey,
    pub launcher: Pubkey,
    pub amount: u64,
}
