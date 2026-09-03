use simplex::program::{ArgumentsTrait, Program};
use simplex::provider::SimplicityNetwork;
use simplex::simplicityhl::Arguments;
use simplex::simplicityhl::elements::{AssetId, Script};
use simplex::transaction::{FinalTransaction, UTXO};

use crate::artifacts::swaption_claim::SwaptionClaimProgram;
use crate::artifacts::swaption_claim::derived_swaption_claim::SwaptionClaimWitness;
use crate::programs::program::SimplexProgram;

/// The program takes no arguments; the token id lives in storage slot 0.
#[derive(Clone)]
struct NoArguments;

impl ArgumentsTrait for NoArguments {
    fn build_arguments(&self) -> Arguments {
        Arguments::default()
    }
}

pub struct SwaptionClaim {
    program: SwaptionClaimProgram,
    lender_nft_asset_id: AssetId,
    network: SimplicityNetwork,
}

impl SwaptionClaim {
    pub fn new(lender_nft_asset_id: AssetId, network: SimplicityNetwork) -> Self {
        let mut program = SwaptionClaimProgram::new(NoArguments).with_storage_capacity(1);
        #[allow(unused_must_use)]
        program.set_storage_at(0, lender_nft_asset_id.into_inner().0);
        Self {
            program,
            lender_nft_asset_id,
            network,
        }
    }

    pub fn lender_nft_asset_id(&self) -> AssetId {
        self.lender_nft_asset_id
    }

    /// Spend one claim coin. The transaction's input 0 must already be one
    /// unit of the lender token (the holder signs it); outputs are free.
    pub fn attach_withdraw(&self, ft: &mut FinalTransaction, claim_utxo: UTXO) {
        assert!(ft.n_inputs() >= 1, "the lender token must be input 0 before any claim input");
        let witness = Box::new(SwaptionClaimWitness {
            lender_nft: self.lender_nft_asset_id.into_inner().0,
        });
        self.add_program_input(ft, claim_utxo, witness);
    }
}

impl SimplexProgram for SwaptionClaim {
    fn get_program_source_code() -> &'static str {
        SwaptionClaimProgram::SOURCE
    }

    fn get_program(&self) -> &Program {
        self.program.as_ref()
    }

    fn get_network(&self) -> &SimplicityNetwork {
        &self.network
    }
}

/// The constant program's tapleaf hash — what a wallet pins.
pub fn tapleaf_hash_of_program() -> [u8; 32] {
    use simplex::simplicityhl::elements::hashes::Hash as _;
    use simplex::simplicityhl::elements::taproot::{LeafVersion, TapLeafHash};
    let cmr = simplex::simplicityhl::CompiledProgram::new(SwaptionClaimProgram::SOURCE, Arguments::default(), true)
        .expect("claim compiles")
        .commit()
        .cmr();
    let script = Script::from(cmr.as_ref().to_vec());
    TapLeafHash::from_script(&script, LeafVersion::from_u8(0xbe).expect("simplicity leaf")).to_byte_array()
}

/// The claim scriptPubKey of a lender token from hashes alone: Simplex's
/// two-leaf tree branch(program, tapdata(token)) under the BIP-341 NUMS key.
pub fn claim_script_pubkey(program_leaf: [u8; 32], lender_nft: [u8; 32]) -> Script {
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
    let node = branch(program_leaf, tap_data(&lender_nft));
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

    #[test]
    fn print_program_leaf() {
        println!("SWAPTION_CLAIM_LEAF={}", hex::encode(tapleaf_hash_of_program()));
        let nft = AssetId::from_slice(&[4u8; 32]).unwrap();
        println!(
            "CLAIM_VECTOR_SCRIPT_04={}",
            hex::encode(SwaptionClaim::new(nft, SimplicityNetwork::LiquidTestnet).get_script_pubkey().as_bytes())
        );
    }

    #[test]
    fn hash_only_script_matches_simplex() {
        let leaf = tapleaf_hash_of_program();
        for b in [4u8, 5, 0xff] {
            let nft = AssetId::from_slice(&[b; 32]).unwrap();
            let claim = SwaptionClaim::new(nft, SimplicityNetwork::LiquidTestnet);
            assert_eq!(claim_script_pubkey(leaf, nft.into_inner().0), claim.get_script_pubkey(), "token {b}");
        }
    }
}
