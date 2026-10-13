use anchor_lang::prelude::*;

use crate::{constants::*, errors::NonceError, mining};

/// Meteora DAMM v2 base fee, time-scheduler mode (linear). Numerators are out of
/// 1,000,000,000. Fee at time t after open:
/// `cliff_fee_numerator - min(t / period_frequency, number_of_period) * reduction_factor`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq, Default)]
pub struct FeeSchedule {
    pub cliff_fee_numerator: u64,
    pub number_of_period: u16,
    pub period_frequency: u64,
    pub reduction_factor: u64,
}

impl FeeSchedule {
    pub fn validate(&self) -> Result<()> {
        let any = self.number_of_period != 0 || self.period_frequency != 0 || self.reduction_factor != 0;
        if any {
            require!(
                self.number_of_period != 0 && self.period_frequency != 0 && self.reduction_factor != 0,
                NonceError::InvalidFeeSchedule
            );
        }
        let total_reduction = (self.number_of_period as u128)
            .checked_mul(self.reduction_factor as u128)
            .ok_or(NonceError::MathOverflow)?;
        require!(
            (self.cliff_fee_numerator as u128) > total_reduction,
            NonceError::InvalidFeeSchedule
        );
        let min_fee = (self.cliff_fee_numerator as u128) - total_reduction;
        require!(
            min_fee >= MIN_FEE_NUMERATOR as u128 && self.cliff_fee_numerator <= MAX_FEE_NUMERATOR,
            NonceError::InvalidFeeSchedule
        );
        Ok(())
    }

    /// Borsh bytes of cp-amm `BorshFeeTimeScheduler` (27 bytes, mode byte at offset 26).
    pub fn to_base_fee_data(&self) -> [u8; 27] {
        let mut out = [0u8; 27];
        out[0..8].copy_from_slice(&self.cliff_fee_numerator.to_le_bytes());
        out[8..10].copy_from_slice(&self.number_of_period.to_le_bytes());
        out[10..18].copy_from_slice(&self.period_frequency.to_le_bytes());
        out[18..26].copy_from_slice(&self.reduction_factor.to_le_bytes());
        out[26] = 0; // BaseFeeMode::FeeTimeSchedulerLinear
        out
    }
}

#[account]
#[derive(InitSpace)]
pub struct Config {
    pub admin: Pubkey,
    /// Receives launch fees and the platform share of pool fees.
    pub treasury: Pubkey,
    pub launch_fee_lamports: u64,
    /// Launcher share of collected pool fees, in bps. The rest goes to the treasury.
    pub launcher_fee_bps: u16,
    pub paused: bool,
    /// Target shares per 10 s block (S).
    pub share_target: u32,
    /// Share target (u64, compared against the first 8 bytes of the share hash) for block 0 of a new coin.
    pub start_target: u64,
    pub default_open_fdv_lamports: u64,
    pub min_open_fdv_lamports: u64,
    pub max_open_fdv_lamports: u64,
    pub fee_schedule: FeeSchedule,
    pub coins_launched: u64,
    pub bump: u8,
    pub reserved: [u8; 64],
}

#[account]
#[derive(InitSpace)]
pub struct Coin {
    pub mint: Pubkey,
    pub launcher: Pubkey,
    pub bump: u8,
    pub vault_bump: u8,
    pub state: u8,
    pub dig_minutes: u8,
    pub total_blocks: u16,
    /// Snapshot of config at launch.
    pub launcher_fee_bps: u16,
    pub share_target: u32,
    pub open_fdv_lamports: u64,
    pub fee_schedule: FeeSchedule,

    pub launched_at: i64,
    pub dig_start: i64,
    pub dig_end: i64,

    /// Index of the open block (== total_blocks once the dig is over).
    pub current_block: u32,
    /// Unix time the open block's challenge was set.
    pub block_open_ts: i64,
    pub challenge: [u8; 32],
    /// A share is valid if u64::from_be_bytes(share[0..8]) < target.
    pub target: u64,
    pub shares_this_block: u64,
    /// Reward rolled forward from empty blocks and rounding dust.
    pub carry: u64,

    pub total_shares: u64,
    /// Tokens assigned to miners through tokens_per_share (sum over blocks of tps * shares).
    pub allocated: u64,
    /// Tokens minted to miners by `claim`.
    pub claimed: u64,
    /// Opening-supply tokens that were never earned (empty final blocks + dust). Never minted.
    pub unmined: u64,

    pub opened_at: i64,
    pub pool: Pubkey,
    pub position: Pubkey,
    pub position_nft_mint: Pubkey,
    pub pool_liquidity: u128,
    pub pool_sqrt_price: u128,
    /// Coin tokens actually deposited in the pool (rest of POOL_SUPPLY is burned).
    pub pool_tokens: u64,

    pub collects: u64,
    pub fees_collected: u64,
    pub fees_to_launcher: u64,
    pub fees_to_treasury: u64,

    /// Tokens (base units) per share for each closed block.
    pub tokens_per_share: [u64; 90],
    /// Share count of each closed block.
    pub block_shares: [u32; 90],
    /// Share target each closed block was mined at (empty blocks included). The open block's
    /// target is `target`. Expected hashes for block b = block_shares[b] * 2^64 / block_targets[b].
    pub block_targets: [u64; 90],
    /// Lifetime count of distinct miners (beneficiaries) that had shares accepted. Miner
    /// accounts close at claim, so counting accounts undercounts.
    pub miners_total: u64,

    pub reserved: [u8; 64],
}

impl Coin {
    pub fn reward_for_block(&self, block: u32) -> u64 {
        mining::reward_for_block(block, self.total_blocks)
    }

    pub fn window_end(&self, block: u32) -> i64 {
        self.dig_start + (block as i64 + 1) * BLOCK_SECS
    }

    /// Lazily close every block whose 10 s window has passed, retarget, and open
    /// the next block with a fresh challenge. Returns true if anything changed.
    pub fn advance(&mut self, now: i64, slot_hash: &[u8; 32]) -> Result<bool> {
        if self.state != STATE_DIGGING {
            return Ok(false);
        }
        let total = self.total_blocks as u32;
        let elapsed = now.saturating_sub(self.dig_start).max(0);
        let due = ((elapsed / BLOCK_SECS) as u64).min(total as u64) as u32;
        if due <= self.current_block {
            return Ok(false);
        }

        // 1. close the open block
        let cur = self.current_block;
        let idx = cur as usize;
        let reward = self
            .reward_for_block(cur)
            .checked_add(self.carry)
            .ok_or(NonceError::MathOverflow)?;
        let shares = self.shares_this_block;
        if shares > 0 {
            let tps = reward / shares;
            let paid = (tps as u128)
                .checked_mul(shares as u128)
                .ok_or(NonceError::MathOverflow)?;
            let paid = u64::try_from(paid).map_err(|_| NonceError::MathOverflow)?;
            self.tokens_per_share[idx] = tps;
            self.allocated = self.allocated.checked_add(paid).ok_or(NonceError::MathOverflow)?;
            self.carry = reward - paid; // rounding dust rolls forward
        } else {
            self.tokens_per_share[idx] = 0;
            self.carry = reward;
        }
        self.block_shares[idx] = u32::try_from(shares).unwrap_or(u32::MAX);
        self.block_targets[idx] = self.target;
        // Only the time the block could actually accept shares counts: from its open until the
        // end of its window plus the late-share grace. A block closed late (nobody cranked)
        // must not look slow and get retargeted easier.
        let accept_end = now.min(self.window_end(cur).saturating_add(LATE_SHARE_GRACE_SECS));
        let duration = accept_end.saturating_sub(self.block_open_ts).max(1);
        self.target = mining::retarget(self.target, shares, duration, self.share_target);

        // 2. blocks nobody opened (no transaction at all during their window) are empty
        let mut b = cur + 1;
        while b < due {
            self.carry = self
                .carry
                .checked_add(self.reward_for_block(b))
                .ok_or(NonceError::MathOverflow)?;
            self.tokens_per_share[b as usize] = 0;
            self.block_shares[b as usize] = 0;
            self.block_targets[b as usize] = self.target;
            self.target = mining::retarget(self.target, 0, BLOCK_SECS, self.share_target);
            b += 1;
        }

        // 3. open the next block, or end the dig
        self.current_block = due;
        self.shares_this_block = 0;
        if due >= total {
            self.state = STATE_ENDED;
            self.unmined = self.carry;
            self.carry = 0;
            self.challenge = [0u8; 32];
        } else {
            self.block_open_ts = now;
            self.challenge = mining::challenge(&self.mint, due, slot_hash);
        }
        Ok(true)
    }
}

#[account]
#[derive(InitSpace)]
pub struct Miner {
    pub coin: Pubkey,
    pub beneficiary: Pubkey,
    /// Paid the rent for this account; refunded when the account is closed at claim.
    pub rent_payer: Pubkey,
    /// Block of the miner's open share run, NO_BLOCK if none.
    pub block: u32,
    pub block_shares: u64,
    /// Smallest nonce still accepted in `block` (last accepted nonce + 1).
    pub next_nonce: u64,
    /// Settled, unclaimed tokens (base units).
    pub pending: u64,
    pub total_shares: u64,
    pub bump: u8,
}

impl Miner {
    /// Move shares of an already-closed block into `pending`.
    pub fn settle(&mut self, coin: &Coin) -> Result<()> {
        if self.block != NO_BLOCK && self.block_shares > 0 && self.block < coin.current_block {
            let tps = coin.tokens_per_share[self.block as usize];
            let amount = (self.block_shares as u128)
                .checked_mul(tps as u128)
                .ok_or(NonceError::MathOverflow)?;
            let amount = u64::try_from(amount).map_err(|_| NonceError::MathOverflow)?;
            self.pending = self.pending.checked_add(amount).ok_or(NonceError::MathOverflow)?;
            self.block_shares = 0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coin(total_blocks: u16) -> Coin {
        Coin {
            mint: Pubkey::new_from_array([7u8; 32]),
            launcher: Pubkey::default(),
            bump: 255,
            vault_bump: 255,
            state: STATE_DIGGING,
            dig_minutes: (total_blocks / 6) as u8,
            total_blocks,
            launcher_fee_bps: DEFAULT_LAUNCHER_FEE_BPS,
            share_target: DEFAULT_SHARE_TARGET,
            open_fdv_lamports: DEFAULT_OPEN_FDV_LAMPORTS,
            fee_schedule: FeeSchedule::default(),
            launched_at: 1_000,
            dig_start: 1_000,
            dig_end: 1_000 + total_blocks as i64 * BLOCK_SECS,
            current_block: 0,
            block_open_ts: 1_000,
            challenge: [1u8; 32],
            target: DEFAULT_START_TARGET,
            shares_this_block: 0,
            carry: 0,
            total_shares: 0,
            allocated: 0,
            claimed: 0,
            unmined: 0,
            opened_at: 0,
            pool: Pubkey::default(),
            position: Pubkey::default(),
            position_nft_mint: Pubkey::default(),
            pool_liquidity: 0,
            pool_sqrt_price: 0,
            pool_tokens: 0,
            collects: 0,
            fees_collected: 0,
            fees_to_launcher: 0,
            fees_to_treasury: 0,
            tokens_per_share: [0u64; 90],
            block_shares: [0u32; 90],
            block_targets: [0u64; 90],
            miners_total: 0,
            reserved: [0u8; 64],
        }
    }

    #[test]
    fn dig_lifecycle_conserves_supply() {
        let sh = [9u8; 32];
        let mut c = coin(60);
        let r = mining::reward_for_block(0, 60);

        // inside block 0: nothing happens
        assert!(!c.advance(1_009, &sh).unwrap());
        // block 0 empty -> reward rolls, 4x easier
        assert!(c.advance(1_010, &sh).unwrap());
        assert_eq!(c.current_block, 1);
        assert_eq!(c.carry, r);
        assert_eq!(c.target, DEFAULT_START_TARGET * 4);
        assert_ne!(c.challenge, [1u8; 32]);

        // block 1: 3 shares over 10 s
        c.shares_this_block = 3;
        c.advance(1_020, &sh).unwrap();
        let tps = (2 * r) / 3;
        assert_eq!(c.tokens_per_share[1], tps);
        assert_eq!(c.block_shares[1], 3);
        assert_eq!(c.allocated, tps * 3);
        assert_eq!(c.carry, 2 * r - tps * 3);

        // miner with those 3 shares settles once the block is closed
        let mut m = Miner {
            coin: Pubkey::default(),
            beneficiary: Pubkey::default(),
            rent_payer: Pubkey::default(),
            block: 1,
            block_shares: 3,
            next_nonce: 0,
            pending: 0,
            total_shares: 3,
            bump: 0,
        };
        m.settle(&c).unwrap();
        assert_eq!(m.pending, tps * 3);
        assert_eq!(m.block_shares, 0);

        // nobody shows up for blocks 2..=4, next tx lands in block 5's window
        c.advance(1_055, &sh).unwrap();
        assert_eq!(c.current_block, 5);
        assert_eq!(c.block_open_ts, 1_055);

        // some shares in block 5, then nothing until well after the end
        c.shares_this_block = 400;
        c.advance(5_000, &sh).unwrap();
        assert_eq!(c.state, STATE_ENDED);
        assert_eq!(c.current_block, 60);
        assert_eq!(c.carry, 0);
        assert_eq!(c.allocated + c.unmined, MINED_SUPPLY);
        assert!(!c.advance(6_000, &sh).unwrap());
    }

    /// A block closed long after its window (nobody cranked) is retargeted on the time it could
    /// accept shares (window + grace), not on the wall-clock gap.
    #[test]
    fn late_close_does_not_ease_difficulty() {
        let sh = [3u8; 32];
        let mut c = coin(30);
        c.shares_this_block = 520; // 400 per 10 s is on pace; 520 over 13 s is on pace too
        c.advance(1_000 + 95, &sh).unwrap(); // closed 85 s late, blocks 1..=8 skipped
        assert_eq!(c.block_targets[0], DEFAULT_START_TARGET);
        // block 0 retarget: target * 400 * 13 / (520 * 10) = target; then 8 empty blocks, 4x each
        assert_eq!(c.block_targets[1], DEFAULT_START_TARGET);
        assert_eq!(c.block_targets[2], DEFAULT_START_TARGET * 4);
        assert_eq!(c.current_block, 9);
        assert_eq!(c.target, (DEFAULT_START_TARGET as u128 * 4u128.pow(8)).min(u64::MAX as u128) as u64);
    }

    #[test]
    fn default_fee_schedule_is_valid() {
        let f = FeeSchedule {
            cliff_fee_numerator: DEFAULT_CLIFF_FEE_NUMERATOR,
            number_of_period: DEFAULT_FEE_PERIODS,
            period_frequency: DEFAULT_FEE_PERIOD_SECS,
            reduction_factor: DEFAULT_FEE_REDUCTION,
        };
        f.validate().unwrap();
        let d = f.to_base_fee_data();
        assert_eq!(d[26], 0);
        assert_eq!(u64::from_le_bytes(d[0..8].try_into().unwrap()), 250_000_000);
        // final fee 1%
        assert_eq!(f.cliff_fee_numerator - 8 * f.reduction_factor, 10_000_000);
        let bad = FeeSchedule { reduction_factor: 40_000_000, ..f };
        assert!(bad.validate().is_err());
    }
}
