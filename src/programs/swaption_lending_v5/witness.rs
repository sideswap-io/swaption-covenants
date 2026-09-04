use simplex::either::Either::{Left, Right};

use crate::artifacts::swaption_lending_v5::derived_swaption_lending_v5::SwaptionLendingV5Witness;
use crate::programs::swaption_lending_v5::WitnessTerms;

#[derive(Debug, Clone, Copy)]
pub enum WitnessBranchV5 {
    Exercise { terms: WitnessTerms, current_debt: u64, amount: u64 },
    Lapse { terms: WitnessTerms, current_debt: u64 },
    LastLook { terms: WitnessTerms, current_debt: u64, amount: u64 },
}

impl WitnessBranchV5 {
    pub fn build_witness(&self) -> Box<SwaptionLendingV5Witness> {
        let path = match self {
            WitnessBranchV5::Exercise {
                terms,
                current_debt,
                amount,
            } => Left((*terms, *current_debt, *amount)),
            WitnessBranchV5::Lapse { terms, current_debt } => Right(Left((*terms, *current_debt))),
            WitnessBranchV5::LastLook { terms, current_debt, amount } => Right(Right((*terms, *current_debt, *amount))),
        };
        Box::new(SwaptionLendingV5Witness { path })
    }
}
