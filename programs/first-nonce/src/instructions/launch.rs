use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};
use anchor_spl::token::{Mint, Token};

use crate::{
    constants::*,
    errors::NonceError,
    events::CoinLaunched,
    metadata::{create_metadata_v3, CreateMetadataAccounts},
    mining,
    state::*,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct LaunchArgs {
    pub name: String,
    pub symbol: String,
    pub uri: String,
    /// 5, 10 or 15.
    pub dig_minutes: u8,
    /// Fully diluted value at open in lamports. 0 = config default.
    pub open_fdv_lamports: u64,
}

#[derive(Accounts)]
pub struct Launch<'info> {
    #[account(mut)]
    pub launcher: Signer<'info>,

    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    /// Fresh keypair, signs the transaction. Classic SPL mint, 6 decimals, mint authority =
    /// the coin PDA, no freeze authority.
    #[account(
        init,
        payer = launcher,
        mint::decimals = DECIMALS,
        mint::authority = coin,
        mint::token_program = token_program,
    )]
    pub mint: Box<Account<'info, Mint>>,

    #[account(
        init,
        payer = launcher,
        space = 8 + Coin::INIT_SPACE,
        seeds = [COIN_SEED, mint.key().as_ref()],
        bump
    )]
    pub coin: Box<Account<'info, Coin>>,

    /// System-owned PDA. Holds the open reserve, owns the pool-side token accounts and the
    /// locked Meteora position.
    #[account(mut, seeds = [VAULT_SEED, coin.key().as_ref()], bump)]
    pub vault: SystemAccount<'info>,

    #[account(mut, address = config.treasury)]
    pub treasury: SystemAccount<'info>,

    /// CHECK: Metaplex metadata PDA, created by the CPI.
    #[account(
        mut,
        seeds = [b"metadata", TOKEN_METADATA_PROGRAM_ID.as_ref(), mint.key().as_ref()],
        seeds::program = TOKEN_METADATA_PROGRAM_ID,
        bump
    )]
    pub metadata: UncheckedAccount<'info>,

    /// CHECK: address checked.
    #[account(address = TOKEN_METADATA_PROGRAM_ID)]
    pub token_metadata_program: UncheckedAccount<'info>,

    /// CHECK: address checked; read raw for the most recent slot hash.
    #[account(address = SLOT_HASHES_ID)]
    pub slot_hashes: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_launch(ctx: Context<Launch>, args: LaunchArgs) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, NonceError::Paused);
    require!(
        matches!(args.dig_minutes, 5 | 10 | 15),
        NonceError::InvalidDigLength
    );
    require!(
        !args.name.is_empty() && args.name.len() <= MAX_NAME_LEN,
        NonceError::InvalidName
    );
    require!(
        !args.symbol.is_empty() && args.symbol.len() <= MAX_SYMBOL_LEN,
        NonceError::InvalidSymbol
    );
    require!(args.uri.len() <= MAX_URI_LEN, NonceError::InvalidUri);

    let open_fdv = if args.open_fdv_lamports == 0 {
        config.default_open_fdv_lamports
    } else {
        args.open_fdv_lamports
    };
    require!(
        open_fdv >= config.min_open_fdv_lamports && open_fdv <= config.max_open_fdv_lamports,
        NonceError::InvalidOpenFdv
    );
    mining::sqrt_price_from_fdv(open_fdv)?;

    let now = Clock::get()?.unix_timestamp;
    let slot_hash = mining::read_slot_hash(&ctx.accounts.slot_hashes.to_account_info())?;

    // launch fee -> treasury
    if config.launch_fee_lamports > 0 {
        transfer(
            CpiContext::new(
                anchor_lang::system_program::ID,
                Transfer {
                    from: ctx.accounts.launcher.to_account_info(),
                    to: ctx.accounts.treasury.to_account_info(),
                },
            ),
            config.launch_fee_lamports,
        )?;
    }
    // open reserve -> vault (pays Meteora pool rent at open)
    transfer(
        CpiContext::new(
            anchor_lang::system_program::ID,
            Transfer {
                from: ctx.accounts.launcher.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
            },
        ),
        OPEN_RESERVE_LAMPORTS,
    )?;

    // immutable metadata, update authority = coin PDA
    let mint_key = ctx.accounts.mint.key();
    let coin_bump = ctx.bumps.coin;
    let coin_seeds: &[&[u8]] = &[COIN_SEED, mint_key.as_ref(), &[coin_bump]];
    create_metadata_v3(
        &CreateMetadataAccounts {
            metadata: ctx.accounts.metadata.to_account_info(),
            mint: ctx.accounts.mint.to_account_info(),
            mint_authority: ctx.accounts.coin.to_account_info(),
            payer: ctx.accounts.launcher.to_account_info(),
            update_authority: ctx.accounts.coin.to_account_info(),
            system_program: ctx.accounts.system_program.to_account_info(),
        },
        args.name,
        args.symbol,
        args.uri,
        &[coin_seeds],
    )?;

    let total_blocks: u16 = (args.dig_minutes as u16) * 60 / (BLOCK_SECS as u16);
    let start_target = config.start_target;
    let coin = &mut ctx.accounts.coin;
    coin.mint = mint_key;
    coin.launcher = ctx.accounts.launcher.key();
    coin.bump = coin_bump;
    coin.vault_bump = ctx.bumps.vault;
    coin.state = STATE_DIGGING;
    coin.dig_minutes = args.dig_minutes;
    coin.total_blocks = total_blocks;
    coin.launcher_fee_bps = config.launcher_fee_bps;
    coin.share_target = config.share_target;
    coin.open_fdv_lamports = open_fdv;
    coin.fee_schedule = config.fee_schedule;
    coin.launched_at = now;
    coin.dig_start = now;
    coin.dig_end = now + (total_blocks as i64) * BLOCK_SECS;
    coin.current_block = 0;
    coin.block_open_ts = now;
    coin.challenge = mining::challenge(&mint_key, 0, &slot_hash);
    coin.target = start_target;
    coin.shares_this_block = 0;
    coin.carry = 0;
    coin.total_shares = 0;
    coin.allocated = 0;
    coin.claimed = 0;
    coin.unmined = 0;
    coin.tokens_per_share = [0u64; 90];
    coin.block_shares = [0u32; 90];
    coin.block_targets = [0u64; 90];
    coin.miners_total = 0;

    emit!(CoinLaunched {
        coin: coin.key(),
        mint: mint_key,
        launcher: coin.launcher,
        dig_minutes: coin.dig_minutes,
        total_blocks,
        dig_start: coin.dig_start,
        dig_end: coin.dig_end,
        open_fdv_lamports: open_fdv,
        start_target,
    });

    let config = &mut ctx.accounts.config;
    config.coins_launched = config.coins_launched.saturating_add(1);
    Ok(())
}
