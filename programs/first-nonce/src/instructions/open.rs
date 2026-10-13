use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};
use anchor_spl::{
    associated_token::{create_idempotent, AssociatedToken, Create},
    token::{self, close_account, sync_native, CloseAccount, Mint as SplMint, SyncNative, Token},
    token_interface::{self, Burn, Mint, MintTo},
};

use crate::{
    constants::*,
    cp_amm::{self, *},
    errors::NonceError,
    events::{BlockAdvanced, CoinOpened, ReserveRefunded},
    funds, mining,
    state::*,
};

/// Token amount of an SPL / Token-2022 token account (same base layout).
pub(crate) fn token_amount(info: &AccountInfo) -> Result<u64> {
    let data = info.try_borrow_data()?;
    require!(data.len() >= 72, NonceError::InvalidTokenAccount);
    let mut b = [0u8; 8];
    b.copy_from_slice(&data[64..72]);
    Ok(u64::from_le_bytes(b))
}

/// Permissionless once the last block's window has passed. Mints the 80% pool supply to the
/// vault PDA and creates a single-sided Meteora DAMM v2 customizable pool (coin / wSOL) whose
/// only position is owned by the vault PDA and permanently locked.
///
/// Everything is paid from the open reserve in the vault, so the cranker pays only the
/// transaction fee. Optional remaining account: the coin's launcher (writable). If it is
/// passed, the unused reserve goes back to the launcher here; otherwise at the next `collect`.
#[derive(Accounts)]
pub struct Open<'info> {
    /// Pays only the transaction fee (writable for compatibility with existing clients).
    #[account(mut)]
    pub cranker: Signer<'info>,

    #[account(mut, seeds = [COIN_SEED, coin.mint.as_ref()], bump = coin.bump, has_one = mint)]
    pub coin: Box<Account<'info, Coin>>,

    /// Classic SPL or Token-2022 mint (`coin.token_2022`).
    #[account(mut)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(mut, seeds = [VAULT_SEED, coin.key().as_ref()], bump = coin.vault_bump)]
    pub vault: SystemAccount<'info>,

    /// CHECK: the vault's coin ATA (under the coin's token program). Created here, paid by the
    /// vault; the ATA program checks the address.
    #[account(mut)]
    pub vault_token_a: UncheckedAccount<'info>,

    /// CHECK: the vault's wSOL ATA. Created here, paid by the vault, closed again at the end;
    /// the ATA program checks the address.
    #[account(mut)]
    pub vault_wsol: UncheckedAccount<'info>,

    #[account(address = NATIVE_MINT)]
    pub native_mint: Box<Account<'info, SplMint>>,

    /// CHECK: our PDA; cp-amm creates it as the token-2022 position NFT mint (we sign for it).
    #[account(mut, seeds = [POSITION_NFT_SEED, coin.key().as_ref()], bump)]
    pub position_nft_mint: UncheckedAccount<'info>,

    /// CHECK: cp-amm PDA ["position_nft_account", position_nft_mint], checked by cp-amm.
    #[account(mut)]
    pub position_nft_account: UncheckedAccount<'info>,

    /// CHECK: cp-amm PDA ["cpool", max(mint, wsol), min(mint, wsol)], checked by cp-amm.
    #[account(mut)]
    pub pool: UncheckedAccount<'info>,

    /// CHECK: cp-amm PDA ["position", position_nft_mint], checked by cp-amm.
    #[account(mut)]
    pub position: UncheckedAccount<'info>,

    /// CHECK: cp-amm PDA ["token_vault", mint, pool], checked by cp-amm.
    #[account(mut)]
    pub token_a_vault: UncheckedAccount<'info>,

    /// CHECK: cp-amm PDA ["token_vault", wsol, pool], checked by cp-amm.
    #[account(mut)]
    pub token_b_vault: UncheckedAccount<'info>,

    /// CHECK: address checked.
    #[account(address = cp_amm::POOL_AUTHORITY)]
    pub pool_authority: UncheckedAccount<'info>,

    /// CHECK: address checked.
    #[account(address = cp_amm::EVENT_AUTHORITY)]
    pub event_authority: UncheckedAccount<'info>,

    /// CHECK: address checked.
    #[account(address = cp_amm::CP_AMM_PROGRAM_ID)]
    pub cp_amm_program: UncheckedAccount<'info>,

    /// CHECK: address checked; read raw.
    #[account(address = SLOT_HASHES_ID)]
    pub slot_hashes: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,

    /// CHECK: address checked. The position NFT program, and the coin's token program for
    /// Token-2022 coins.
    #[account(address = TOKEN_2022_PROGRAM_ID)]
    pub token_2022_program: UncheckedAccount<'info>,

    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_open<'info>(ctx: Context<'info, Open<'info>>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let slot_hash = mining::read_slot_hash(&ctx.accounts.slot_hashes.to_account_info())?;
    let coin_key = ctx.accounts.coin.key();

    // close any blocks still open, then require the dig to be over
    {
        let coin = &mut ctx.accounts.coin;
        if coin.advance(now, &slot_hash)? {
            emit!(BlockAdvanced {
                coin: coin_key,
                current_block: coin.current_block,
                target: coin.target,
                challenge: coin.challenge,
                ended: coin.state != STATE_DIGGING,
            });
        }
        require!(coin.state == STATE_ENDED, NonceError::DigNotOver);
    }

    let mint_key = ctx.accounts.mint.key();
    let coin_bump = ctx.accounts.coin.bump;
    let vault_bump = ctx.accounts.coin.vault_bump;
    let nft_bump = ctx.bumps.position_nft_mint;
    let coin_seeds: &[&[u8]] = &[COIN_SEED, mint_key.as_ref(), &[coin_bump]];
    let vault_seeds: &[&[u8]] = &[VAULT_SEED, coin_key.as_ref(), &[vault_bump]];
    let nft_seeds: &[&[u8]] = &[POSITION_NFT_SEED, coin_key.as_ref(), &[nft_bump]];

    // the coin's token program
    let token_a_program = if ctx.accounts.coin.token_2022 {
        ctx.accounts.token_2022_program.to_account_info()
    } else {
        ctx.accounts.token_program.to_account_info()
    };
    require_keys_eq!(
        *ctx.accounts.mint.to_account_info().owner,
        token_a_program.key(),
        NonceError::InvalidTokenProgram
    );
    let classic_program = ctx.accounts.token_program.to_account_info();
    let vault_info = ctx.accounts.vault.to_account_info();

    // 1. the vault's two token accounts, paid from the reserve
    for (ata, mint, program) in [
        (
            ctx.accounts.vault_token_a.to_account_info(),
            ctx.accounts.mint.to_account_info(),
            token_a_program.clone(),
        ),
        (
            ctx.accounts.vault_wsol.to_account_info(),
            ctx.accounts.native_mint.to_account_info(),
            classic_program.clone(),
        ),
    ] {
        create_idempotent(CpiContext::new_with_signer(
            anchor_spl::associated_token::ID,
            Create {
                payer: vault_info.clone(),
                associated_token: ata,
                authority: vault_info.clone(),
                mint,
                system_program: ctx.accounts.system_program.to_account_info(),
                token_program: program,
            },
            &[vault_seeds],
        ))?;
    }

    // 2. mint the 80% pool supply to the vault
    token_interface::mint_to(
        CpiContext::new_with_signer(
            token_a_program.key(),
            MintTo {
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.vault_token_a.to_account_info(),
                authority: ctx.accounts.coin.to_account_info(),
            },
            &[coin_seeds],
        ),
        POOL_SUPPLY,
    )?;

    // cp-amm always transfers max(amount, 1) of each token from the payer ("prove ownership of
    // token mints"), so the vault needs 1 lamport of wSOL.
    if token_amount(&ctx.accounts.vault_wsol.to_account_info())? == 0 {
        transfer(
            CpiContext::new_with_signer(
                anchor_lang::system_program::ID,
                Transfer {
                    from: vault_info.clone(),
                    to: ctx.accounts.vault_wsol.to_account_info(),
                },
                &[vault_seeds],
            ),
            1,
        )?;
        sync_native(CpiContext::new(
            token::ID,
            SyncNative {
                account: ctx.accounts.vault_wsol.to_account_info(),
            },
        ))?;
    }

    // 3. single-sided pool: initial price == min price, so only token A is deposited
    let sqrt_price = mining::sqrt_price_from_fdv(ctx.accounts.coin.open_fdv_lamports)?;
    let liquidity = mining::single_sided_liquidity(POOL_SUPPLY, sqrt_price)?;
    let fee = ctx.accounts.coin.fee_schedule;
    let params = InitializeCustomizablePoolParameters {
        pool_fees: PoolFeeParameters {
            base_fee: BaseFeeParameters {
                data: fee.to_base_fee_data(),
            },
            compounding_fee_bps: 0,
            padding: 0,
            dynamic_fee: None,
        },
        sqrt_min_price: sqrt_price,
        sqrt_max_price: MAX_SQRT_PRICE,
        has_alpha_vault: false,
        liquidity,
        sqrt_price,
        activation_type: ACTIVATION_TYPE_TIMESTAMP,
        collect_fee_mode: COLLECT_FEE_MODE_ONLY_B,
        activation_point: None, // trading starts now
    };

    cp_amm::initialize_customizable_pool(
        &InitializeCustomizablePoolAccounts {
            creator: vault_info.clone(),
            position_nft_mint: ctx.accounts.position_nft_mint.to_account_info(),
            position_nft_account: ctx.accounts.position_nft_account.to_account_info(),
            payer: vault_info.clone(),
            pool_authority: ctx.accounts.pool_authority.to_account_info(),
            pool: ctx.accounts.pool.to_account_info(),
            position: ctx.accounts.position.to_account_info(),
            token_a_mint: ctx.accounts.mint.to_account_info(),
            token_b_mint: ctx.accounts.native_mint.to_account_info(),
            token_a_vault: ctx.accounts.token_a_vault.to_account_info(),
            token_b_vault: ctx.accounts.token_b_vault.to_account_info(),
            payer_token_a: ctx.accounts.vault_token_a.to_account_info(),
            payer_token_b: ctx.accounts.vault_wsol.to_account_info(),
            token_a_program: token_a_program.clone(),
            token_b_program: classic_program.clone(),
            token_2022_program: ctx.accounts.token_2022_program.to_account_info(),
            system_program: ctx.accounts.system_program.to_account_info(),
            event_authority: ctx.accounts.event_authority.to_account_info(),
            program: ctx.accounts.cp_amm_program.to_account_info(),
        },
        &params,
        &[vault_seeds, nft_seeds],
    )?;

    // 4. lock all of it, forever
    cp_amm::permanent_lock_position(
        &PermanentLockPositionAccounts {
            pool: ctx.accounts.pool.to_account_info(),
            position: ctx.accounts.position.to_account_info(),
            position_nft_account: ctx.accounts.position_nft_account.to_account_info(),
            signer: vault_info.clone(),
            event_authority: ctx.accounts.event_authority.to_account_info(),
            program: ctx.accounts.cp_amm_program.to_account_info(),
        },
        liquidity,
        &[vault_seeds],
    )?;

    // 5. burn the rounding remainder that did not go into the pool
    let leftover = token_amount(&ctx.accounts.vault_token_a.to_account_info())?;
    if leftover > 0 {
        token_interface::burn(
            CpiContext::new_with_signer(
                token_a_program.key(),
                Burn {
                    mint: ctx.accounts.mint.to_account_info(),
                    from: ctx.accounts.vault_token_a.to_account_info(),
                    authority: vault_info.clone(),
                },
                &[vault_seeds],
            ),
            leftover,
        )?;
    }

    // 6. close the (now empty) wSOL account: its rent goes back into the vault
    close_account(CpiContext::new_with_signer(
        token::ID,
        CloseAccount {
            account: ctx.accounts.vault_wsol.to_account_info(),
            destination: vault_info.clone(),
            authority: vault_info.clone(),
        },
        &[vault_seeds],
    ))?;

    // 7. the unused reserve goes back to the launcher, if the caller passed the launcher
    let launcher = ctx.accounts.coin.launcher;
    if let Some(launcher_info) = ctx.remaining_accounts.first() {
        require_keys_eq!(launcher_info.key(), launcher, NonceError::InvalidLauncher);
        let refund = funds::pay_launcher(&vault_info, launcher_info, vault_seeds, 0)?;
        if refund > 0 {
            let coin = &mut ctx.accounts.coin;
            coin.reserve_refunded = coin.reserve_refunded.saturating_add(refund);
            emit!(ReserveRefunded {
                coin: coin_key,
                launcher,
                amount: refund,
            });
        }
    }

    let coin = &mut ctx.accounts.coin;
    coin.state = STATE_OPENED;
    coin.opened_at = now;
    coin.pool = ctx.accounts.pool.key();
    coin.position = ctx.accounts.position.key();
    coin.position_nft_mint = ctx.accounts.position_nft_mint.key();
    coin.pool_liquidity = liquidity;
    coin.pool_sqrt_price = sqrt_price;
    coin.pool_tokens = POOL_SUPPLY - leftover;

    emit!(CoinOpened {
        coin: coin_key,
        pool: coin.pool,
        position: coin.position,
        sqrt_price,
        liquidity,
        pool_tokens: coin.pool_tokens,
        burned: leftover,
        allocated: coin.allocated,
        unmined: coin.unmined,
    });
    Ok(())
}
