use simplex::either::Either::{Left, Right};

use crate::artifacts::swaption_lending_v4::derived_swaption_lending_v4::SwaptionLendingV4Witness;
use crate::programs::swaption_lending_v4::WitnessTerms;

#[derive(Debug, Clone, Copy)]
pub enum WitnessBranchV4 {
    Exercise { terms: WitnessTerms, current_debt: u64, amount: u64 },
    Lapse { terms: WitnessTerms, current_debt: u64 },
    LastLook { terms: WitnessTerms, current_debt: u64 },
}

impl WitnessBranchV4 {
    pub fn build_witness(&self) -> Box<SwaptionLendingV4Witness> {
        let path = match self {
            WitnessBranchV4::Exercise {
                terms,
                current_debt,
                amount,
            } => Left((*terms, *current_debt, *amount)),
            WitnessBranchV4::Lapse { terms, current_debt } => Right(Left((*terms, *current_debt))),
            WitnessBranchV4::LastLook { terms, current_debt } => Right(Right((*terms, *current_debt))),
        };
        Box::new(SwaptionLendingV4Witness { path })
    }
}
