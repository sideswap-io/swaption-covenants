use simplex::either::Either::{Left, Right};

use crate::artifacts::swaption_lending::derived_swaption_lending::SwaptionLendingWitness;

#[derive(Debug, Clone, Copy)]
pub enum SwaptionPositionWitnessBranch {
    /// Borrower pays `amount` (≤ `current_debt`) of cash to the lender script.
    Exercise { current_debt: u64, amount: u64 },
    /// After expiry the lender takes what is left.
    Lapse { current_debt: u64 },
}

impl SwaptionPositionWitnessBranch {
    pub fn build_witness(&self) -> Box<SwaptionLendingWitness> {
        let path = match self {
            SwaptionPositionWitnessBranch::Exercise {
                current_debt,
                amount,
            } => Left((*current_debt, *amount)),
            SwaptionPositionWitnessBranch::Lapse { current_debt } => Right(*current_debt),
        };

        Box::new(SwaptionLendingWitness { path })
    }
}
