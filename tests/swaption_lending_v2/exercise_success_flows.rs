use lending_contracts::programs::swaption_lending::PositionTerms;

use super::setup::{exercise, fill_position, lender_cash_balance, position_utxo};

fn default_terms(context: &simplex::TestContext) -> anyhow::Result<PositionTerms> {
    let current_height = context.get_default_provider().fetch_tip_height()?;
    Ok(PositionTerms {
        collateral_amount: 3000,
        buyback_amount: 11_000,
        expiry_height: current_height + 60,
    })
}

#[simplex::test]
fn full_exercise_pays_lender_and_closes_position(
    context: simplex::TestContext,
) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let terms = default_terms(&context)?;
    let mut fill = fill_position(&context, &lender, terms)?;

    let before = lender_cash_balance(&lender, fill.parameters.cash_asset_id)?;

    exercise(&context, &mut fill, terms.buyback_amount)?;

    assert!(fill.position.is_settled());
    assert!(position_utxo(&context, &fill.position)?.is_none());
    assert_eq!(
        lender_cash_balance(&lender, fill.parameters.cash_asset_id)?,
        before + terms.buyback_amount
    );

    Ok(())
}

#[simplex::test]
fn three_odd_partial_exercises_then_full_release_everything(
    context: simplex::TestContext,
) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let terms = PositionTerms {
        collateral_amount: 3001,
        buyback_amount: 10_007,
        expiry_height: context.get_default_provider().fetch_tip_height()? + 100,
    };
    let mut fill = fill_position(&context, &lender, terms)?;
    let before = lender_cash_balance(&lender, fill.parameters.cash_asset_id)?;

    // The reference contract would fail on the third of these (per-step floor
    // drift); ours derives remaining collateral from cumulative paid debt.
    for amount in [1_234u64, 999, 3] {
        exercise(&context, &mut fill, amount)?;
        let utxo = position_utxo(&context, &fill.position)?.expect("position continues");
        assert_eq!(
            utxo.explicit_amount(),
            terms.remaining_collateral(fill.position.get_remaining_debt())
        );
    }

    let rest = fill.position.get_remaining_debt();
    exercise(&context, &mut fill, rest)?;

    assert!(position_utxo(&context, &fill.position)?.is_none());
    assert_eq!(
        lender_cash_balance(&lender, fill.parameters.cash_asset_id)?,
        before + terms.buyback_amount
    );

    Ok(())
}
