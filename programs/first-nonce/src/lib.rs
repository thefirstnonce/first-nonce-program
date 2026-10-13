//! The First Nonce: a Solana launchpad where a new coin's opening supply can't be bought,
//! only mined with proof-of-work, and trading then opens on a single-sided, permanently
//! locked Meteora DAMM v2 pool.
//!
//! Unaudited. Keep the upgrade authority until it is reviewed.

pub mod constants;
pub mod cp_amm;
pub mod errors;
pub mod events;
pub mod funds;
pub mod instructions;
pub mod metadata;
pub mod mining;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("GUW16mLqFWfGjD5AFFSwDxQyGAFSDvPdEbryVa98TGs3");

#[program]
pub mod first_nonce {
    use super::*;

    /// Upgrade authority only: create the global config.
    pub fn init_config(ctx: Context<InitConfig>, admin: Pubkey, treasury: Pubkey) -> Result<()> {
        instructions::admin::handle_init_config(ctx, admin, treasury)
    }

    /// Admin: change treasury, fees, pause, difficulty defaults, price range, fee schedule.
    pub fn update_config(ctx: Context<UpdateConfig>, args: UpdateConfigArgs) -> Result<()> {
        instructions::admin::handle_update_config(ctx, args)
    }

    /// Create a coin: mint, immutable metadata, and block 0 of the dig. Pass Token-2022 as
    /// `token_program` for a Token-2022 mint with its metadata in the mint (the default in the
    /// SDK), or the classic token program for a classic mint with Metaplex metadata.
    pub fn launch(ctx: Context<Launch>, args: LaunchArgs) -> Result<()> {
        instructions::launch::handle_launch(ctx, args)
    }

    /// Submit up to 8 proof-of-work shares for one beneficiary in the open block.
    pub fn submit_shares(ctx: Context<SubmitShares>, block: u32, nonces: Vec<u64>) -> Result<()> {
        instructions::submit::handle_submit_shares(ctx, block, nonces)
    }

    /// Permissionless: close finished blocks and open the next one.
    pub fn crank(ctx: Context<Crank>) -> Result<()> {
        instructions::crank::handle_crank(ctx)
    }

    /// Permissionless after the dig: create and lock the Meteora pool. Optional remaining
    /// account: the coin's launcher (writable), to get the unused open reserve back right away.
    pub fn open<'info>(ctx: Context<'info, Open<'info>>) -> Result<()> {
        instructions::open::handle_open(ctx)
    }

    /// Permissionless after open: mint a miner's tokens to the beneficiary.
    pub fn claim(ctx: Context<Claim>) -> Result<()> {
        instructions::claim::handle_claim(ctx)
    }

    /// Permissionless: claim pool fees and split them launcher / treasury. Also returns any
    /// unused open reserve to the launcher. Remaining account for Token-2022 coins: the
    /// Token-2022 program.
    pub fn collect<'info>(ctx: Context<'info, Collect<'info>>) -> Result<()> {
        instructions::collect::handle_collect(ctx)
    }

    /// Permissionless once everything is claimed: drop the mint authority.
    pub fn seal(ctx: Context<Seal>) -> Result<()> {
        instructions::claim::handle_seal(ctx)
    }
}
