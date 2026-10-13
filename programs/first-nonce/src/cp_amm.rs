//! Hand-written CPI into Meteora DAMM v2 (cp-amm).
//!
//! Checked against github.com/MeteoraAg/damm-v2 @ a85c926 (cp_amm 0.2.4) and the published
//! IDL in github.com/MeteoraAg/damm-v2-sdk @ 79ebbfe (src/idl/cp_amm.json, 0.2.5).
//! Discriminators are anchor's sha256("global:<name>")[..8]; they match the IDL.
//! See docs/meteora-notes.md for the source references.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::{instruction::Instruction, program::invoke_signed};

use crate::errors::NonceError;

pub const CP_AMM_PROGRAM_ID: Pubkey = pubkey!("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG");
/// PDA ["pool_authority"] of cp-amm (const_pda.rs).
pub const POOL_AUTHORITY: Pubkey = pubkey!("HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC");
/// PDA ["__event_authority"] of cp-amm (event_cpi).
pub const EVENT_AUTHORITY: Pubkey = pubkey!("3rmHSu74h1ZcmAisVcWerTCiRDQbUrBKmcwptYGjHfet");

/// constants.rs
pub const MIN_SQRT_PRICE: u128 = 4_295_048_016;
pub const MAX_SQRT_PRICE: u128 = 79_226_673_521_066_979_257_578_248_091;

pub const ACTIVATION_TYPE_TIMESTAMP: u8 = 1; // ActivationType::{Slot = 0, Timestamp = 1}
pub const COLLECT_FEE_MODE_ONLY_B: u8 = 1; // CollectFeeMode::{BothToken = 0, OnlyB = 1, Compounding = 2}

pub const IX_INITIALIZE_CUSTOMIZABLE_POOL: [u8; 8] = [20, 161, 241, 24, 189, 221, 180, 2];
pub const IX_PERMANENT_LOCK_POSITION: [u8; 8] = [165, 176, 125, 6, 231, 171, 186, 213];
pub const IX_CLAIM_POSITION_FEE: [u8; 8] = [180, 38, 154, 17, 133, 33, 162, 211];

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct BaseFeeParameters {
    pub data: [u8; 27],
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct DynamicFeeParameters {
    pub bin_step: u16,
    pub bin_step_u128: u128,
    pub filter_period: u16,
    pub decay_period: u16,
    pub reduction_factor: u16,
    pub max_volatility_accumulator: u32,
    pub variable_fee_control: u32,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct PoolFeeParameters {
    pub base_fee: BaseFeeParameters,
    pub compounding_fee_bps: u16,
    pub padding: u8,
    pub dynamic_fee: Option<DynamicFeeParameters>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy)]
pub struct InitializeCustomizablePoolParameters {
    pub pool_fees: PoolFeeParameters,
    pub sqrt_min_price: u128,
    pub sqrt_max_price: u128,
    pub has_alpha_vault: bool,
    pub liquidity: u128,
    pub sqrt_price: u128,
    pub activation_type: u8,
    pub collect_fee_mode: u8,
    pub activation_point: Option<u64>,
}

fn ix_data<T: AnchorSerialize>(disc: [u8; 8], args: &T) -> Result<Vec<u8>> {
    let mut data = disc.to_vec();
    let body = anchor_lang::prelude::borsh::to_vec(args).map_err(|_| error!(NonceError::Serialization))?;
    data.extend_from_slice(&body);
    Ok(data)
}

pub struct InitializeCustomizablePoolAccounts<'info> {
    pub creator: AccountInfo<'info>,
    pub position_nft_mint: AccountInfo<'info>,
    pub position_nft_account: AccountInfo<'info>,
    pub payer: AccountInfo<'info>,
    pub pool_authority: AccountInfo<'info>,
    pub pool: AccountInfo<'info>,
    pub position: AccountInfo<'info>,
    pub token_a_mint: AccountInfo<'info>,
    pub token_b_mint: AccountInfo<'info>,
    pub token_a_vault: AccountInfo<'info>,
    pub token_b_vault: AccountInfo<'info>,
    pub payer_token_a: AccountInfo<'info>,
    pub payer_token_b: AccountInfo<'info>,
    pub token_a_program: AccountInfo<'info>,
    pub token_b_program: AccountInfo<'info>,
    pub token_2022_program: AccountInfo<'info>,
    pub system_program: AccountInfo<'info>,
    pub event_authority: AccountInfo<'info>,
    pub program: AccountInfo<'info>,
}

/// Account order = InitializeCustomizablePoolCtx (ix_initialize_customizable_pool.rs) + event_cpi.
pub fn initialize_customizable_pool<'info>(
    a: &InitializeCustomizablePoolAccounts<'info>,
    params: &InitializeCustomizablePoolParameters,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let accounts = vec![
        AccountMeta::new_readonly(a.creator.key(), false),
        AccountMeta::new(a.position_nft_mint.key(), true),
        AccountMeta::new(a.position_nft_account.key(), false),
        AccountMeta::new(a.payer.key(), true),
        AccountMeta::new_readonly(a.pool_authority.key(), false),
        AccountMeta::new(a.pool.key(), false),
        AccountMeta::new(a.position.key(), false),
        AccountMeta::new_readonly(a.token_a_mint.key(), false),
        AccountMeta::new_readonly(a.token_b_mint.key(), false),
        AccountMeta::new(a.token_a_vault.key(), false),
        AccountMeta::new(a.token_b_vault.key(), false),
        AccountMeta::new(a.payer_token_a.key(), false),
        AccountMeta::new(a.payer_token_b.key(), false),
        AccountMeta::new_readonly(a.token_a_program.key(), false),
        AccountMeta::new_readonly(a.token_b_program.key(), false),
        AccountMeta::new_readonly(a.token_2022_program.key(), false),
        AccountMeta::new_readonly(a.system_program.key(), false),
        AccountMeta::new_readonly(a.event_authority.key(), false),
        AccountMeta::new_readonly(a.program.key(), false),
    ];
    let ix = Instruction {
        program_id: CP_AMM_PROGRAM_ID,
        accounts,
        data: ix_data(IX_INITIALIZE_CUSTOMIZABLE_POOL, params)?,
    };
    invoke_signed(
        &ix,
        &[
            a.creator.clone(),
            a.position_nft_mint.clone(),
            a.position_nft_account.clone(),
            a.payer.clone(),
            a.pool_authority.clone(),
            a.pool.clone(),
            a.position.clone(),
            a.token_a_mint.clone(),
            a.token_b_mint.clone(),
            a.token_a_vault.clone(),
            a.token_b_vault.clone(),
            a.payer_token_a.clone(),
            a.payer_token_b.clone(),
            a.token_a_program.clone(),
            a.token_b_program.clone(),
            a.token_2022_program.clone(),
            a.system_program.clone(),
            a.event_authority.clone(),
            a.program.clone(),
        ],
        signer_seeds,
    )?;
    Ok(())
}

pub struct PermanentLockPositionAccounts<'info> {
    pub pool: AccountInfo<'info>,
    pub position: AccountInfo<'info>,
    pub position_nft_account: AccountInfo<'info>,
    pub signer: AccountInfo<'info>,
    pub event_authority: AccountInfo<'info>,
    pub program: AccountInfo<'info>,
}

/// Account order = PermanentLockPositionCtx (ix_permanent_lock_position.rs) + event_cpi.
pub fn permanent_lock_position<'info>(
    a: &PermanentLockPositionAccounts<'info>,
    liquidity: u128,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let accounts = vec![
        AccountMeta::new(a.pool.key(), false),
        AccountMeta::new(a.position.key(), false),
        AccountMeta::new_readonly(a.position_nft_account.key(), false),
        AccountMeta::new_readonly(a.signer.key(), true),
        AccountMeta::new_readonly(a.event_authority.key(), false),
        AccountMeta::new_readonly(a.program.key(), false),
    ];
    let ix = Instruction {
        program_id: CP_AMM_PROGRAM_ID,
        accounts,
        data: ix_data(IX_PERMANENT_LOCK_POSITION, &liquidity)?,
    };
    invoke_signed(
        &ix,
        &[
            a.pool.clone(),
            a.position.clone(),
            a.position_nft_account.clone(),
            a.signer.clone(),
            a.event_authority.clone(),
            a.program.clone(),
        ],
        signer_seeds,
    )?;
    Ok(())
}

pub struct ClaimPositionFeeAccounts<'info> {
    pub pool_authority: AccountInfo<'info>,
    pub pool: AccountInfo<'info>,
    pub position: AccountInfo<'info>,
    pub token_a_account: AccountInfo<'info>,
    pub token_b_account: AccountInfo<'info>,
    pub token_a_vault: AccountInfo<'info>,
    pub token_b_vault: AccountInfo<'info>,
    pub token_a_mint: AccountInfo<'info>,
    pub token_b_mint: AccountInfo<'info>,
    pub position_nft_account: AccountInfo<'info>,
    pub signer: AccountInfo<'info>,
    pub token_a_program: AccountInfo<'info>,
    pub token_b_program: AccountInfo<'info>,
    pub event_authority: AccountInfo<'info>,
    pub program: AccountInfo<'info>,
}

/// Account order = ClaimPositionFeeCtx (ix_claim_position_fee.rs) + event_cpi.
pub fn claim_position_fee<'info>(
    a: &ClaimPositionFeeAccounts<'info>,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let accounts = vec![
        AccountMeta::new_readonly(a.pool_authority.key(), false),
        AccountMeta::new_readonly(a.pool.key(), false),
        AccountMeta::new(a.position.key(), false),
        AccountMeta::new(a.token_a_account.key(), false),
        AccountMeta::new(a.token_b_account.key(), false),
        AccountMeta::new(a.token_a_vault.key(), false),
        AccountMeta::new(a.token_b_vault.key(), false),
        AccountMeta::new_readonly(a.token_a_mint.key(), false),
        AccountMeta::new_readonly(a.token_b_mint.key(), false),
        AccountMeta::new_readonly(a.position_nft_account.key(), false),
        AccountMeta::new_readonly(a.signer.key(), true),
        AccountMeta::new_readonly(a.token_a_program.key(), false),
        AccountMeta::new_readonly(a.token_b_program.key(), false),
        AccountMeta::new_readonly(a.event_authority.key(), false),
        AccountMeta::new_readonly(a.program.key(), false),
    ];
    let ix = Instruction {
        program_id: CP_AMM_PROGRAM_ID,
        accounts,
        data: IX_CLAIM_POSITION_FEE.to_vec(),
    };
    invoke_signed(
        &ix,
        &[
            a.pool_authority.clone(),
            a.pool.clone(),
            a.position.clone(),
            a.token_a_account.clone(),
            a.token_b_account.clone(),
            a.token_a_vault.clone(),
            a.token_b_vault.clone(),
            a.token_a_mint.clone(),
            a.token_b_mint.clone(),
            a.position_nft_account.clone(),
            a.signer.clone(),
            a.token_a_program.clone(),
            a.token_b_program.clone(),
            a.event_authority.clone(),
            a.program.clone(),
        ],
        signer_seeds,
    )?;
    Ok(())
}
