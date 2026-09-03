use simplex::either::Either::{Left, Right};

use crate::artifacts::swaption_lending_v3::derived_swaption_lending_v3::SwaptionLendingV3Witness;
use crate::programs::swaption_lending_v3::WitnessTerms;

#[derive(Debug, Clone, Copy)]
pub enum WitnessBranchV3 {
    Exercise { terms: WitnessTerms, current_debt: u64, amount: u64 },
    Lapse { terms: WitnessTerms, current_debt: u64 },
}

impl WitnessBranchV3 {
    pub fn build_witness(&self) -> Box<SwaptionLendingV3Witness> {
        let path = match self {
            WitnessBranchV3::Exercise {
                terms,
                current_debt,
                amount,
            } => Left((*terms, *current_debt, *amount)),
            WitnessBranchV3::Lapse { terms, current_debt } => Right((*terms, *current_debt)),
        };
        Box::new(SwaptionLendingV3Witness { path })
    }
}
