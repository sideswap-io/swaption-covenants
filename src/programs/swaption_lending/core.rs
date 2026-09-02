use simplex::{
    program::Program,
    provider::SimplicityNetwork,
    simplicityhl::elements::{
        LockTime, Script, Sequence, Transaction,
        hashes::{Hash, sha256},
    },
    transaction::{FinalTransaction, PartialInput, PartialOutput, UTXO},
};

use crate::{
    artifacts::swaption_lending::SwaptionLendingProgram,
    programs::{
        program::{MetadataProgram, SimplexProgram},
        swaption_lending::{
            PositionTerms, SwaptionPositionCreationMetadata, SwaptionPositionError,
            SwaptionPositionParameters, SwaptionPositionWitnessBranch,
        },
    },
    utils::op_return_payload,
};

/// Fill transaction output layout (fixed by the server's template builder):
/// 0 position covenant (collateral), 1 borrower NFT, 2 lender NFT,
/// 3 creation metadata (OP_RETURN). Cash to the borrower, Swaption fees and
/// change follow and are not part of the contract.
pub const FILL_POSITION_OUTPUT_INDEX: usize = 0;
pub const FILL_BORROWER_NFT_OUTPUT_INDEX: usize = 1;
pub const FILL_LENDER_NFT_OUTPUT_INDEX: usize = 2;
pub const FILL_METADATA_OUTPUT_INDEX: usize = 3;

/// The single storage slot: cash still owed to buy back the remaining collateral.
pub struct SwaptionPositionStorage {
    pub remaining_debt: u64,
}

impl SwaptionPositionStorage {
    pub fn set_storage_slots(&self, program: &mut SwaptionLendingProgram) {
        #[allow(unused_must_use)]
        program.set_storage_at(0, self.slot_value());
    }

    fn slot_value(&self) -> [u8; 32] {
        let mut slot = [0u8; 32];
        slot[24..32].copy_from_slice(&self.remaining_debt.to_be_bytes());
        slot
    }
}

/// SHA-256 of a scriptPubKey — what the `input_script_hash` / `output_script_hash`
/// jets expose and what `LENDER_PAYOUT_SCRIPT_HASH` commits to.
pub fn script_hash(script: &Script) -> [u8; 32] {
    sha256::Hash::hash(script.as_bytes()).to_byte_array()
}

pub struct SwaptionPosition {
    program: SwaptionLendingProgram,
    parameters: SwaptionPositionParameters,
    storage: SwaptionPositionStorage,
}

impl SwaptionPosition {
    /// A freshly filled position: nothing bought back yet.
    pub fn new_filled(parameters: SwaptionPositionParameters) -> Self {
        Self::new(parameters, parameters.terms.buyback_amount)
    }

    pub fn new(parameters: SwaptionPositionParameters, remaining_debt: u64) -> Self {
        assert!(
            remaining_debt <= parameters.terms.buyback_amount,
            "remaining debt can't exceed the buyback amount"
        );

        let storage = SwaptionPositionStorage { remaining_debt };
        let mut program =
            SwaptionLendingProgram::new(parameters.build_arguments()).with_storage_capacity(1);
        storage.set_storage_slots(&mut program);

        Self {
            program,
            parameters,
            storage,
        }
    }

    /// Rebuild a freshly filled position from its fill transaction.
    pub fn try_from_fill_tx(
        tx: &Transaction,
        network: SimplicityNetwork,
    ) -> Result<Self, SwaptionPositionError> {
        if tx.output.len() <= FILL_METADATA_OUTPUT_INDEX
            || !tx.output[FILL_METADATA_OUTPUT_INDEX].is_null_data()
        {
            return Err(SwaptionPositionError::NotAPositionCreationTx(tx.txid()));
        }

        let op_return_bytes = op_return_payload(&tx.output[FILL_METADATA_OUTPUT_INDEX].script_pubkey)
            .ok_or_else(|| SwaptionPositionError::NotAPositionCreationTx(tx.txid()))?;

        let metadata = Self::decode_metadata_op_return(op_return_bytes.to_vec())?;
        if metadata.program_id != Self::get_program_id() {
            return Err(SwaptionPositionError::UnknownProgramId);
        }

        let explicit = |index: usize| {
            let out = &tx.output[index];
            match (out.asset.explicit(), out.value.explicit()) {
                (Some(asset), Some(amount)) => Ok((asset, amount)),
                _ => Err(SwaptionPositionError::NotExplicit(index)),
            }
        };

        let (collateral_asset_id, collateral_amount) = explicit(FILL_POSITION_OUTPUT_INDEX)?;
        let (borrower_nft_asset_id, _) = explicit(FILL_BORROWER_NFT_OUTPUT_INDEX)?;
        let (lender_nft_asset_id, _) = explicit(FILL_LENDER_NFT_OUTPUT_INDEX)?;

        let parameters = SwaptionPositionParameters {
            collateral_asset_id,
            cash_asset_id: metadata.cash_asset_id,
            borrower_nft_asset_id,
            lender_nft_asset_id,
            lender_payout_script_hash: metadata.lender_payout_script_hash,
            terms: PositionTerms {
                collateral_amount,
                buyback_amount: metadata.buyback_amount,
                expiry_height: metadata.expiry_height,
            },
            network,
        };

        let position = Self::new_filled(parameters);

        if tx.output[FILL_POSITION_OUTPUT_INDEX].script_pubkey != position.get_script_pubkey() {
            return Err(SwaptionPositionError::NotAPositionCreationTx(tx.txid()));
        }

        Ok(position)
    }

    pub fn get_parameters(&self) -> &SwaptionPositionParameters {
        &self.parameters
    }

    pub fn get_remaining_debt(&self) -> u64 {
        self.storage.remaining_debt
    }

    pub fn get_remaining_collateral(&self) -> u64 {
        self.parameters
            .terms
            .remaining_collateral(self.storage.remaining_debt)
    }

    pub fn is_settled(&self) -> bool {
        self.storage.remaining_debt == 0
    }

    /// Fill: outputs 0 position covenant, 1 borrower NFT, 2 lender NFT,
    /// 3 creation metadata. Must be the first outputs of the fill transaction;
    /// cash to the borrower, Swaption fees and change come after.
    pub fn attach_fill(
        &self,
        ft: &mut FinalTransaction,
        borrower_script_pubkey: Script,
        lender_script_pubkey: Script,
    ) {
        assert!(
            ft.n_outputs() == FILL_POSITION_OUTPUT_INDEX,
            "position output must be output 0 of the fill transaction"
        );

        self.add_program_output(
            ft,
            self.parameters.collateral_asset_id,
            self.parameters.terms.collateral_amount,
        );

        ft.add_output(PartialOutput::new(
            borrower_script_pubkey,
            1,
            self.parameters.borrower_nft_asset_id,
        ));

        ft.add_output(PartialOutput::new(
            lender_script_pubkey,
            1,
            self.parameters.lender_nft_asset_id,
        ));

        ft.add_output(PartialOutput::new_metadata(
            &self.encode_metadata_op_return(),
        ));
    }

    /// Exercise: pay `amount` of cash to the lender script, unlock collateral.
    ///
    /// Expects the borrower NFT already attached as input 0 and, for a partial
    /// exercise, its round-trip output as output 0. For a full exercise this
    /// adds the burn output itself. Adds: the position input (index 1), the
    /// continuing position output (partial only, index 1), the lender payout
    /// output. The released collateral output is the caller's (any index after).
    ///
    /// `lender_payout_script_pubkey` must hash to the committed
    /// `lender_payout_script_hash` or the covenant rejects the transaction;
    /// it is not checked here so that builders can prove that on regtest.
    pub fn attach_exercise(
        &mut self,
        ft: &mut FinalTransaction,
        position_utxo: UTXO,
        amount: u64,
        lender_payout_script_pubkey: Script,
    ) {
        assert!(ft.n_inputs() == 1, "borrower NFT must be input 0");

        let current_debt = self.get_remaining_debt();
        assert!(amount > 0 && amount <= current_debt, "invalid exercise amount");

        let new_debt = current_debt - amount;
        let is_partial = new_debt > 0;

        if is_partial {
            assert!(ft.n_outputs() == 1, "borrower NFT round-trip must be output 0");
        } else {
            assert!(ft.n_outputs() == 0, "full exercise adds the NFT burn itself");
            ft.add_output(PartialOutput::new(
                Script::new_op_return(b"burn"),
                1,
                self.parameters.borrower_nft_asset_id,
            ));
        }

        self.add_program_input(
            ft,
            position_utxo,
            SwaptionPositionWitnessBranch::Exercise {
                current_debt,
                amount,
            }
            .build_witness(),
        );

        self.set_remaining_debt(new_debt);

        if is_partial {
            self.add_program_output(
                ft,
                self.parameters.collateral_asset_id,
                self.get_remaining_collateral(),
            );
        }

        ft.add_output(PartialOutput::new(
            lender_payout_script_pubkey,
            amount,
            self.parameters.cash_asset_id,
        ));
    }

    /// Lapse: after expiry the lender takes what is left.
    ///
    /// Adds the position input at index 0 (with the locktime) and the lender
    /// NFT burn output at index 0. The caller then adds the lender NFT input at
    /// index 1 and the collateral output wherever it likes.
    pub fn attach_lapse(&self, ft: &mut FinalTransaction, position_utxo: UTXO) {
        assert!(ft.n_inputs() == 0 && ft.n_outputs() == 0, "lapse must start the transaction");

        let locktime = LockTime::from_height(self.parameters.terms.expiry_height)
            .expect("expiry height is a valid lock height");

        let position_input = PartialInput::new(position_utxo)
            .with_sequence(Sequence::ENABLE_LOCKTIME_NO_RBF)
            .with_locktime(locktime);

        self.add_program_input_from_partial_input(
            ft,
            position_input,
            SwaptionPositionWitnessBranch::Lapse {
                current_debt: self.get_remaining_debt(),
            }
            .build_witness(),
        );

        ft.add_output(PartialOutput::new(
            Script::new_op_return(b"burn"),
            1,
            self.parameters.lender_nft_asset_id,
        ));
    }

    fn set_remaining_debt(&mut self, new_debt: u64) {
        self.storage.remaining_debt = new_debt;
        self.storage.set_storage_slots(&mut self.program);
    }
}

impl SimplexProgram for SwaptionPosition {
    fn get_program_source_code() -> &'static str {
        SwaptionLendingProgram::SOURCE
    }

    fn get_program(&self) -> &Program {
        self.program.as_ref()
    }

    fn get_network(&self) -> &SimplicityNetwork {
        &self.parameters.network
    }
}

impl MetadataProgram for SwaptionPosition {
    type Metadata = SwaptionPositionCreationMetadata;

    fn build_metadata(&self) -> Self::Metadata {
        SwaptionPositionCreationMetadata::new(
            Self::get_program_id(),
            self.parameters.cash_asset_id,
            self.parameters.terms,
            self.parameters.lender_payout_script_hash,
        )
    }
}
