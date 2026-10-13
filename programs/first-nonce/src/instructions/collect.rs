use anchor_lang::prelude::*;
use anchor_lang::system_program::{
    allocate, assign, create_account, transfer, Allocate, Assign, CreateAccount, Transfer,
};
use anchor_spl::token::{
    self, close_account, initialize_account3, CloseAccount, InitializeAccount3, Mint as SplMint,
    Token,
};
use anchor_spl::token_interface::Mint;

use crate::{
    constants::*,
    cp_amm::{self, ClaimPositionFeeAccounts},
    errors::NonceError,
    events::{FeesCollected, ReserveRefunded},
    funds,
    instructions::open::token_amount,
    state::*,
};

/// Permissionless crank. Claims the locked position's trading fees (SOL only, the pool collects
/// fees in token B), unwraps them and pays `launcher_fee_bps` to the launcher and the rest to
/// the platform treasury.
///
/// Flow: create a temporary wSOL account at PDA ["fee", coin] (rent from the vault) -> claim
/// into it -> close it into the vault (unwraps fees + rent) -> pay out the fee amount. Any
/// unused open reserve still in the vault goes to the launcher with its share.
///
/// Remaining account for Token-2022 coins: the Token-2022 program (cp-amm needs the coin's
/// token program).
#[derive(Accounts)]
pub struct Collect<'info> {
    pub cranker: Signer<'info>,

    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,

    #[account(
        mut,
        seeds = [COIN_SEED, coin.mint.as_ref()],
        bump = coin.bump,
        has_one = mint,
        has_one = launcher,
        has_one = pool,
        has_one = position,
    )]
    pub coin: Box<Account<'info, Coin>>,

    /// Classic SPL or Token-2022 mint (`coin.token_2022`).
    pub mint: Box<InterfaceAccount<'info, Mint>>,

    #[account(mut, seeds = [VAULT_SEED, coin.key().as_ref()], bump = coin.vault_bump)]
    pub vault: SystemAccount<'info>,

    /// CHECK: a coin token account owned by the vault (its ATA); checked in the handler. It
    /// receives the token A side of the fee claim, which is always 0 (fees are SOL only).
    #[account(mut)]
    pub vault_token_a: UncheckedAccount<'info>,

    /// CHECK: temporary wSOL account, created and closed inside this instruction.
    #[account(mut, seeds = [FEE_SEED, coin.key().as_ref()], bump)]
    pub fee_account: UncheckedAccount<'info>,

    #[account(address = NATIVE_MINT)]
    pub native_mint: Box<Account<'info, SplMint>>,

    /// CHECK: has_one on coin.
    #[account(mut)]
    pub launcher: UncheckedAccount<'info>,

    /// CHECK: address checked against config.
    #[account(mut, address = config.treasury)]
    pub treasury: UncheckedAccount<'info>,

    /// CHECK: address checked.
    #[account(address = cp_amm::POOL_AUTHORITY)]
    pub pool_authority: UncheckedAccount<'info>,

    /// CHECK: has_one on coin.
    pub pool: UncheckedAccount<'info>,

    /// CHECK: has_one on coin.
    #[account(mut)]
    pub position: UncheckedAccount<'info>,

    /// CHECK: checked by cp-amm (pool has_one).
    #[account(mut)]
    pub token_a_vault: UncheckedAccount<'info>,

    /// CHECK: checked by cp-amm (pool has_one).
    #[account(mut)]
    pub token_b_vault: UncheckedAccount<'info>,

    /// CHECK: checked by cp-amm (mint == position.nft_mint, amount == 1, owner == signer).
    pub position_nft_account: UncheckedAccount<'info>,

    /// CHECK: address checked.
    #[account(address = cp_amm::EVENT_AUTHORITY)]
    pub event_authority: UncheckedAccount<'info>,

    /// CHECK: address checked.
    #[account(address = cp_amm::CP_AMM_PROGRAM_ID)]
    pub cp_amm_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// Create a PDA-owned account even if someone pre-funded its address with lamports.
fn create_pda_account<'info>(
    payer: &AccountInfo<'info>,
    target: &AccountInfo<'info>,
    space: u64,
    owner: &Pubkey,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let rent = Rent::get()?.minimum_balance(space as usize);
    let current = target.lamports();
    if current == 0 {
        create_account(
            CpiContext::new_with_signer(
                anchor_lang::system_program::ID,
                CreateAccount {
                    from: payer.clone(),
                    to: target.clone(),
                },
                signer_seeds,
            ),
            rent,
            space,
            owner,
        )?;
    } else {
        let needed = rent.saturating_sub(current);
        if needed > 0 {
            transfer(
                CpiContext::new_with_signer(
                    anchor_lang::system_program::ID,
                    Transfer {
                        from: payer.clone(),
                        to: target.clone(),
                    },
                    signer_seeds,
                ),
                needed,
            )?;
        }
        allocate(
            CpiContext::new_with_signer(
                anchor_lang::system_program::ID,
                Allocate {
                    account_to_allocate: target.clone(),
                },
                signer_seeds,
            ),
            space,
        )?;
        assign(
            CpiContext::new_with_signer(
                anchor_lang::system_program::ID,
                Assign {
                    account_to_assign: target.clone(),
                },
                signer_seeds,
            ),
            owner,
        )?;
    }
    Ok(())
}

pub fn handle_collect<'info>(ctx: Context<'info, Collect<'info>>) -> Result<()> {
    require!(ctx.accounts.coin.state == STATE_OPENED, NonceError::NotOpen);

    // the coin's token program: classic, or Token-2022 passed as the first remaining account
    let token_a_program = if ctx.accounts.coin.token_2022 {
        let p = ctx
            .remaining_accounts
            .first()
            .ok_or(NonceError::InvalidTokenProgram)?;
        require_keys_eq!(p.key(), TOKEN_2022_PROGRAM_ID, NonceError::InvalidTokenProgram);
        p.clone()
    } else {
        ctx.accounts.token_program.to_account_info()
    };
    {
        // vault_token_a: a token account of this mint, owned by the vault, under that program
        let info = ctx.accounts.vault_token_a.to_account_info();
        require_keys_eq!(*info.owner, token_a_program.key(), NonceError::InvalidTokenAccount);
        let data = info.try_borrow_data()?;
        require!(
            data.len() >= 64
                && data[0..32] == ctx.accounts.mint.key().to_bytes()
                && data[32..64] == ctx.accounts.vault.key().to_bytes(),
            NonceError::InvalidTokenAccount
        );
    }

    let coin_key = ctx.accounts.coin.key();
    let vault_bump = ctx.accounts.coin.vault_bump;
    let fee_bump = ctx.bumps.fee_account;
    let vault_seeds: &[&[u8]] = &[VAULT_SEED, coin_key.as_ref(), &[vault_bump]];
    let fee_seeds: &[&[u8]] = &[FEE_SEED, coin_key.as_ref(), &[fee_bump]];

    let vault = ctx.accounts.vault.to_account_info();
    let fee_account = ctx.accounts.fee_account.to_account_info();

    // 1. temporary wSOL account owned by the vault
    create_pda_account(
        &vault,
        &fee_account,
        TOKEN_ACCOUNT_LEN as u64,
        &token::ID,
        &[vault_seeds, fee_seeds],
    )?;
    initialize_account3(CpiContext::new(
        token::ID,
        InitializeAccount3 {
            account: fee_account.clone(),
            mint: ctx.accounts.native_mint.to_account_info(),
            authority: vault.clone(),
        },
    ))?;

    // 2. claim the locked position's fees (token A fee is always 0 in OnlyB mode)
    cp_amm::claim_position_fee(
        &ClaimPositionFeeAccounts {
            pool_authority: ctx.accounts.pool_authority.to_account_info(),
            pool: ctx.accounts.pool.to_account_info(),
            position: ctx.accounts.position.to_account_info(),
            token_a_account: ctx.accounts.vault_token_a.to_account_info(),
            token_b_account: fee_account.clone(),
            token_a_vault: ctx.accounts.token_a_vault.to_account_info(),
            token_b_vault: ctx.accounts.token_b_vault.to_account_info(),
            token_a_mint: ctx.accounts.mint.to_account_info(),
            token_b_mint: ctx.accounts.native_mint.to_account_info(),
            position_nft_account: ctx.accounts.position_nft_account.to_account_info(),
            signer: vault.clone(),
            token_a_program: token_a_program.clone(),
            token_b_program: ctx.accounts.token_program.to_account_info(),
            event_authority: ctx.accounts.event_authority.to_account_info(),
            program: ctx.accounts.cp_amm_program.to_account_info(),
        },
        &[vault_seeds],
    )?;
    let amount = token_amount(&fee_account)?;

    // 3. unwrap: close into the vault (fee amount + rent land in the vault)
    close_account(CpiContext::new_with_signer(
        token::ID,
        CloseAccount {
            account: fee_account.clone(),
            destination: vault.clone(),
            authority: vault.clone(),
        },
        &[vault_seeds],
    ))?;

    // 4. split: treasury first, then the launcher's share plus any unused reserve
    let launcher_share = ((amount as u128) * (ctx.accounts.coin.launcher_fee_bps as u128)
        / (BPS as u128)) as u64;
    let treasury_share = amount - launcher_share;
    if treasury_share > 0 {
        transfer(
            CpiContext::new_with_signer(
                anchor_lang::system_program::ID,
                Transfer {
                    from: vault.clone(),
                    to: ctx.accounts.treasury.to_account_info(),
                },
                &[vault_seeds],
            ),
            treasury_share,
        )?;
    }
    let refund = funds::pay_launcher(
        &vault,
        &ctx.accounts.launcher.to_account_info(),
        vault_seeds,
        launcher_share,
    )?;
    if refund > 0 {
        let launcher = ctx.accounts.coin.launcher;
        let coin = &mut ctx.accounts.coin;
        coin.reserve_refunded = coin.reserve_refunded.saturating_add(refund);
        emit!(ReserveRefunded {
            coin: coin_key,
            launcher,
            amount: refund,
        });
    }

    let coin = &mut ctx.accounts.coin;
    coin.collects = coin.collects.saturating_add(1);
    coin.fees_collected = coin.fees_collected.saturating_add(amount);
    coin.fees_to_launcher = coin.fees_to_launcher.saturating_add(launcher_share);
    coin.fees_to_treasury = coin.fees_to_treasury.saturating_add(treasury_share);

    emit!(FeesCollected {
        coin: coin_key,
        amount,
        to_launcher: launcher_share,
        to_treasury: treasury_share,
    });
    Ok(())
}
