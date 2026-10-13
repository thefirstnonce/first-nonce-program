use anchor_lang::prelude::*;

use crate::{
    constants::*,
    errors::NonceError,
    events::{BlockAdvanced, SharesSubmitted},
    mining,
    state::*,
};

#[derive(Accounts)]
pub struct SubmitShares<'info> {
    /// Pays the transaction and, on the first submission, the miner account rent (refunded
    /// at claim). Does not have to be the beneficiary: shares can only ever credit the
    /// beneficiary whose key is inside the hash.
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(mut, seeds = [COIN_SEED, coin.mint.as_ref()], bump = coin.bump)]
    pub coin: Box<Account<'info, Coin>>,

    /// CHECK: any pubkey; it is hashed into every share.
    pub beneficiary: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + Miner::INIT_SPACE,
        seeds = [MINER_SEED, coin.key().as_ref(), beneficiary.key().as_ref()],
        bump
    )]
    pub miner: Box<Account<'info, Miner>>,

    /// CHECK: address checked; read raw.
    #[account(address = SLOT_HASHES_ID)]
    pub slot_hashes: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handle_submit_shares(ctx: Context<SubmitShares>, block: u32, nonces: Vec<u64>) -> Result<()> {
    require!(!nonces.is_empty(), NonceError::NoNonces);
    require!(nonces.len() <= MAX_NONCES_PER_TX, NonceError::TooManyNonces);

    let now = Clock::get()?.unix_timestamp;
    let slot_hash = mining::read_slot_hash(&ctx.accounts.slot_hashes.to_account_info())?;

    let coin_key = ctx.accounts.coin.key();
    let beneficiary = ctx.accounts.beneficiary.key();
    let coin = &mut ctx.accounts.coin;
    let miner = &mut ctx.accounts.miner;

    // Shares count for the open block until it is closed, with a short grace after its window.
    // Anything else fails, so a miner never pays for a submission that credits nothing (and a
    // client's preflight catches it before it is sent). Closing a block whose window and grace
    // are over is `crank`'s job (or the next live submission's).
    require!(block <= coin.current_block || coin.state != STATE_DIGGING, NonceError::BlockNotOpen);
    require!(
        coin.state == STATE_DIGGING
            && block == coin.current_block
            && now < coin.window_end(block) + LATE_SHARE_GRACE_SECS,
        NonceError::BlockClosed
    );

    if miner.coin == Pubkey::default() {
        miner.coin = coin_key;
        miner.beneficiary = beneficiary;
        miner.rent_payer = ctx.accounts.payer.key();
        miner.block = NO_BLOCK;
        miner.block_shares = 0;
        miner.next_nonce = 0;
        miner.pending = 0;
        miner.total_shares = 0;
        miner.bump = ctx.bumps.miner;
        coin.miners_total = coin.miners_total.saturating_add(1);
    }

    miner.settle(coin)?;
    if miner.block != block {
        miner.block = block;
        miner.block_shares = 0;
        miner.next_nonce = 0;
    }
    let challenge = coin.challenge;
    let target = coin.target;
    let mut next_nonce = miner.next_nonce;
    for &nonce in nonces.iter() {
        require!(nonce >= next_nonce, NonceError::NonceNotIncreasing);
        require!(
            mining::is_valid_share(&challenge, &beneficiary, nonce, target),
            NonceError::ShareAboveTarget
        );
        next_nonce = nonce.checked_add(1).ok_or(NonceError::MathOverflow)?;
    }
    let n = nonces.len() as u64;
    miner.next_nonce = next_nonce;
    miner.block_shares = miner.block_shares.checked_add(n).ok_or(NonceError::MathOverflow)?;
    miner.total_shares = miner.total_shares.checked_add(n).ok_or(NonceError::MathOverflow)?;
    coin.shares_this_block = coin.shares_this_block.checked_add(n).ok_or(NonceError::MathOverflow)?;
    coin.total_shares = coin.total_shares.checked_add(n).ok_or(NonceError::MathOverflow)?;

    emit!(SharesSubmitted {
        coin: coin_key,
        beneficiary,
        block,
        accepted: n as u8,
        block_shares: coin.shares_this_block,
        nonces,
        challenge,
        target,
    });

    // A late share (inside the grace) also closes the block it was counted in.
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
