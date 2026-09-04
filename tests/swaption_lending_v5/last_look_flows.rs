//! Last look (v5): from `last_look_height` the venue key may exercise on
//! the borrower's behalf in ROUNDS — each round pays the lender `amount`
//! and, while debt is left, continues the position with the proportional
//! collateral. Before the window it is non-final; without the venue's coin
//! at input 1, paying the lender short, or continuing with too little
//! collateral, it is refused.

use simplex::simplicityhl::elements::{LockTime, Sequence};
use simplex::transaction::{FinalTransaction, PartialInput, PartialOutput, RequiredSignature};

use lending_contracts::programs::program::SimplexProgram;
use lending_contracts::programs::swaption_lending_v5::{PositionTerms, WitnessBranchV5};

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

/// One honest last-look round of `amount`: position in, venue coin in,
/// lender paid `amount`, the released collateral to the venue, a policy
/// payment to the borrower. Advances `fill.position` to the new state.
fn last_look_round(context: &simplex::TestContext, fill: &mut Fill, amount: u64, borrower_payment: u64) -> anyhow::Result<FinalTransaction> {
    let params = fill.parameters;
    let utxo = position_utxo(context, &fill.position)?.expect("position utxo");
    // After a round the venue's coins are fresh outputs of its own
    // transaction; give the index a moment, and pick the largest L-BTC
    // coin (a released chunk of collateral may be tiny) and a cash coin
    // that covers this round.
    let (venue_lbtc, cash_utxo) = {
        let mut found = None;
        for _ in 0..10 {
            let mut lbtc: Vec<_> = fill.venue.get_utxos_asset(params.collateral_asset_id)?;
            lbtc.sort_by_key(|u| std::cmp::Reverse(u.explicit_amount()));
            let cash = fill
                .venue
                .get_utxos_filter(&|u| u.explicit_asset() == params.cash_asset_id && u.explicit_amount() >= amount + borrower_payment, &|_| true)?;
            if let (Some(l), Some(c)) = (lbtc.first(), cash.first())
                && l.explicit_amount() > 1000
            {
                found = Some((l.clone(), c.clone()));
                break;
            }
            let tip = context.get_default_provider().fetch_tip_height()?;
            context.get_network_utils().mine_until_height(u64::from(tip) + 1)?;
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        found.expect("venue coins for the round")
    };
    let cash_amount = cash_utxo.explicit_amount();
    let collateral_before = fill.position.get_remaining_collateral();

    let mut ft = FinalTransaction::new();
    fill.position.attach_last_look(&mut ft, utxo, amount, fill.lender_script.clone());
    let released = collateral_before - fill.position.get_remaining_collateral();
    ft.add_input(PartialInput::new(venue_lbtc.clone()), RequiredSignature::NativeEcdsa);
    ft.add_input(PartialInput::new(cash_utxo), RequiredSignature::NativeEcdsa);
    // The venue takes the released collateral and its fee coin back.
    ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), released, params.collateral_asset_id));
    ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), venue_lbtc.explicit_amount() - 1000, params.collateral_asset_id));
    if borrower_payment > 0 {
        ft.add_output(PartialOutput::new(context.get_default_signer().get_address().script_pubkey(), borrower_payment, params.cash_asset_id));
    }
    let change = cash_amount - amount - borrower_payment;
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
fn venue_last_look_in_full_pays_the_lender_and_the_borrower(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    let borrower = context.get_default_signer();
    let borrower_cash_before = wallet_balance(borrower, params.cash_asset_id)?;

    mine_into_window(&context, &fill)?;
    assert!(context.get_default_provider().fetch_tip_height()? < params.terms.expiry_height);

    let debt = fill.position.get_remaining_debt();
    let ft = last_look_round(&context, &mut fill, debt, 500)?;
    fill.venue.broadcast(&ft)?.wait()?;

    assert!(fill.position.is_settled());
    assert!(position_utxo(&context, &fill.position)?.is_none());
    assert_eq!(claim_balance(&context, &fill, params.cash_asset_id)?, params.terms.buyback_amount, "lender paid in full");
    assert_eq!(wallet_balance(borrower, params.cash_asset_id)? - borrower_cash_before, 500, "borrower got the policy payment");
    assert!(wallet_balance(&fill.venue, params.collateral_asset_id)? >= params.terms.collateral_amount, "venue holds the collateral");
    Ok(())
}

/// Three rounds — 4000, 4000, 3000 of an 11 000 debt — each continuing
/// the position with the proportional collateral, the last closing it.
#[simplex::test]
fn venue_last_look_in_rounds(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    mine_into_window(&context, &fill)?;

    let mut venue_collateral = 0u64;
    for (i, amount) in [4000u64, 4000, 3000].into_iter().enumerate() {
        let before = fill.position.get_remaining_collateral();
        let ft = last_look_round(&context, &mut fill, amount, 100)?;
        fill.venue.broadcast(&ft)?.wait()?;
        venue_collateral += before - fill.position.get_remaining_collateral();
        if i < 2 {
            assert!(position_utxo(&context, &fill.position)?.is_some(), "round {i}: position continues");
            assert_eq!(fill.position.get_remaining_collateral(), params.terms.remaining_collateral(fill.position.get_remaining_debt()));
        }
    }
    assert!(fill.position.is_settled());
    assert!(position_utxo(&context, &fill.position)?.is_none());
    assert_eq!(claim_balance(&context, &fill, params.cash_asset_id)?, params.terms.buyback_amount, "lender paid in full over three rounds");
    assert_eq!(venue_collateral, params.terms.collateral_amount, "the venue ended up with all the collateral");
    Ok(())
}

/// A partial round, then the borrower buys the rest back itself.
#[simplex::test]
fn borrower_can_finish_after_a_partial_last_look(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    mine_into_window(&context, &fill)?;

    let ft = last_look_round(&context, &mut fill, 5000, 0)?;
    fill.venue.broadcast(&ft)?.wait()?;
    assert_eq!(fill.position.get_remaining_debt(), 6000);

    exercise(&context, &mut fill, 6000)?;
    assert!(fill.position.is_settled());
    assert!(position_utxo(&context, &fill.position)?.is_none());
    assert_eq!(claim_balance(&context, &fill, params.cash_asset_id)?, params.terms.buyback_amount);
    Ok(())
}

#[simplex::test]
fn last_look_after_a_borrower_partial_pays_what_is_left(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    let paid = params.terms.buyback_amount / 4;
    exercise(&context, &mut fill, paid)?;

    mine_into_window(&context, &fill)?;
    let debt = fill.position.get_remaining_debt();
    let ft = last_look_round(&context, &mut fill, debt, 0)?;
    fill.venue.broadcast(&ft)?.wait()?;

    assert!(position_utxo(&context, &fill.position)?.is_none());
    assert_eq!(claim_balance(&context, &fill, params.cash_asset_id)?, params.terms.buyback_amount, "quarter from the borrower, the rest from the venue");
    Ok(())
}

#[simplex::test]
fn last_look_fails_before_the_window(context: simplex::TestContext) -> anyhow::Result<()> {
    let provider = context.get_default_provider();
    let lender = context.random_signer();
    let mut fill = default_fill(&context, &lender)?;
    assert!(provider.fetch_tip_height()? < fill.parameters.last_look_height);

    let debt = fill.position.get_remaining_debt();
    let ft = last_look_round(&context, &mut fill, debt, 0)?;
    let refused = match fill.venue.finalize(&ft) {
        Err(_) => true,
        Ok((tx, _)) => provider.broadcast_transaction(&tx).is_err(),
    };
    assert!(refused, "last look before the window must be refused");
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
    let debt = fill.position.get_remaining_debt();
    let mut ft = FinalTransaction::new();
    // A scratch copy: the attempt must not advance the real position's state.
    let mut probe = lending_contracts::programs::swaption_lending_v5::SwaptionPositionV5::new(params, debt);
    probe.attach_last_look(&mut ft, utxo, debt, fill.lender_script.clone());
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

/// A partial round whose continuing position keeps too little collateral
/// (the venue tries to take more than the round released) is refused.
#[simplex::test]
fn partial_last_look_fails_when_it_over_releases(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    mine_into_window(&context, &fill)?;

    let utxo = position_utxo(&context, &fill.position)?.expect("position utxo");
    let venue_lbtc = fill.venue.get_utxos_asset(params.collateral_asset_id)?[0].clone();
    let cash = fill.venue.get_utxos_asset(params.cash_asset_id)?[0].clone();
    let debt = fill.position.get_remaining_debt();
    let amount = 5000;
    let new_debt = debt - amount;
    let honest_remaining = params.terms.remaining_collateral(new_debt);

    let mut ft = FinalTransaction::new();
    let locktime = LockTime::from_height(params.last_look_height)?;
    let input = PartialInput::new(utxo)
        .with_sequence(Sequence::ENABLE_LOCKTIME_NO_RBF)
        .with_locktime(locktime);
    fill.position.add_program_input_from_partial_input(
        &mut ft,
        input,
        WitnessBranchV5::LastLook {
            terms: params.witness_terms(),
            current_debt: debt,
            amount,
        }
        .build_witness(),
    );
    // Continue the position one sat short of what the covenant requires.
    let mut next = lending_contracts::programs::swaption_lending_v5::SwaptionPositionV5::new(params, new_debt);
    let _ = &mut next;
    ft.add_output(PartialOutput::new(next.get_script_pubkey(), honest_remaining - 1, params.collateral_asset_id));
    ft.add_output(PartialOutput::new(fill.lender_script.clone(), amount, params.cash_asset_id));
    ft.add_input(PartialInput::new(venue_lbtc.clone()), RequiredSignature::NativeEcdsa);
    ft.add_input(PartialInput::new(cash.clone()), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), params.terms.collateral_amount - (honest_remaining - 1), params.collateral_asset_id));
    ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), venue_lbtc.explicit_amount(), params.collateral_asset_id));
    ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), cash.explicit_amount() - amount, params.cash_asset_id));
    assert!(fill.venue.finalize(&ft).is_err(), "the continuing position must hold the proportional collateral");
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
    // Claim a full round (amount == debt) but pay one sat less.
    fill.position.add_program_input_from_partial_input(
        &mut ft,
        input,
        WitnessBranchV5::LastLook {
            terms: params.witness_terms(),
            current_debt: debt,
            amount: debt,
        }
        .build_witness(),
    );
    ft.add_output(PartialOutput::new(fill.lender_script.clone(), debt - 1, params.cash_asset_id));
    ft.add_input(PartialInput::new(venue_lbtc.clone()), RequiredSignature::NativeEcdsa);
    ft.add_input(PartialInput::new(cash.clone()), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), params.terms.collateral_amount, params.collateral_asset_id));
    ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), venue_lbtc.explicit_amount(), params.collateral_asset_id));
    ft.add_output(PartialOutput::new(fill.venue.get_address().script_pubkey(), cash.explicit_amount() - (debt - 1), params.cash_asset_id));
    assert!(fill.venue.finalize(&ft).is_err(), "the lender must get the stated amount");
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
