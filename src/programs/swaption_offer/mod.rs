//! Swaption offer v1 — a lender's cash escrowed in a covenant so that a
//! fill needs only the borrower's signature. Rust wrapper around
//! `simf/swaption_offer.simf`. The terms travel in the witness and are bound
//! through storage slot 0 (their SHA-256 digest); slot 1 is the remaining
//! cash. A fill creates a v5 position whose `lender_nft` is the lender token
//! and whose payout is that token's claim script. Brief: Dropbox
//! `Swaption/Lending/13 Escrowed lend offers — design brief and fee decisions`.

mod core;
mod params;
mod witness;

pub use core::{SwaptionOffer, offer_script_pubkey, tapleaf_hash_of_program};
pub use params::{FillOutcome, OFFER_ROWS, OfferParameters, OfferRow, SCALE, WitnessOfferTerms, WitnessRow, WitnessRows};
pub use witness::{WitnessBranchOffer, WitnessFillArgs};
