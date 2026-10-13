use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, mint_to, set_authority, Mint, MintTo, SetAuthority, Token, TokenAccount},
};

use crate::{constants::*, errors::NonceError, events::Claimed, state::*};

/// Permissionless: mints a miner's earned tokens to the beneficiary's associated token
/// account and closes the miner account (rent back to whoever created it). Only after open,
/// so nobody holds tokens before the pool exists.
#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(mut, seeds = [COIN_SEED, coin.mint.as_ref()], bump = coin.bump, has_one = mint)]
    pub coin: Box<Account<'info, Coin>>,

    #[account(mut)]
    pub mint: Box<Account<'info, Mint>>,

    /// CHECK: bound through the miner PDA seeds.
    pub beneficiary: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [MINER_SEED, coin.key().as_ref(), beneficiary.key().as_ref()],
        bump = miner.bump,
        close = rent_payer
    )]
    pub miner: Box<Account<'info, Miner>>,

    /// CHECK: address checked against the miner account.
    #[account(mut, address = miner.rent_payer)]
    pub rent_payer: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = mint,
        associated_token::authority = beneficiary,
        associated_token::token_program = token_program,
    )]
    pub beneficiary_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_claim(ctx: Context<Claim>) -> Result<()> {
    require!(ctx.accounts.coin.state == STATE_OPENED, NonceError::NotOpen);
    let coin_key = ctx.accounts.coin.key();
    let mint_key = ctx.accounts.mint.key();
    let coin_bump = ctx.accounts.coin.bump;

    {
        let coin = &ctx.accounts.coin;
        let miner = &mut ctx.accounts.miner;
        miner.settle(coin)?;
    }
    let amount = ctx.accounts.miner.pending;

    if amount > 0 {
        let coin_seeds: &[&[u8]] = &[COIN_SEED, mint_key.as_ref(), &[coin_bump]];
        mint_to(
            CpiContext::new_with_signer(
                token::ID,
                MintTo {
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.beneficiary_ata.to_account_info(),
                    authority: ctx.accounts.coin.to_account_info(),
                },
                &[coin_seeds],
            ),
            amount,
        )?;
        let coin = &mut ctx.accounts.coin;
        coin.claimed = coin.claimed.checked_add(amount).ok_or(NonceError::MathOverflow)?;
        require!(coin.claimed <= coin.allocated, NonceError::MathOverflow);
        ctx.accounts.miner.pending = 0;
    }

    emit!(Claimed {
        coin: coin_key,
        beneficiary: ctx.accounts.beneficiary.key(),
        amount,
        total_shares: ctx.accounts.miner.total_shares,
    });
    Ok(())
}

/// Permissionless: once every allocated token has been claimed, remove the mint authority
/// for good. Supply is then fixed on chain.
#[derive(Accounts)]
pub struct Seal<'info> {
    #[account(seeds = [COIN_SEED, coin.mint.as_ref()], bump = coin.bump, has_one = mint)]
    pub coin: Box<Account<'info, Coin>>,

    #[account(mut)]
    pub mint: Box<Account<'info, Mint>>,

    pub token_program: Program<'info, Token>,
}

pub fn handle_seal(ctx: Context<Seal>) -> Result<()> {
    let coin = &ctx.accounts.coin;
    require!(coin.state == STATE_OPENED, NonceError::NotOpen);
    require!(coin.claimed == coin.allocated, NonceError::UnclaimedTokens);
    let mint_key = ctx.accounts.mint.key();
    let coin_seeds: &[&[u8]] = &[COIN_SEED, mint_key.as_ref(), &[coin.bump]];
    set_authority(
        CpiContext::new_with_signer(
            token::ID,
            SetAuthority {
                current_authority: ctx.accounts.coin.to_account_info(),
                account_or_mint: ctx.accounts.mint.to_account_info(),
            },
            &[coin_seeds],
        ),
        token::spl_token::instruction::AuthorityType::MintTokens,
        None,
    )?;
    Ok(())
}
