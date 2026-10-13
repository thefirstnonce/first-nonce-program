//! Hand-written CPI into Metaplex Token Metadata `CreateMetadataAccountV3` (discriminator 33).
//! Layout checked against mpl-token-metadata clients/rust/src/generated/instructions/
//! create_metadata_account_v3.rs. Built by hand to avoid pulling mpl-token-metadata into the
//! anchor 1.2 / solana v3 dependency tree.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::{instruction::Instruction, program::invoke_signed};

use crate::{constants::TOKEN_METADATA_PROGRAM_ID, errors::NonceError};

#[derive(AnchorSerialize)]
struct DataV2 {
    name: String,
    symbol: String,
    uri: String,
    seller_fee_basis_points: u16,
    creators: Option<Vec<u8>>,   // always None (encoded as a single 0 byte)
    collection: Option<u8>,      // always None
    uses: Option<u8>,            // always None
}

#[derive(AnchorSerialize)]
struct CreateMetadataAccountArgsV3 {
    data: DataV2,
    is_mutable: bool,
    collection_details: Option<u8>, // always None
}

pub struct CreateMetadataAccounts<'info> {
    pub metadata: AccountInfo<'info>,
    pub mint: AccountInfo<'info>,
    pub mint_authority: AccountInfo<'info>,
    pub payer: AccountInfo<'info>,
    pub update_authority: AccountInfo<'info>,
    pub system_program: AccountInfo<'info>,
}

/// Creates immutable metadata (is_mutable = false, no creators, no collection).
pub fn create_metadata_v3<'info>(
    a: &CreateMetadataAccounts<'info>,
    name: String,
    symbol: String,
    uri: String,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let args = CreateMetadataAccountArgsV3 {
        data: DataV2 {
            name,
            symbol,
            uri,
            seller_fee_basis_points: 0,
            creators: None,
            collection: None,
            uses: None,
        },
        is_mutable: false,
        collection_details: None,
    };
    let mut data = vec![33u8];
    data.extend_from_slice(
        &anchor_lang::prelude::borsh::to_vec(&args).map_err(|_| error!(NonceError::Serialization))?,
    );
    let ix = Instruction {
        program_id: TOKEN_METADATA_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(a.metadata.key(), false),
            AccountMeta::new_readonly(a.mint.key(), false),
            AccountMeta::new_readonly(a.mint_authority.key(), true),
            AccountMeta::new(a.payer.key(), true),
            AccountMeta::new_readonly(a.update_authority.key(), true),
            AccountMeta::new_readonly(a.system_program.key(), false),
        ],
        data,
    };
    invoke_signed(
        &ix,
        &[
            a.metadata.clone(),
            a.mint.clone(),
            a.mint_authority.clone(),
            a.payer.clone(),
            a.update_authority.clone(),
            a.system_program.clone(),
        ],
        signer_seeds,
    )?;
    Ok(())
}
