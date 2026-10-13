use anchor_lang::prelude::*;

// ---------------------------------------------------------------- PDA seeds
pub const CONFIG_SEED: &[u8] = b"config";
pub const COIN_SEED: &[u8] = b"coin";
pub const VAULT_SEED: &[u8] = b"vault";
pub const MINER_SEED: &[u8] = b"miner";
pub const POSITION_NFT_SEED: &[u8] = b"position_nft";
pub const FEE_SEED: &[u8] = b"fee";

// ---------------------------------------------------------------- supply
pub const DECIMALS: u8 = 6;
/// 1,000,000,000 tokens with 6 decimals.
pub const TOTAL_SUPPLY: u64 = 1_000_000_000_000_000;
/// 20% of supply, mined during the dig.
pub const MINED_SUPPLY: u64 = 200_000_000_000_000;
/// 80% of supply, deposited single-sided into the Meteora pool at open.
pub const POOL_SUPPLY: u64 = 800_000_000_000_000;

// ---------------------------------------------------------------- the dig
pub const BLOCK_SECS: i64 = 10;
/// A share for block n still counts if it lands within this many seconds after
/// block n's window ended and nobody has closed the block yet.
pub const LATE_SHARE_GRACE_SECS: i64 = 3;
/// 15 minutes / 10 s.
pub const MAX_BLOCKS: usize = 90;
pub const MAX_NONCES_PER_TX: usize = 8;
/// Difficulty may move at most 4x easier or 4x harder per block.
pub const RETARGET_CLAMP: u128 = 4;
/// Sentinel for "miner has no open block".
pub const NO_BLOCK: u32 = u32::MAX;

pub const STATE_DIGGING: u8 = 0;
pub const STATE_ENDED: u8 = 1;
pub const STATE_OPENED: u8 = 2;

// ---------------------------------------------------------------- money
pub const BPS: u64 = 10_000;
/// Lamports the launcher parks in the coin's vault PDA at launch. The vault pays
/// rent for the Meteora pool accounts at open (~0.0224 SOL measured from the
/// cp-amm account sizes) and for the temporary fee account on every collect
/// (refunded in the same instruction).
pub const OPEN_RESERVE_LAMPORTS: u64 = 35_000_000;

// ---------------------------------------------------------------- metadata limits (Metaplex)
pub const MAX_NAME_LEN: usize = 32;
pub const MAX_SYMBOL_LEN: usize = 10;
pub const MAX_URI_LEN: usize = 200;

// ---------------------------------------------------------------- defaults for init_config
pub const DEFAULT_LAUNCH_FEE_LAMPORTS: u64 = 50_000_000; // 0.05 SOL
pub const DEFAULT_LAUNCHER_FEE_BPS: u16 = 8_000; // 80% launcher / 20% platform
pub const DEFAULT_SHARE_TARGET: u32 = 400; // shares per 10 s block
/// Expected hashes per share = 2^64 / target = 2^18 = 262,144.
pub const DEFAULT_START_TARGET: u64 = 1u64 << 46;
pub const DEFAULT_OPEN_FDV_LAMPORTS: u64 = 30_000_000_000; // 30 SOL fully diluted at open
pub const DEFAULT_MIN_OPEN_FDV_LAMPORTS: u64 = 10_000_000_000; // 10 SOL
pub const DEFAULT_MAX_OPEN_FDV_LAMPORTS: u64 = 1_000_000_000_000; // 1,000 SOL
/// Hard ceiling for the opening FDV so the sqrt-price math stays in u128.
pub const HARD_MAX_OPEN_FDV_LAMPORTS: u64 = 100_000_000_000_000; // 100k SOL

/// Meteora fee numerator denominator (cp-amm `FEE_DENOMINATOR`).
pub const FEE_DENOMINATOR: u64 = 1_000_000_000;
/// Anti-sniper schedule: 25% at open, minus 3% every 30 s for 8 periods -> 1% after 4 minutes.
pub const DEFAULT_CLIFF_FEE_NUMERATOR: u64 = 250_000_000;
pub const DEFAULT_FEE_PERIODS: u16 = 8;
pub const DEFAULT_FEE_PERIOD_SECS: u64 = 30;
pub const DEFAULT_FEE_REDUCTION: u64 = 30_000_000;
/// cp-amm MIN_FEE_NUMERATOR (0.01%) and MAX_FEE_NUMERATOR_V1 (99%).
pub const MIN_FEE_NUMERATOR: u64 = 100_000;
pub const MAX_FEE_NUMERATOR: u64 = 990_000_000;

// ---------------------------------------------------------------- external programs and sysvars
pub const TOKEN_METADATA_PROGRAM_ID: Pubkey = pubkey!("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");
pub const SLOT_HASHES_ID: Pubkey = pubkey!("SysvarS1otHashes111111111111111111111111111");
pub const NATIVE_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
pub const TOKEN_2022_PROGRAM_ID: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// SPL token account size.
pub const TOKEN_ACCOUNT_LEN: u64 = 165;
