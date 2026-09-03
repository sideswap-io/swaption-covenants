//! Last look (v4): from `last_look_height` the venue key may exercise on
//! the borrower's behalf — it pays the lender the whole remaining debt and
//! takes the collateral. Before the window it is non-final; without the
//! venue's coin at input 1, or paying the lender short, it is refused.

use simplex::simplicityhl::elements::{LockTime, Sequence};
use simplex::transaction::{FinalTransaction, PartialInput, PartialOutput, RequiredSignature};

use lending_contracts::programs::program::SimplexProgram;
use lending_contracts::programs::swaption_lending_v4::{PositionTerms, WitnessBranchV4};

use super::setup::{Fill, LAST_LOOK_BLOCKS, claim_balance, exercise, fill_position, fund_lender, position_utxo, wallet_balance};

fn default_fill(context: &simplex::TestContext, lender: &simplex::signer::Signer) -> anyhow::Result<Fill> {
    let current_height = context.get_default_provider().fetch_tip_height()?;
    fill_position(
        context,
        lender,
        PositionTerms {
            collateral_amount: 3000,
            buyback_amount: 11_000,
            expiry_height: current_height + 40,
        },
    )
}

/// The honest last look: position in, venue coin in, lender paid in full,
/// collateral to the venue, a policy payment to the borrower.
fn last_look_tx(context: &simplex::TestContext, fill: &Fill, borrower_payment: u64) -> anyhow::Result<FinalTransaction> {
    let params = fill.parameters;
    let utxo = position_utxo(context, &fill.position)?.expect("position utxo");
    let venue_lbtc = fill.venue.get_utxos_asset(params.collateral_asset_id)?[0].clone();
    let debt = fill.position.get_remaining_debt();
    let cash_utxo = fill
        .venue
        .get_utxos_filter(&|u| u.explicit_asset() == params.cash_asset_id && u.explicit_amount() >= debt + borrower_payment, &|_| true)?[0]
        .clone();
    let cash_amount = cash_utxo.explicit_amount();

    let mut ft = FinalTransaction::new();
    fill.position.attach_last_look(&mut ft, utxo, fill.lender_script.clone());
    ft.add_input(PartialInput::new(venue_lbtc.clone()), RequiredSignature::NativeEcdsa);
    ft.add_input(PartialInput::new(cash_utxo), RequiredSignature::NativeEcdsa);
    // The venue takes the collateral and its fee coin back.
    ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), fill.position.get_remaining_collateral(), params.collateral_asset_id));
    ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), venue_lbtc.explicit_amount(), params.collateral_asset_id));
    if borrower_payment > 0 {
        ft.add_output(PartialOutput::new(context.get_default_signer().get_address().script_pubkey(), borrower_payment, params.cash_asset_id));
    }
    let change = cash_amount - debt - borrower_payment;
    if change > 0 {
        ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), change, params.cash_asset_id));
    }
    Ok(ft)
}

fn mine_into_window(context: &simplex::TestContext, fill: &Fill) -> anyhow::Result<()> {
    context.get_network_utils().mine_until_height(u64::from(fill.parameters.last_look_height) + 1)?;
    Ok(())
}

#[simplex::test]
fn venue_last_look_pays_the_lender_and_the_borrower(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    let borrower = context.get_default_signer();
    let borrower_cash_before = wallet_balance(borrower, params.cash_asset_id)?;

    mine_into_window(&context, &fill)?;
    assert!(context.get_default_provider().fetch_tip_height()? < params.terms.expiry_height);

    let ft = last_look_tx(&context, &fill, 500)?;
    fill.venue.broadcast(&ft)?.wait()?;

    assert!(position_utxo(&context, &fill.position)?.is_none());
    assert_eq!(claim_balance(&context, &fill, params.cash_asset_id)?, params.terms.buyback_amount, "lender paid in full");
    assert_eq!(wallet_balance(borrower, params.cash_asset_id)? - borrower_cash_before, 500, "borrower got the policy payment");
    assert!(wallet_balance(&fill.venue, params.collateral_asset_id)? >= params.terms.collateral_amount, "venue holds the collateral");
    Ok(())
}

#[simplex::test]
fn last_look_after_a_partial_pays_what_is_left(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    let paid = params.terms.buyback_amount / 4;
    exercise(&context, &mut fill, paid)?;

    mine_into_window(&context, &fill)?;
    let ft = last_look_tx(&context, &fill, 0)?;
    fill.venue.broadcast(&ft)?.wait()?;

    assert!(position_utxo(&context, &fill.position)?.is_none());
    assert_eq!(claim_balance(&context, &fill, params.cash_asset_id)?, params.terms.buyback_amount, "quarter from the borrower, the rest from the venue");
    Ok(())
}

#[simplex::test]
fn last_look_fails_before_the_window(context: simplex::TestContext) -> anyhow::Result<()> {
    let provider = context.get_default_provider();
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    assert!(provider.fetch_tip_height()? < fill.parameters.last_look_height);

    let ft = last_look_tx(&context, &fill, 0)?;
    let refused = match fill.venue.finalize(&ft) {
        Err(_) => true,
        Ok((tx, _)) => provider.broadcast_transaction(&tx).is_err(),
    };
    assert!(refused, "last look before the window must be refused");
    assert!(position_utxo(&context, &fill.position)?.is_some());
    Ok(())
}

#[simplex::test]
fn last_look_fails_without_the_venue_coin(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    // A stranger with cash tries the venue's branch with its own coin at input 1.
    let stranger = context.random_signer();
    fund_lender(&context, &stranger, params.cash_asset_id, params.terms.buyback_amount + 1)?;
    mine_into_window(&context, &fill)?;

    let utxo = position_utxo(&context, &fill.position)?.expect("position utxo");
    let lbtc = stranger.get_utxos_asset(params.collateral_asset_id)?[0].clone();
    let cash = stranger.get_utxos_asset(params.cash_asset_id)?[0].clone();
    let mut ft = FinalTransaction::new();
    fill.position.attach_last_look(&mut ft, utxo, fill.lender_script.clone());
    ft.add_input(PartialInput::new(lbtc.clone()), RequiredSignature::NativeEcdsa);
    ft.add_input(PartialInput::new(cash.clone()), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(stranger.get_address().script_pubkey(), params.terms.collateral_amount, params.collateral_asset_id));
    ft.add_output(PartialOutput::new(stranger.get_address().script_pubkey(), lbtc.explicit_amount(), params.collateral_asset_id));
    let change = cash.explicit_amount() - params.terms.buyback_amount;
    if change > 0 {
        ft.add_output(PartialOutput::new(stranger.get_address().script_pubkey(), change, params.cash_asset_id));
    }
    assert!(stranger.finalize(&ft).is_err(), "only the venue key may take the last look");
    assert!(position_utxo(&context, &fill.position)?.is_some());
    Ok(())
}

#[simplex::test]
fn last_look_fails_when_the_lender_is_paid_short(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    mine_into_window(&context, &fill)?;

    let utxo = position_utxo(&context, &fill.position)?.expect("position utxo");
    let venue_lbtc = fill.venue.get_utxos_asset(params.collateral_asset_id)?[0].clone();
    let cash = fill.venue.get_utxos_asset(params.cash_asset_id)?[0].clone();
    let debt = fill.position.get_remaining_debt();

    let mut ft = FinalTransaction::new();
    let locktime = LockTime::from_height(params.last_look_height)?;
    let input = PartialInput::new(utxo)
        .with_sequence(Sequence::ENABLE_LOCKTIME_NO_RBF)
        .with_locktime(locktime);
    fill.position.add_program_input_from_partial_input(
        &mut ft,
        input,
        WitnessBranchV4::LastLook {
            terms: params.witness_terms(),
            current_debt: debt,
        }
        .build_witness(),
    );
    ft.add_output(PartialOutput::new(fill.lender_script.clone(), debt - 1, params.cash_asset_id));
    ft.add_input(PartialInput::new(venue_lbtc.clone()), RequiredSignature::NativeEcdsa);
    ft.add_input(PartialInput::new(cash.clone()), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), params.terms.collateral_amount, params.collateral_asset_id));
    ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), venue_lbtc.explicit_amount(), params.collateral_asset_id));
    ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), cash.explicit_amount() - (debt - 1), params.cash_asset_id));
    assert!(fill.venue.finalize(&ft).is_err(), "the lender must get the whole remaining debt");
    Ok(())
}

#[simplex::test]
fn borrower_can_still_buy_back_inside_the_window(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    mine_into_window(&context, &fill)?;
    exercise(&context, &mut fill, params.terms.buyback_amount)?;
    assert!(fill.position.is_settled());
    assert!(position_utxo(&context, &fill.position)?.is_none());
    let _ = LAST_LOOK_BLOCKS;
    Ok(())
}
