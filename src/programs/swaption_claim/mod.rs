//! Swaption claim — a token-gated coin: spendable by whoever spends one
//! unit of the lender token as input 0. Rust wrapper around
//! `simf/swaption_claim.simf`. Positions (v3) pay the lender here so the
//! lender's claim travels with the token.

mod core;

pub use core::{SwaptionClaim, claim_script_pubkey, tapleaf_hash_of_program};
