//! Swaption lending v5 — v4 plus a PARTIAL last look: the venue may take a
//! position off in rounds, paying the lender `amount` per round. The
//! terms live in taproot storage slot 0 (their SHA-256 digest) and travel
//! in the witness. Rust wrapper around `simf/swaption_lending_v5.simf`.

mod core;
mod params;
mod witness;

pub use core::{SwaptionPositionV5, position_script_pubkey, tapleaf_hash_of_program};
pub use params::{PositionParametersV5, WitnessTerms};
pub use witness::WitnessBranchV5;

pub use crate::programs::swaption_lending::{PositionTerms, script_hash};
