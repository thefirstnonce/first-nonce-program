use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, burn, mint_to, sync_native, Burn, Mint, MintTo, SyncNative, Token, TokenAccount},
};

use crate::{
    constants::*,
    cp_amm::{self, *},
    errors::NonceError,
    events::{BlockAdvanced, CoinOpened},
    mining,
    state::*,
};

/// Permissionless once the last block's window has passed. Mints the 80% pool supply to the
/// vault PDA and creates a single-sided Meteora DAMM v2 customizable pool (coin / wSOL) whose
/// only position is owned by the vault PDA and permanently locked.
#[derive(Accounts)]
pub struct Open<'info> {
    /// Pays rent for the vault's two associated token accounts and 1 lamport of wSOL.
    #[account(mut)]
    pub cranker: Signer<'info>,

    #[account(mut, seeds = [COIN_SEED, coin.mint.as_ref()], bump = coin.bump, has_one = mint)]
    pub coin: Box<Account<'info, Coin>>,

    #[account(mut)]
    pub mint: Box<Account<'info, Mint>>,

    #[account(mut, seeds = [VAULT_SEED, coin.key().as_ref()], bump = coin.vault_bump)]
    pub vault: SystemAccount<'info>,

    #[account(
        init_if_needed,
        payer = cranker,
        associated_token::mint = mint,
        associated_token::authority = vault,
        associated_token::token_program = token_program,
    )]
    pub vault_token_a: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = cranker,
        associated_token::mint = native_mint,
        associated_token::authority = vault,
        associated_token::token_program = token_program,
    )]
    pub vault_wsol: Box<Account<'info, TokenAccount>>,

    #[account(address = NATIVE_MINT)]
    pub native_mint: Box<Account<'info, Mint>>,

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

    /// CHECK: address checked.
    #[account(address = TOKEN_2022_PROGRAM_ID)]
    pub token_2022_program: UncheckedAccount<'info>,

    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_open(ctx: Context<Open>) -> Result<()> {
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

    // 1. mint the 80% pool supply to the vault
    mint_to(
        CpiContext::new_with_signer(
            token::ID,
            MintTo {
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.vault_token_a.to_account_info(),
                authority: ctx.accounts.coin.to_account_info(),
            },
            &[coin_seeds],
        ),
        POOL_SUPPLY,
    )?;

    // 2. cp-amm always transfers max(amount, 1) of each token from the payer ("prove
    //    ownership of token mints"), so the vault needs 1 lamport of wSOL.
    if ctx.accounts.vault_wsol.amount == 0 {
        transfer(
            CpiContext::new(
                anchor_lang::system_program::ID,
                Transfer {
                    from: ctx.accounts.cranker.to_account_info(),
                    to: ctx.accounts.vault_wsol.to_account_info(),
                },
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

    let vault_info = ctx.accounts.vault.to_account_info();
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
            token_a_program: ctx.accounts.token_program.to_account_info(),
            token_b_program: ctx.accounts.token_program.to_account_info(),
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
    ctx.accounts.vault_token_a.reload()?;
    let leftover = ctx.accounts.vault_token_a.amount;
    if leftover > 0 {
        burn(
            CpiContext::new_with_signer(
                token::ID,
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
