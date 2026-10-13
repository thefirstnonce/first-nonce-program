//! Pure mining and pricing math. Everything here must match `program/sdk` byte for byte.

use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use crate::{constants::*, cp_amm, errors::NonceError};

/// challenge = keccak256(mint (32) || block_number as u64 LE (8) || slot_hash (32))
pub fn challenge(mint: &Pubkey, block: u32, slot_hash: &[u8; 32]) -> [u8; 32] {
    let block_le = (block as u64).to_le_bytes();
    hashv(&[mint.as_ref(), &block_le, slot_hash]).to_bytes()
}

/// share = keccak256(challenge (32) || beneficiary (32) || nonce u64 LE (8))
pub fn share_hash(challenge: &[u8; 32], beneficiary: &Pubkey, nonce: u64) -> [u8; 32] {
    let nonce_le = nonce.to_le_bytes();
    hashv(&[challenge, beneficiary.as_ref(), &nonce_le]).to_bytes()
}

/// First 8 bytes of the share hash read as a big-endian u64.
pub fn share_value(share: &[u8; 32]) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&share[0..8]);
    u64::from_be_bytes(b)
}

pub fn is_valid_share(challenge: &[u8; 32], beneficiary: &Pubkey, nonce: u64, target: u64) -> bool {
    share_value(&share_hash(challenge, beneficiary, nonce)) < target
}

/// Keep shares per block near `share_target`:
/// next = target * S * duration / (shares * BLOCK_SECS), clamped to [target/4, target*4],
/// where `duration` is how long the block actually accepted shares. Zero shares -> 4x easier.
pub fn retarget(target: u64, shares: u64, duration_secs: i64, share_target: u32) -> u64 {
    let t = target.max(1) as u128;
    let easiest = t.saturating_mul(RETARGET_CLAMP).min(u64::MAX as u128);
    let hardest = (t / RETARGET_CLAMP).max(1);
    let next = if shares == 0 {
        easiest
    } else {
        let duration = duration_secs.max(1) as u128;
        let num = t
            .saturating_mul(share_target as u128)
            .saturating_mul(duration);
        let den = (shares as u128).saturating_mul(BLOCK_SECS as u128);
        num / den
    };
    next.clamp(hardest, easiest) as u64
}

/// Flat reward: MINED_SUPPLY / total_blocks per block, remainder added to the last block.
pub fn reward_for_block(block: u32, total_blocks: u16) -> u64 {
    let total = total_blocks.max(1) as u64;
    let base = MINED_SUPPLY / total;
    if block as u64 == total - 1 {
        base + MINED_SUPPLY % total
    } else {
        base
    }
}

/// Integer square root (floor) of a u128.
pub fn isqrt_u128(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let bits = 128 - n.leading_zeros();
    let mut x: u128 = 1u128 << ((bits + 1) / 2); // x >= sqrt(n)
    loop {
        let y = (x + n / x) >> 1;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// Meteora sqrt price (Q64.64 of sqrt(lamports per coin base unit)) for a fully diluted
/// opening value of `fdv_lamports` across TOTAL_SUPPLY.
/// sqrt_price = floor(sqrt(floor(fdv * 2^64 / TOTAL_SUPPLY) * 2^64))
pub fn sqrt_price_from_fdv(fdv_lamports: u64) -> Result<u128> {
    require!(
        fdv_lamports > 0 && fdv_lamports <= HARD_MAX_OPEN_FDV_LAMPORTS,
        NonceError::InvalidOpenFdv
    );
    let price_q64 = ((fdv_lamports as u128) << 64) / (TOTAL_SUPPLY as u128);
    require!(price_q64 > 0 && price_q64 < (1u128 << 64), NonceError::InvalidOpenFdv);
    let s = isqrt_u128(price_q64 << 64);
    require!(
        s >= cp_amm::MIN_SQRT_PRICE && s < cp_amm::MAX_SQRT_PRICE,
        NonceError::InvalidSqrtPrice
    );
    Ok(s)
}

/// Liquidity for a token-A-only deposit of `amount_a` on [sqrt_price, MAX_SQRT_PRICE].
/// Exact value is amount_a * s * smax / (smax - s); we use amount_a * s, which is always
/// a little smaller, so the deposit Meteora computes (rounded up) never exceeds amount_a.
/// The difference (about 0.001 token at the default price) is burned.
pub fn single_sided_liquidity(amount_a: u64, sqrt_price: u128) -> Result<u128> {
    (amount_a as u128)
        .checked_mul(sqrt_price)
        .ok_or_else(|| error!(NonceError::MathOverflow))
}

/// Most recent slot hash from the SlotHashes sysvar account data:
/// [u64 entry count][ (u64 slot, [u8;32] hash) ... ] newest first.
pub fn read_slot_hash(slot_hashes: &AccountInfo) -> Result<[u8; 32]> {
    require_keys_eq!(*slot_hashes.key, SLOT_HASHES_ID, NonceError::InvalidSlotHashes);
    let data = slot_hashes.try_borrow_data()?;
    require!(data.len() >= 48, NonceError::InvalidSlotHashes);
    let mut len = [0u8; 8];
    len.copy_from_slice(&data[0..8]);
    require!(u64::from_le_bytes(len) > 0, NonceError::InvalidSlotHashes);
    let mut h = [0u8; 32];
    h.copy_from_slice(&data[16..48]);
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_matches() {
        for n in [0u128, 1, 2, 3, 4, 15, 16, 17, 1 << 64, (1 << 100) + 12345, u128::MAX] {
            let r = isqrt_u128(n);
            assert!(r.checked_mul(r).map_or(false, |sq| sq <= n));
            let r1 = r + 1;
            assert!(r1.checked_mul(r1).map_or(true, |sq| sq > n));
        }
    }

    #[test]
    fn retarget_clamps() {
        let t = 1u64 << 40;
        assert_eq!(retarget(t, 0, 10, 400), t * 4);
        assert_eq!(retarget(t, 400, 10, 400), t);
        assert_eq!(retarget(t, 4_000_000, 10, 400), t / 4);
        assert_eq!(retarget(t, 800, 10, 400), t / 2);
        assert_eq!(retarget(t, 200, 5, 400), t); // 200 shares in 5 s is on pace
        assert_eq!(retarget(u64::MAX, 0, 10, 400), u64::MAX);
        assert_eq!(retarget(1, 1_000_000, 10, 400), 1);
    }

    #[test]
    fn rewards_sum_to_mined_supply() {
        for total in [30u16, 60, 90] {
            let sum: u64 = (0..total as u32).map(|b| reward_for_block(b, total)).sum();
            assert_eq!(sum, MINED_SUPPLY);
        }
    }

    #[test]
    fn default_price_is_in_range() {
        let s = sqrt_price_from_fdv(DEFAULT_OPEN_FDV_LAMPORTS).unwrap();
        // price = 30e9 / 1e15 = 3e-5 lamports per base unit; sqrt * 2^64 ~= 1.0103e17
        assert!(s > 101_000_000_000_000_000 && s < 101_100_000_000_000_000);
        let l = single_sided_liquidity(POOL_SUPPLY, s).unwrap();
        assert!(l > 0);
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// The program's hashing must match program/sdk/test-vectors.json (generated with js-sha3,
    /// cross-checked with an independent Python Keccak).
    #[test]
    fn matches_sdk_test_vectors() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../../sdk/test-vectors.json")).unwrap();
        let mut n = 0;
        for c in v["challenges"].as_array().unwrap() {
            let mint = Pubkey::try_from(unhex(c["mintHex"].as_str().unwrap()).as_slice()).unwrap();
            let slot: [u8; 32] = unhex(c["slotHash"].as_str().unwrap()).try_into().unwrap();
            let block = c["block"].as_u64().unwrap() as u32;
            let got = challenge(&mint, block, &slot);
            assert_eq!(got.to_vec(), unhex(c["challenge"].as_str().unwrap()));
            n += 1;
        }
        for s in v["shares"].as_array().unwrap() {
            let ch: [u8; 32] = unhex(s["challenge"].as_str().unwrap()).try_into().unwrap();
            let bene = Pubkey::try_from(unhex(s["beneficiaryHex"].as_str().unwrap()).as_slice()).unwrap();
            let nonce: u64 = s["nonce"].as_str().unwrap().parse().unwrap();
            let h = share_hash(&ch, &bene, nonce);
            assert_eq!(h.to_vec(), unhex(s["hash"].as_str().unwrap()));
            assert_eq!(share_value(&h).to_string(), s["value"].as_str().unwrap());
            n += 1;
        }
        let f = &v["foundAtStartTarget"];
        let ch: [u8; 32] = unhex(f["challenge"].as_str().unwrap()).try_into().unwrap();
        let bene: Pubkey = f["beneficiary"].as_str().unwrap().parse().unwrap();
        let nonce: u64 = f["nonce"].as_str().unwrap().parse().unwrap();
        assert!(is_valid_share(&ch, &bene, nonce, DEFAULT_START_TARGET));
        assert!(n > 70);
    }
}
