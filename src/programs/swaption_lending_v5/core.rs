use simplex::program::{ArgumentsTrait, Program};
use simplex::provider::SimplicityNetwork;
use simplex::simplicityhl::Arguments;
use simplex::simplicityhl::elements::{LockTime, Script, Sequence};
use simplex::transaction::{FinalTransaction, PartialInput, PartialOutput, UTXO};

use crate::artifacts::swaption_lending_v5::SwaptionLendingV5Program;
use crate::programs::program::SimplexProgram;
use crate::programs::swaption_lending_v5::{PositionParametersV5, WitnessBranchV5};

/// The program takes no arguments — that is the point of v4.
#[derive(Clone)]
struct NoArguments;

impl ArgumentsTrait for NoArguments {
    fn build_arguments(&self) -> Arguments {
        Arguments::default()
    }
}

pub struct SwaptionPositionV5 {
    program: SwaptionLendingV5Program,
    parameters: PositionParametersV5,
    remaining_debt: u64,
}

fn debt_slot(remaining_debt: u64) -> [u8; 32] {
    let mut slot = [0u8; 32];
    slot[24..32].copy_from_slice(&remaining_debt.to_be_bytes());
    slot
}

impl SwaptionPositionV5 {
    pub fn new_filled(parameters: PositionParametersV5) -> Self {
        Self::new(parameters, parameters.terms.buyback_amount)
    }

    pub fn new(parameters: PositionParametersV5, remaining_debt: u64) -> Self {
        assert!(remaining_debt <= parameters.terms.buyback_amount, "remaining debt can't exceed the buyback amount");
        let mut program = SwaptionLendingV5Program::new(NoArguments).with_storage_capacity(2);
        #[allow(unused_must_use)]
        program.set_storage_at(0, parameters.digest());
        #[allow(unused_must_use)]
        program.set_storage_at(1, debt_slot(remaining_debt));
        Self {
            program,
            parameters,
            remaining_debt,
        }
    }

    pub fn get_parameters(&self) -> &PositionParametersV5 {
        &self.parameters
    }

    pub fn get_remaining_debt(&self) -> u64 {
        self.remaining_debt
    }

    pub fn get_remaining_collateral(&self) -> u64 {
        self.parameters.terms.remaining_collateral(self.remaining_debt)
    }

    pub fn is_settled(&self) -> bool {
        self.remaining_debt == 0
    }

    /// Fill outputs: 0 position, 1 borrower NFT, 2 lender NFT. No metadata
    /// output — the terms are bound in the tree and the server keeps them.
    pub fn attach_fill(&self, ft: &mut FinalTransaction, borrower_script_pubkey: Script, lender_script_pubkey: Script) {
        assert!(ft.n_outputs() == 0, "position output must be output 0 of the fill transaction");
        self.add_program_output(ft, self.parameters.collateral_asset_id, self.parameters.terms.collateral_amount);
        ft.add_output(PartialOutput::new(borrower_script_pubkey, 1, self.parameters.borrower_nft_asset_id));
        ft.add_output(PartialOutput::new(lender_script_pubkey, 1, self.parameters.lender_nft_asset_id));
    }

    /// Same contract as v1's `attach_exercise`.
    pub fn attach_exercise(&mut self, ft: &mut FinalTransaction, position_utxo: UTXO, amount: u64, lender_payout_script_pubkey: Script) {
        assert!(ft.n_inputs() == 1, "borrower NFT must be input 0");
        let current_debt = self.remaining_debt;
        assert!(amount > 0 && amount <= current_debt, "invalid exercise amount");
        let new_debt = current_debt - amount;
        let is_partial = new_debt > 0;
        if is_partial {
            assert!(ft.n_outputs() == 1, "borrower NFT round-trip must be output 0");
        } else {
            assert!(ft.n_outputs() == 0, "full exercise adds the NFT burn itself");
            ft.add_output(PartialOutput::new(Script::new_op_return(b"burn"), 1, self.parameters.borrower_nft_asset_id));
        }
        let witness = WitnessBranchV5::Exercise {
            terms: self.parameters.witness_terms(),
            current_debt,
            amount,
        }
        .build_witness();
        self.add_program_input(ft, position_utxo, witness);
        self.set_remaining_debt(new_debt);
        if is_partial {
            self.add_program_output(ft, self.parameters.collateral_asset_id, self.get_remaining_collateral());
        }
        ft.add_output(PartialOutput::new(lender_payout_script_pubkey, amount, self.parameters.cash_asset_id));
    }

    /// Lapse: the position is input 0 and output 0 pays the remaining
    /// collateral, in full, to the lender's payout script. Anyone may add
    /// the fee input and broadcast; no token is needed.
    pub fn attach_lapse(&self, ft: &mut FinalTransaction, position_utxo: UTXO, lender_payout_script_pubkey: Script) {
        assert!(ft.n_inputs() == 0 && ft.n_outputs() == 0, "lapse must start the transaction");
        let locktime = LockTime::from_height(self.parameters.terms.expiry_height).expect("expiry height is a valid lock height");
        let position_input = PartialInput::new(position_utxo)
            .with_sequence(Sequence::ENABLE_LOCKTIME_NO_RBF)
            .with_locktime(locktime);
        let witness = WitnessBranchV5::Lapse {
            terms: self.parameters.witness_terms(),
            current_debt: self.remaining_debt,
        }
        .build_witness();
        self.add_program_input_from_partial_input(ft, position_input, witness);
        ft.add_output(PartialOutput::new(
            lender_payout_script_pubkey,
            self.get_remaining_collateral(),
            self.parameters.collateral_asset_id,
        ));
    }

    /// Last look round: the position is input 0, the venue's coin at the
    /// last-look script is input 1 (the caller adds it right after). A
    /// partial round (`amount < debt`) continues the position at output 0
    /// and pays the lender `amount` at output 1; the final round pays the
    /// lender at output 0. The venue's own outputs (the released
    /// collateral, the borrower's policy payment) follow.
    pub fn attach_last_look(&mut self, ft: &mut FinalTransaction, position_utxo: UTXO, amount: u64, lender_payout_script_pubkey: Script) {
        assert!(ft.n_inputs() == 0 && ft.n_outputs() == 0, "last look must start the transaction");
        let current_debt = self.remaining_debt;
        assert!(amount > 0 && amount <= current_debt, "invalid last-look amount");
        let new_debt = current_debt - amount;
        let locktime = LockTime::from_height(self.parameters.last_look_height).expect("last-look height is a valid lock height");
        let position_input = PartialInput::new(position_utxo)
            .with_sequence(Sequence::ENABLE_LOCKTIME_NO_RBF)
            .with_locktime(locktime);
        let witness = WitnessBranchV5::LastLook {
            terms: self.parameters.witness_terms(),
            current_debt,
            amount,
        }
        .build_witness();
        self.add_program_input_from_partial_input(ft, position_input, witness);
        self.set_remaining_debt(new_debt);
        if new_debt > 0 {
            self.add_program_output(ft, self.parameters.collateral_asset_id, self.get_remaining_collateral());
        }
        ft.add_output(PartialOutput::new(lender_payout_script_pubkey, amount, self.parameters.cash_asset_id));
    }

    fn set_remaining_debt(&mut self, new_debt: u64) {
        self.remaining_debt = new_debt;
        #[allow(unused_must_use)]
        self.program.set_storage_at(1, debt_slot(new_debt));
    }
}

impl SimplexProgram for SwaptionPositionV5 {
    fn get_program_source_code() -> &'static str {
        SwaptionLendingV5Program::SOURCE
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
    let program = SwaptionLendingV5Program::new(NoArguments);
    let cmr = simplex::simplicityhl::CompiledProgram::new(SwaptionLendingV5Program::SOURCE, Arguments::default(), true)
        .expect("v5 compiles")
        .commit()
        .cmr();
    let _ = program;
    let script = Script::from(cmr.as_ref().to_vec());
    TapLeafHash::from_script(&script, LeafVersion::from_u8(0xbe).expect("simplicity leaf")).to_byte_array()
}

/// The position scriptPubKey from the terms digest and the debt, using
/// nothing but hashes — the check a wallet can do without a compiler.
/// Mirrors `get_script_hash_for_storage` in the covenant and Simplex's
/// three-leaf tree: branch(branch(program, digest), debt).
pub fn position_script_pubkey(program_leaf: [u8; 32], digest: [u8; 32], remaining_debt: u64) -> Script {
    use simplex::simplicityhl::elements::hashes::{Hash as _, HashEngine as _, sha256};
    use simplex::simplicityhl::elements::secp256k1_zkp::{SECP256K1, XOnlyPublicKey};
    use simplex::simplicityhl::elements::taproot::{TapNodeHash, TapTweakHash};
    fn tap_data(v: &[u8; 32]) -> [u8; 32] {
        let tag = sha256::Hash::hash(b"TapData");
        let mut e = sha256::Hash::engine();
        e.input(tag.as_ref());
        e.input(tag.as_ref());
        e.input(v);
        sha256::Hash::from_engine(e).to_byte_array()
    }
    fn branch(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
        let (l, r) = if a <= b { (a, b) } else { (b, a) };
        let tag = sha256::Hash::hash(b"TapBranch/elements");
        let mut e = sha256::Hash::engine();
        e.input(tag.as_ref());
        e.input(tag.as_ref());
        e.input(&l);
        e.input(&r);
        sha256::Hash::from_engine(e).to_byte_array()
    }
    let node = branch(branch(program_leaf, tap_data(&digest)), tap_data(&debt_slot(remaining_debt)));
    let nums = XOnlyPublicKey::from_slice(&[
        0x50, 0x92, 0x9b, 0x74, 0xc1, 0xa0, 0x49, 0x54, 0xb7, 0x8b, 0x4b, 0x60, 0x35, 0xe9, 0x7a, 0x5e, 0x07, 0x8a, 0x5a, 0x0f, 0x28, 0xec, 0x96,
        0xd5, 0x47, 0xbf, 0xee, 0x9a, 0xce, 0x80, 0x3a, 0xc0,
    ])
    .expect("nums");
    let tweak = TapTweakHash::from_key_and_tweak(nums, Some(TapNodeHash::from_byte_array(node)));
    let (output_key, _) = nums.add_tweak(SECP256K1, &tweak.to_scalar()).expect("tweak");
    simplex::simplicityhl::elements::script::Builder::new()
        .push_opcode(simplex::simplicityhl::elements::opcodes::all::OP_PUSHNUM_1)
        .push_slice(&output_key.serialize())
        .into_script()
}

#[cfg(test)]
mod tests {
    use super::*;
    use simplex::simplicityhl::elements::AssetId;

    fn params() -> PositionParametersV5 {
        let asset = |b: u8| AssetId::from_slice(&[b; 32]).unwrap();
        PositionParametersV5 {
            collateral_asset_id: asset(1),
            cash_asset_id: asset(2),
            borrower_nft_asset_id: asset(3),
            lender_nft_asset_id: asset(4),
            lender_payout_script_hash: [9u8; 32],
            borrower_payout_script_hash: [10u8; 32],
            last_look_script_hash: [11u8; 32],
            last_look_height: 3_199_500,
            terms: crate::programs::swaption_lending::PositionTerms {
                collateral_amount: 50_000_000,
                buyback_amount: 3_100_000_000_000,
                expiry_height: 3_200_000,
            },
            network: SimplicityNetwork::LiquidTestnet,
        }
    }

    /// Prints the constant program's tapleaf hash — the value a wallet pins.
    #[test]
    fn print_program_leaf() {
        println!("SWAPTION_LENDING_V5_LEAF={}", hex::encode(tapleaf_hash_of_program()));
        // A vector for the SDK's hash-only reconstruction: the test params
        // at full debt and at debt 7.
        let p = params();
        println!("V5_VECTOR_DIGEST={}", hex::encode(p.digest()));
        for debt in [3_100_000_000_000u64, 7] {
            println!("V5_VECTOR_SCRIPT_{debt}={}", hex::encode(SwaptionPositionV5::new(p, debt).get_script_pubkey().as_bytes()));
        }
    }

    /// The hash-only reconstruction must land on Simplex's script for the
    /// same terms and state — this is what the wallet relies on.
    #[test]
    fn hash_only_script_matches_simplex() {
        let leaf = tapleaf_hash_of_program();
        let p = params();
        for debt in [3_100_000_000_000u64, 7, 0] {
            let position = SwaptionPositionV5::new(p, debt);
            assert_eq!(position_script_pubkey(leaf, p.digest(), debt), position.get_script_pubkey(), "debt {debt}");
        }
        // Different terms, different script.
        let mut q = params();
        q.terms.buyback_amount += 1;
        assert_ne!(position_script_pubkey(leaf, q.digest(), 7), position_script_pubkey(leaf, p.digest(), 7));
    }
}
