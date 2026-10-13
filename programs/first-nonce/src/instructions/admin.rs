use anchor_lang::prelude::*;
use anchor_lang::solana_program::bpf_loader_upgradeable::get_program_data_address;

use crate::{constants::*, errors::NonceError, state::*};

/// Only the program's upgrade authority can create the config (prevents a front-run of the
/// first call).
#[derive(Accounts)]
pub struct InitConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        init,
        payer = authority,
        space = 8 + Config::INIT_SPACE,
        seeds = [CONFIG_SEED],
        bump
    )]
    pub config: Box<Account<'info, Config>>,
    #[account(
        address = get_program_data_address(&crate::ID),
        constraint = program_data.upgrade_authority_address == Some(authority.key()) @ NonceError::Unauthorized
    )]
    pub program_data: Account<'info, ProgramData>,
    pub system_program: Program<'info, System>,
}

pub fn handle_init_config(ctx: Context<InitConfig>, admin: Pubkey, treasury: Pubkey) -> Result<()> {
    let config = &mut ctx.accounts.config;
    config.admin = admin;
    config.treasury = treasury;
    config.launch_fee_lamports = DEFAULT_LAUNCH_FEE_LAMPORTS;
    config.launcher_fee_bps = DEFAULT_LAUNCHER_FEE_BPS;
    config.paused = false;
    config.share_target = DEFAULT_SHARE_TARGET;
    config.start_target = DEFAULT_START_TARGET;
    config.default_open_fdv_lamports = DEFAULT_OPEN_FDV_LAMPORTS;
    config.min_open_fdv_lamports = DEFAULT_MIN_OPEN_FDV_LAMPORTS;
    config.max_open_fdv_lamports = DEFAULT_MAX_OPEN_FDV_LAMPORTS;
    config.fee_schedule = FeeSchedule {
        cliff_fee_numerator: DEFAULT_CLIFF_FEE_NUMERATOR,
        number_of_period: DEFAULT_FEE_PERIODS,
        period_frequency: DEFAULT_FEE_PERIOD_SECS,
        reduction_factor: DEFAULT_FEE_REDUCTION,
    };
    config.fee_schedule.validate()?;
    config.coins_launched = 0;
    config.bump = ctx.bumps.config;
    config.reserved = [0u8; 64];
    Ok(())
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct UpdateConfigArgs {
    pub admin: Option<Pubkey>,
    pub treasury: Option<Pubkey>,
    pub launch_fee_lamports: Option<u64>,
    pub launcher_fee_bps: Option<u16>,
    pub paused: Option<bool>,
    pub share_target: Option<u32>,
    pub start_target: Option<u64>,
    pub default_open_fdv_lamports: Option<u64>,
    pub min_open_fdv_lamports: Option<u64>,
    pub max_open_fdv_lamports: Option<u64>,
    pub fee_schedule: Option<FeeSchedule>,
}

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin)]
    pub config: Box<Account<'info, Config>>,
}

pub fn handle_update_config(ctx: Context<UpdateConfig>, args: UpdateConfigArgs) -> Result<()> {
    let c = &mut ctx.accounts.config;
    if let Some(v) = args.admin {
        c.admin = v;
    }
    if let Some(v) = args.treasury {
        c.treasury = v;
    }
    if let Some(v) = args.launch_fee_lamports {
        c.launch_fee_lamports = v;
    }
    if let Some(v) = args.launcher_fee_bps {
        require!(v as u64 <= BPS, NonceError::InvalidFeeBps);
        c.launcher_fee_bps = v;
    }
    if let Some(v) = args.paused {
        c.paused = v;
    }
    if let Some(v) = args.share_target {
        require!(v >= 1 && v <= 10_000, NonceError::InvalidShareTarget);
        c.share_target = v;
    }
    if let Some(v) = args.start_target {
        require!(v > 0, NonceError::InvalidStartTarget);
        c.start_target = v;
    }
    if let Some(v) = args.default_open_fdv_lamports {
        c.default_open_fdv_lamports = v;
    }
    if let Some(v) = args.min_open_fdv_lamports {
        c.min_open_fdv_lamports = v;
    }
    if let Some(v) = args.max_open_fdv_lamports {
        c.max_open_fdv_lamports = v;
    }
    if let Some(v) = args.fee_schedule {
        v.validate()?;
        c.fee_schedule = v;
    }
    require!(
        c.min_open_fdv_lamports > 0
            && c.min_open_fdv_lamports <= c.default_open_fdv_lamports
            && c.default_open_fdv_lamports <= c.max_open_fdv_lamports
            && c.max_open_fdv_lamports <= HARD_MAX_OPEN_FDV_LAMPORTS,
        NonceError::InvalidFdvRange
    );
    // the price math must accept the whole range
    crate::mining::sqrt_price_from_fdv(c.min_open_fdv_lamports)?;
    crate::mining::sqrt_price_from_fdv(c.max_open_fdv_lamports)?;
    Ok(())
}
