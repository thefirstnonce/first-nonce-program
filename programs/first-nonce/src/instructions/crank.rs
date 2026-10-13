use anchor_lang::prelude::*;

use crate::{constants::*, events::BlockAdvanced, mining, state::*};

/// Permissionless: closes every block whose window has passed and opens the next one.
/// Miners also do this implicitly with every submission.
#[derive(Accounts)]
pub struct Crank<'info> {
    #[account(mut, seeds = [COIN_SEED, coin.mint.as_ref()], bump = coin.bump)]
    pub coin: Box<Account<'info, Coin>>,

    /// CHECK: address checked; read raw.
    #[account(address = SLOT_HASHES_ID)]
    pub slot_hashes: UncheckedAccount<'info>,
}

pub fn handle_crank(ctx: Context<Crank>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let slot_hash = mining::read_slot_hash(&ctx.accounts.slot_hashes.to_account_info())?;
    let coin_key = ctx.accounts.coin.key();
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
    Ok(())
}
