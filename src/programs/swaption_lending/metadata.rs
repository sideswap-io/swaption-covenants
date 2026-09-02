use simplex::simplicityhl::elements::AssetId;

use crate::programs::{
    program::{CreationMetadata, PROGRAM_ID_LENGTH, ProgramId},
    swaption_lending::{PositionTerms, SwaptionPositionError},
};

/// OP_RETURN payload written by the fill transaction so that a chain watcher
/// can rebuild the covenant parameters without the server's database.
/// Everything else (collateral asset/amount, both NFT ids) is read from the
/// fill outputs. 80 bytes — the Elements default `datacarriersize`.
///
/// Layout: program_id(4) | cash_asset_id(32) | buyback_amount(8 LE)
///         | expiry_height(4 LE) | lender_payout_script_hash(32)
const CREATION_METADATA_LENGTH: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwaptionPositionCreationMetadata {
    pub program_id: ProgramId,
    pub cash_asset_id: AssetId,
    pub buyback_amount: u64,
    pub expiry_height: u32,
    pub lender_payout_script_hash: [u8; 32],
}

impl SwaptionPositionCreationMetadata {
    pub fn new(
        program_id: ProgramId,
        cash_asset_id: AssetId,
        terms: PositionTerms,
        lender_payout_script_hash: [u8; 32],
    ) -> Self {
        Self {
            program_id,
            cash_asset_id,
            buyback_amount: terms.buyback_amount,
            expiry_height: terms.expiry_height,
            lender_payout_script_hash,
        }
    }
}

impl CreationMetadata for SwaptionPositionCreationMetadata {
    type Error = SwaptionPositionError;

    const DATA_LENGTH: usize = CREATION_METADATA_LENGTH;

    fn decode(op_return_bytes: &[u8]) -> Result<Self, Self::Error> {
        Self::validate_length(op_return_bytes, |expected, actual| {
            SwaptionPositionError::InvalidCreationMetadataLength { expected, actual }
        })?;

        let mut cursor = 0;

        let program_id = Self::decode_program_id(op_return_bytes);
        cursor += PROGRAM_ID_LENGTH;

        let cash_asset_id = AssetId::from_slice(&op_return_bytes[cursor..cursor + 32])?;
        cursor += 32;

        let buyback_amount = u64::from_le_bytes(
            op_return_bytes[cursor..cursor + 8]
                .try_into()
                .expect("u64 length is fixed"),
        );
        cursor += 8;

        let expiry_height = u32::from_le_bytes(
            op_return_bytes[cursor..cursor + 4]
                .try_into()
                .expect("u32 length is fixed"),
        );
        cursor += 4;

        let mut lender_payout_script_hash = [0u8; 32];
        lender_payout_script_hash.copy_from_slice(&op_return_bytes[cursor..cursor + 32]);

        Ok(Self {
            program_id,
            cash_asset_id,
            buyback_amount,
            expiry_height,
            lender_payout_script_hash,
        })
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CREATION_METADATA_LENGTH);
        out.extend_from_slice(&self.program_id);
        out.extend_from_slice(&self.cash_asset_id.into_inner().0);
        out.extend_from_slice(&self.buyback_amount.to_le_bytes());
        out.extend_from_slice(&self.expiry_height.to_le_bytes());
        out.extend_from_slice(&self.lender_payout_script_hash);
        debug_assert_eq!(out.len(), CREATION_METADATA_LENGTH);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_roundtrips_at_80_bytes() {
        let m = SwaptionPositionCreationMetadata {
            program_id: [1, 2, 3, 4],
            cash_asset_id: AssetId::from_slice(&[7u8; 32]).unwrap(),
            buyback_amount: 6_200_000_000,
            expiry_height: 3_141_592,
            lender_payout_script_hash: [9u8; 32],
        };
        let bytes = m.encode();
        assert_eq!(bytes.len(), 80);
        assert_eq!(SwaptionPositionCreationMetadata::decode(&bytes).unwrap(), m);
        assert!(SwaptionPositionCreationMetadata::decode(&bytes[..79]).is_err());
    }
}
