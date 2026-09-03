//! Swaption lending v3 — v2 with a permissionless lapse that pays the
//! lender's payout script (normally the token's claim script). The
//! terms live in taproot storage slot 0 (their SHA-256 digest) and travel
//! in the witness. Rust wrapper around `simf/swaption_lending_v3.simf`.

mod core;
mod params;
mod witness;

pub use core::{SwaptionPositionV3, position_script_pubkey, tapleaf_hash_of_program};
pub use params::{PositionParametersV3, WitnessTerms};
pub use witness::WitnessBranchV3;

pub use crate::programs::swaption_lending::{PositionTerms, script_hash};
