/// The economic terms of one position, all in satoshi units of the two assets.
///
/// `collateral_amount` of the collateral asset is held by the covenant;
/// `buyback_amount` of the cash asset buys all of it back (the initial debt);
/// `expiry_height` is the Liquid block height after which the lender may lapse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionTerms {
    pub collateral_amount: u64,
    pub buyback_amount: u64,
    pub expiry_height: u32,
}

impl PositionTerms {
    /// Collateral unlocked once `paid_debt` of `buyback_amount` has been paid
    /// (floor of the proportional share). Mirrors `get_unlocked_collateral`.
    pub fn unlocked_collateral(&self, paid_debt: u64) -> u64 {
        assert!(paid_debt <= self.buyback_amount, "paid more than the buyback amount");
        let product = u128::from(paid_debt) * u128::from(self.collateral_amount);
        (product / u128::from(self.buyback_amount)) as u64
    }

    /// Collateral the covenant must hold while `remaining_debt` is still owed.
    /// Derived from cumulative paid debt, so a sequence of partial exercises
    /// never drifts. Mirrors `get_remaining_collateral`.
    pub fn remaining_collateral(&self, remaining_debt: u64) -> u64 {
        assert!(remaining_debt <= self.buyback_amount, "debt above the buyback amount");
        let paid = self.buyback_amount - remaining_debt;
        self.collateral_amount - self.unlocked_collateral(paid)
    }

    /// Collateral released by paying `amount` while `current_debt` is owed.
    pub fn released_by(&self, current_debt: u64, amount: u64) -> u64 {
        assert!(amount <= current_debt, "amount above the current debt");
        self.remaining_collateral(current_debt) - self.remaining_collateral(current_debt - amount)
    }

    /// The fee embedded in the terms, in cash units, given what the lender paid.
    pub fn fee(&self, sale_amount: u64) -> u64 {
        self.buyback_amount.saturating_sub(sale_amount)
    }
}

#[cfg(test)]
mod tests {
    use super::PositionTerms;

    fn terms() -> PositionTerms {
        PositionTerms {
            collateral_amount: 100_000_000, // 1 BTC
            buyback_amount: 62_000_000_00,  // 62,000 USDt (8 decimals)
            expiry_height: 3_000_000,
        }
    }

    #[test]
    fn full_debt_holds_all_collateral_and_zero_debt_holds_none() {
        let t = terms();
        assert_eq!(t.remaining_collateral(t.buyback_amount), t.collateral_amount);
        assert_eq!(t.remaining_collateral(0), 0);
    }

    #[test]
    fn remaining_collateral_is_monotone_in_debt() {
        let t = terms();
        let mut prev = t.remaining_collateral(t.buyback_amount);
        for step in 1..=100u64 {
            let debt = t.buyback_amount - t.buyback_amount * step / 100;
            let now = t.remaining_collateral(debt);
            assert!(now <= prev);
            prev = now;
        }
    }

    #[test]
    fn many_odd_partials_end_at_zero_without_drift() {
        // The reference contract subtracts per-step floors and fails on the
        // third odd partial; ours derives from cumulative paid debt.
        let t = PositionTerms {
            collateral_amount: 1_000_003,
            buyback_amount: 7_777_777,
            expiry_height: 1,
        };
        let mut debt = t.buyback_amount;
        let mut held = t.collateral_amount;
        let steps = [1_234_567u64, 999_999, 3, 2_000_001, 1_111_111];
        for amount in steps {
            let released = t.released_by(debt, amount);
            held -= released;
            debt -= amount;
            assert_eq!(held, t.remaining_collateral(debt));
        }
        let released = t.released_by(debt, debt);
        held -= released;
        assert_eq!(held, 0);
    }

    #[test]
    fn released_share_never_exceeds_paid_share() {
        let t = terms();
        let amount = t.buyback_amount / 3;
        let released = t.released_by(t.buyback_amount, amount);
        // floor(1/3 of collateral)
        assert_eq!(released, t.collateral_amount / 3);
    }
}
