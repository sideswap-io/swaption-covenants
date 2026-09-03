//! Swaption lending v2 — one constant program for every position; the
//! terms live in taproot storage slot 0 (their SHA-256 digest) and travel
//! in the witness. Rust wrapper around `simf/swaption_lending_v2.simf`.

mod core;
mod params;
mod witness;

pub use core::{SwaptionPositionV2, position_script_pubkey, tapleaf_hash_of_program};
pub use params::{PositionParametersV2, WitnessTerms};
pub use witness::WitnessBranchV2;

pub use crate::programs::swaption_lending::{PositionTerms, script_hash};
