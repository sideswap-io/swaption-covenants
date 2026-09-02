use simplex::simplicityhl::elements::{Txid, hashes::FromSliceError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SwaptionPositionError {
    #[error("transaction {0} is not a Swaption position creation")]
    NotAPositionCreationTx(Txid),
    #[error("creation metadata has length {actual}, expected {expected}")]
    InvalidCreationMetadataLength { expected: usize, actual: usize },
    #[error("creation metadata carries an unknown program id")]
    UnknownProgramId,
    #[error("asset id: {0}")]
    AssetId(#[from] FromSliceError),
    #[error("amount or asset in output {0} is not explicit")]
    NotExplicit(usize),
}
