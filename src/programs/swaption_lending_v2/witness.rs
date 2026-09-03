use simplex::either::Either::{Left, Right};

use crate::artifacts::swaption_lending_v2::derived_swaption_lending_v2::SwaptionLendingV2Witness;
use crate::programs::swaption_lending_v2::WitnessTerms;

#[derive(Debug, Clone, Copy)]
pub enum WitnessBranchV2 {
    Exercise { terms: WitnessTerms, current_debt: u64, amount: u64 },
    Lapse { terms: WitnessTerms, current_debt: u64 },
}

impl WitnessBranchV2 {
    pub fn build_witness(&self) -> Box<SwaptionLendingV2Witness> {
        let path = match self {
            WitnessBranchV2::Exercise {
                terms,
                current_debt,
                amount,
            } => Left((*terms, *current_debt, *amount)),
            WitnessBranchV2::Lapse { terms, current_debt } => Right((*terms, *current_debt)),
        };
        Box::new(SwaptionLendingV2Witness { path })
    }
}
