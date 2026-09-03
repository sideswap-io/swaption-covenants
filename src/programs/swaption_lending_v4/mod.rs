//! Swaption lending v4 — v3 (permissionless lapse to the lender's payout
//! script) plus the venue's LAST LOOK before expiry. The
//! terms live in taproot storage slot 0 (their SHA-256 digest) and travel
//! in the witness. Rust wrapper around `simf/swaption_lending_v4.simf`.

mod core;
mod params;
mod witness;

pub use core::{SwaptionPositionV4, position_script_pubkey, tapleaf_hash_of_program};
pub use params::{PositionParametersV4, WitnessTerms};
pub use witness::WitnessBranchV4;

pub use crate::programs::swaption_lending::{PositionTerms, script_hash};
