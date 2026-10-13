//! The open reserve: how much SOL the launcher parks in the vault, and what goes back.

use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

use crate::{constants::*, errors::NonceError};

/// Lamports the vault keeps after `open`: the rent of the temporary wSOL account `collect`
/// creates and closes in the same instruction. Also above the vault's own rent-exempt minimum.
pub fn vault_keep(rent: &Rent) -> u64 {
    rent.minimum_balance(TOKEN_ACCOUNT_LEN)
}

/// Everything `open` pays for from the vault, at the given rent, plus the margin:
/// - cp-amm's pool, position, position NFT mint and NFT account, and its two token vaults;
/// - the vault's own coin token account (kept: `collect` passes it to cp-amm);
/// - the vault's wSOL token account (closed again inside `open`, so its rent comes back);
/// - 1 lamport of wSOL that cp-amm takes as "proof of ownership";
/// - `vault_keep` for `collect`.
pub fn open_reserve(rent: &Rent, token_2022: bool) -> u64 {
    let r = |len: usize| rent.minimum_balance(len);
    let coin_ata = if token_2022 { TOKEN_2022_ATA_LEN } else { TOKEN_ACCOUNT_LEN };
    r(CP_POOL_LEN)
        + r(CP_POSITION_LEN)
        + r(CP_POSITION_NFT_MINT_LEN)
        + r(TOKEN_ACCOUNT_LEN) // position NFT account
        + r(TOKEN_ACCOUNT_LEN) // cp-amm token A vault
        + r(TOKEN_ACCOUNT_LEN) // cp-amm token B (wSOL) vault
        + r(coin_ata) // vault's coin token account
        + r(TOKEN_ACCOUNT_LEN) // vault's wSOL token account (refunded in open)
        + 1
        + vault_keep(rent)
        + OPEN_RESERVE_MARGIN_LAMPORTS
}

/// Send everything in the vault above `vault_keep` to the launcher. Returns the amount sent.
/// `extra` is added to the same transfer (the launcher's fee share in `collect`).
pub fn pay_launcher<'info>(
    vault: &AccountInfo<'info>,
    launcher: &AccountInfo<'info>,
    vault_seeds: &[&[u8]],
    extra: u64,
) -> Result<u64> {
    let keep = vault_keep(&Rent::get()?);
    let lamports = vault.lamports();
    // `extra` is already in the vault; the refund is whatever is left above it and the keep.
    let refund = lamports.saturating_sub(extra).saturating_sub(keep);
    let total = extra.checked_add(refund).ok_or(NonceError::MathOverflow)?;
    if total > 0 {
        transfer(
            CpiContext::new_with_signer(
                anchor_lang::system_program::ID,
                Transfer {
                    from: vault.clone(),
                    to: launcher.clone(),
                },
                &[vault_seeds],
            ),
            total,
        )?;
    }
    Ok(refund)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mainnet rent since SIMD-0437-2: 5,080 lamports per byte (incl. the 128-byte overhead).
    fn mainnet_rent() -> Rent {
        Rent::with_lamports_per_byte(5_080)
    }

    #[test]
    fn reserve_at_mainnet_rent() {
        let rent = mainnet_rent();
        assert_eq!(rent.minimum_balance(165), 1_488_440);
        // cp-amm accounts measured on the mainnet canary: 16,499,840 lamports
        let cp_amm = rent.minimum_balance(CP_POOL_LEN)
            + rent.minimum_balance(CP_POSITION_LEN)
            + rent.minimum_balance(CP_POSITION_NFT_MINT_LEN)
            + 3 * rent.minimum_balance(TOKEN_ACCOUNT_LEN);
        assert_eq!(cp_amm, 16_499_840);
        // classic: cp-amm + 2 vault ATAs + 1 + keep + margin
        assert_eq!(
            open_reserve(&rent, false),
            16_499_840 + 2 * 1_488_440 + 1 + 1_488_440 + OPEN_RESERVE_MARGIN_LAMPORTS
        );
        // Token-2022: the vault's coin ATA is 170 bytes
        assert_eq!(open_reserve(&rent, true), open_reserve(&rent, false) + 5 * 5_080);
        // far below the old flat 0.035 SOL
        assert!(open_reserve(&rent, true) < LEGACY_OPEN_RESERVE_LAMPORTS);
        assert_eq!(vault_keep(&rent), 1_488_440);
    }
}
