//! Swaption lending v1 — a sale with a buyback right (fully collateralised
//! American call). Rust wrapper around `simf/swaption_lending.simf`.
//!
//! Forked from the vendored Blockstream `lending` module; see
//! `docs/LENDING-DESIGN.md` and the Dropbox spec (Swaption/Lending/06).

mod core;
mod error;
mod metadata;
mod params;
mod terms;
mod witness;

pub use core::{
    FILL_BORROWER_NFT_OUTPUT_INDEX, FILL_LENDER_NFT_OUTPUT_INDEX, FILL_METADATA_OUTPUT_INDEX, FILL_POSITION_OUTPUT_INDEX, SwaptionPosition,
    SwaptionPositionStorage, script_hash,
};
pub use error::SwaptionPositionError;
pub use metadata::SwaptionPositionCreationMetadata;
pub use params::SwaptionPositionParameters;
pub use terms::PositionTerms;
pub use witness::SwaptionPositionWitnessBranch;
