use simplex::program::{ArgumentsTrait, Program};
use simplex::provider::SimplicityNetwork;
use simplex::simplicityhl::Arguments;
use simplex::simplicityhl::elements::{LockTime, Script, Sequence};
use simplex::transaction::{FinalTransaction, PartialInput, PartialOutput, UTXO};

use crate::artifacts::swaption_offer::SwaptionOfferProgram;
use crate::programs::program::SimplexProgram;
use crate::programs::swaption_lending_v5::{PositionParametersV5, SwaptionPositionV5};
use crate::programs::swaption_offer::{FillOutcome, OfferParameters, WitnessBranchOffer};

/// The program takes no arguments: the terms live in storage slot 0.
#[derive(Clone)]
struct NoArguments;

impl ArgumentsTrait for NoArguments {
    fn build_arguments(&self) -> Arguments {
        Arguments::default()
    }
}

pub struct SwaptionOffer {
    program: SwaptionOfferProgram,
    parameters: OfferParameters,
    remaining: u64,
}

fn u64_slot(value: u64) -> [u8; 32] {
    let mut slot = [0u8; 32];
    slot[24..32].copy_from_slice(&value.to_be_bytes());
    slot
}

impl SwaptionOffer {
    pub fn new(parameters: OfferParameters, remaining: u64) -> Self {
        let mut program = SwaptionOfferProgram::new(NoArguments).with_storage_capacity(2);
        #[allow(unused_must_use)]
        program.set_storage_at(0, parameters.digest());
        #[allow(unused_must_use)]
        program.set_storage_at(1, u64_slot(remaining));
        Self {
            program,
            parameters,
            remaining,
        }
    }

    pub fn get_parameters(&self) -> &OfferParameters {
        &self.parameters
    }

    pub fn get_remaining(&self) -> u64 {
        self.remaining
    }

    /// Post: the offer output, `remaining` of the cash asset, at the next
    /// output index (the brief puts it at 0; the covenant does not care).
    pub fn attach_post(&self, ft: &mut FinalTransaction) {
        self.add_program_output(ft, self.parameters.cash_asset_id, self.remaining);
    }

    /// Fill: the offer is input 0 (the caller adds the borrower token, the
    /// collateral and the fee coin after). Outputs 0..: the v5 position for
    /// `position`, the borrower token to `borrower_token_script`, the
    /// continuation or the leftover (to `claim_script`) when cash is left,
    /// then `fee` of the cash to `fee_script`. The proceeds and every change
    /// are the caller's. Returns the continued offer, if any.
    #[allow(clippy::too_many_arguments)]
    pub fn attach_fill(
        &self,
        ft: &mut FinalTransaction,
        offer_utxo: UTXO,
        row: u8,
        position: PositionParametersV5,
        borrower_token_script: Script,
        claim_script: Script,
        fee_script: Script,
        fee: u64,
    ) -> Option<SwaptionOffer> {
        assert!(ft.n_inputs() == 0 && ft.n_outputs() == 0, "the offer must start the fill transaction");
        let n = position.terms.collateral_amount;
        let outcome = self.parameters.fill_outcome(row, self.remaining, n);
        let witness = WitnessBranchOffer::fill(&self.parameters, self.remaining, row, &position).build_witness();
        self.add_program_input(ft, offer_utxo, witness);

        let created = SwaptionPositionV5::new_filled(position);
        ft.add_output(PartialOutput::new(created.get_script_pubkey(), n, position.collateral_asset_id));
        ft.add_output(PartialOutput::new(borrower_token_script, 1, position.borrower_nft_asset_id));
        let next = self.attach_rest(ft, &outcome, claim_script);
        ft.add_output(PartialOutput::new(fee_script, fee, self.parameters.cash_asset_id));
        next
    }

    /// Output 2 of a fill: the continuation, the leftover, or nothing.
    pub fn attach_rest(&self, ft: &mut FinalTransaction, outcome: &FillOutcome, claim_script: Script) -> Option<SwaptionOffer> {
        if outcome.continues {
            let next = SwaptionOffer::new(self.parameters, outcome.rest);
            next.add_program_output(ft, self.parameters.cash_asset_id, outcome.rest);
            Some(next)
        } else {
            if outcome.rest > 0 {
                ft.add_output(PartialOutput::new(claim_script, outcome.rest, self.parameters.cash_asset_id));
            }
            None
        }
    }

    /// Cancel: one unit of the lender token must already be input 0 (its
    /// holder signs it); the offer becomes input 1. Outputs are free.
    pub fn attach_cancel(&self, ft: &mut FinalTransaction, offer_utxo: UTXO) {
        assert!(ft.n_inputs() == 1, "the lender token must be input 0, then the offer");
        let witness = WitnessBranchOffer::Cancel {
            terms: self.parameters.witness_terms(),
            remaining: self.remaining,
        }
        .build_witness();
        self.add_program_input(ft, offer_utxo, witness);
    }

    /// Expire: the offer is input 0 with the cutoff as lock height, output 0
    /// pays all the remaining cash to `claim_script`. Anyone may add the fee
    /// input and broadcast.
    pub fn attach_expire(&self, ft: &mut FinalTransaction, offer_utxo: UTXO, claim_script: Script) {
        assert!(ft.n_inputs() == 0 && ft.n_outputs() == 0, "expire must start the transaction");
        let locktime = LockTime::from_height(self.parameters.cutoff_height).expect("cutoff height is a valid lock height");
        let input = PartialInput::new(offer_utxo)
            .with_sequence(Sequence::ENABLE_LOCKTIME_NO_RBF)
            .with_locktime(locktime);
        let witness = WitnessBranchOffer::Expire {
            terms: self.parameters.witness_terms(),
            remaining: self.remaining,
        }
        .build_witness();
        self.add_program_input_from_partial_input(ft, input, witness);
        ft.add_output(PartialOutput::new(claim_script, self.remaining, self.parameters.cash_asset_id));
    }
}

impl SimplexProgram for SwaptionOffer {
    fn get_program_source_code() -> &'static str {
        SwaptionOfferProgram::SOURCE
    }

    fn get_program(&self) -> &Program {
        self.program.as_ref()
    }

    fn get_network(&self) -> &SimplicityNetwork {
        &self.parameters.network
    }
}

/// The constant program's tapleaf hash — what a wallet pins.
pub fn tapleaf_hash_of_program() -> [u8; 32] {
    use simplex::simplicityhl::elements::hashes::Hash as _;
    use simplex::simplicityhl::elements::taproot::{LeafVersion, TapLeafHash};
    let cmr = simplex::simplicityhl::CompiledProgram::new(SwaptionOfferProgram::SOURCE, Arguments::default(), true)
        .expect("offer compiles")
        .commit()
        .cmr();
    let script = Script::from(cmr.as_ref().to_vec());
    TapLeafHash::from_script(&script, LeafVersion::from_u8(0xbe).expect("simplicity leaf")).to_byte_array()
}

/// The offer scriptPubKey from hashes alone: the same three-leaf tree as a
/// position, branch(branch(program, digest), remaining).
pub fn offer_script_pubkey(program_leaf: [u8; 32], digest: [u8; 32], remaining: u64) -> Script {
    crate::programs::swaption_lending_v5::position_script_pubkey(program_leaf, digest, remaining)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::programs::swaption_offer::{OFFER_ROWS, OfferRow};
    use simplex::simplicityhl::elements::AssetId;

    fn params() -> OfferParameters {
        let asset = |b: u8| AssetId::from_slice(&[b; 32]).unwrap();
        let mut rows = [OfferRow::empty(); OFFER_ROWS];
        rows[0] = OfferRow {
            collateral_asset_id: asset(1),
            expiry_height: 3_200_000,
            price_out: 300_000_000,
            buyback_price: 400_000_000,
            fee_per_unit: 2_000_000,
            min_size: 1_000,
        };
        OfferParameters {
            cash_asset_id: asset(2),
            lender_token_asset_id: asset(4),
            claim_script_hash: [9u8; 32],
            fee_script_hash: [12u8; 32],
            position_leaf: crate::programs::swaption_lending_v5::tapleaf_hash_of_program(),
            fee_min: 50,
            cutoff_height: 3_199_000,
            rows,
            network: SimplicityNetwork::LiquidTestnet,
        }
    }

    /// Compiles the program (the real SimplicityHL check) and prints the
    /// leaf a wallet pins.
    #[test]
    fn print_program_leaf() {
        println!("SWAPTION_OFFER_LEAF={}", hex::encode(tapleaf_hash_of_program()));
        // Vectors for the SDK's hash-only reconstruction (`lending::offer_script`).
        let p = params();
        println!("OFFER_VECTOR_DIGEST={}", hex::encode(p.digest()));
        for remaining in [25_000u64, 1] {
            println!("OFFER_VECTOR_SCRIPT_{remaining}={}", hex::encode(SwaptionOffer::new(p, remaining).get_script_pubkey().as_bytes()));
        }
    }

    #[test]
    fn hash_only_script_matches_simplex() {
        let leaf = tapleaf_hash_of_program();
        let p = params();
        for remaining in [25_000u64, 16_000, 1] {
            let offer = SwaptionOffer::new(p, remaining);
            assert_eq!(offer_script_pubkey(leaf, p.digest(), remaining), offer.get_script_pubkey(), "remaining {remaining}");
        }
    }

    #[test]
    fn continuation_changes_only_slot_one() {
        let p = params();
        let a = SwaptionOffer::new(p, 25_000);
        let b = SwaptionOffer::new(p, 16_000);
        assert_ne!(a.get_script_pubkey(), b.get_script_pubkey());
        assert_eq!(a.program.get_storage_at(0), b.program.get_storage_at(0));
        assert_eq!(a.program.get_storage_at(1), u64_slot(25_000));
    }
}
