//! Lapse (v4): after expiry ANYONE may sweep the remaining collateral, but
//! only to the lender's claim script and only in full. The lender then
//! withdraws from the claim coins with the lender token. Before expiry the
//! sweep is non-final.

use simplex::simplicityhl::elements::{LockTime, Sequence};
use simplex::transaction::{FinalTransaction, PartialInput, PartialOutput, RequiredSignature};

use lending_contracts::programs::program::SimplexProgram;
use lending_contracts::programs::swaption_lending_v4::{PositionTerms, WitnessBranchV4};

use super::setup::{
    Fill, claim_balance, claim_utxos, exercise, fill_position, fund_lender, position_utxo, wallet_balance,
    withdraw_claims,
};

fn default_fill(context: &simplex::TestContext, lender: &simplex::signer::Signer) -> anyhow::Result<Fill> {
    let current_height = context.get_default_provider().fetch_tip_height()?;
    fill_position(
        context,
        lender,
        PositionTerms {
            collateral_amount: 3000,
            buyback_amount: 11_000,
            expiry_height: current_height + 30,
        },
    )
}

/// The honest sweep: position in, remaining collateral out to the claim.
fn sweep_tx(context: &simplex::TestContext, fill: &Fill) -> anyhow::Result<FinalTransaction> {
    let utxo = position_utxo(context, &fill.position)?.expect("position utxo");
    let mut ft = FinalTransaction::new();
    fill.position.attach_lapse(&mut ft, utxo, fill.lender_script.clone());
    Ok(ft)
}

/// A hand-built sweep whose output 0 is chosen by the test.
fn sweep_tx_paying(
    context: &simplex::TestContext,
    fill: &Fill,
    output: PartialOutput,
) -> anyhow::Result<FinalTransaction> {
    let params = fill.parameters;
    let utxo = position_utxo(context, &fill.position)?.expect("position utxo");
    let mut ft = FinalTransaction::new();
    let locktime = LockTime::from_height(params.terms.expiry_height)?;
    let input = PartialInput::new(utxo)
        .with_sequence(Sequence::ENABLE_LOCKTIME_NO_RBF)
        .with_locktime(locktime);
    fill.position.add_program_input_from_partial_input(
        &mut ft,
        input,
        WitnessBranchV4::Lapse {
            terms: params.witness_terms(),
            current_debt: fill.position.get_remaining_debt(),
        }
        .build_witness(),
    );
    ft.add_output(output);
    Ok(ft)
}

#[simplex::test]
fn anyone_can_lapse_after_expiry_into_the_claim(context: simplex::TestContext) -> anyhow::Result<()> {
    let provider = context.get_default_provider();
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    let expiry = params.terms.expiry_height;

    // A stranger with nothing but fee money.
    let stranger = context.random_signer();
    fund_lender(&context, &stranger, params.cash_asset_id, 1)?;

    context.get_network_utils().mine_until_height(u64::from(expiry) + 1)?;
    assert!(provider.fetch_tip_height()? >= expiry);

    let ft = sweep_tx(&context, &fill)?;
    stranger.broadcast(&ft)?.wait()?;

    assert!(position_utxo(&context, &fill.position)?.is_none());
    assert_eq!(claim_balance(&context, &fill, params.collateral_asset_id)?, params.terms.collateral_amount);

    // The lender collects with its token.
    let before = wallet_balance(&lender, params.collateral_asset_id)?;
    withdraw_claims(&context, &lender, &fill)?;
    assert!(claim_utxos(&context, &fill)?.is_empty());
    assert!(wallet_balance(&lender, params.collateral_asset_id)? > before);
    assert_eq!(wallet_balance(&lender, params.lender_nft_asset_id)?, 1, "the token is kept");
    Ok(())
}

#[simplex::test]
fn lapse_after_partial_exercise_sweeps_only_what_is_left(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    let terms = params.terms;

    let paid = terms.buyback_amount / 4;
    exercise(&context, &mut fill, paid)?;
    let left = fill.position.get_remaining_collateral();
    assert!(left < terms.collateral_amount && left > 0);
    assert_eq!(claim_balance(&context, &fill, params.cash_asset_id)?, paid);

    context.get_network_utils().mine_until_height(u64::from(terms.expiry_height) + 1)?;

    let ft = sweep_tx(&context, &fill)?;
    lender.broadcast(&ft)?.wait()?;

    assert!(position_utxo(&context, &fill.position)?.is_none());
    assert_eq!(claim_balance(&context, &fill, params.collateral_asset_id)?, left);

    // One withdrawal collects the cash and the collateral together.
    withdraw_claims(&context, &lender, &fill)?;
    assert!(claim_utxos(&context, &fill)?.is_empty());
    Ok(())
}

#[simplex::test]
fn lapse_fails_before_expiry(context: simplex::TestContext) -> anyhow::Result<()> {
    let provider = context.get_default_provider();
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    assert!(provider.fetch_tip_height()? < fill.parameters.terms.expiry_height);

    let ft = sweep_tx(&context, &fill)?;

    // Script-valid but non-final: either finalize or broadcast must refuse it.
    let refused = match lender.finalize(&ft) {
        Err(_) => true,
        Ok((tx, _)) => provider.broadcast_transaction(&tx).is_err(),
    };
    assert!(refused, "lapse before expiry must be refused");
    assert!(position_utxo(&context, &fill.position)?.is_some());
    Ok(())
}

#[simplex::test]
fn lapse_fails_when_the_collateral_goes_elsewhere(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    let params = fill.parameters;

    context.get_network_utils().mine_until_height(u64::from(params.terms.expiry_height) + 1)?;

    // The lender's own wallet instead of its claim script.
    let ft = sweep_tx_paying(
        &context,
        &fill,
        PartialOutput::new(lender.get_address().script_pubkey(), params.terms.collateral_amount, params.collateral_asset_id),
    )?;
    assert!(lender.finalize(&ft).is_err(), "lapse must pay the claim script");
    assert!(position_utxo(&context, &fill.position)?.is_some());
    Ok(())
}

#[simplex::test]
fn lapse_fails_when_short(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    let params = fill.parameters;

    context.get_network_utils().mine_until_height(u64::from(params.terms.expiry_height) + 1)?;

    let mut ft = sweep_tx_paying(
        &context,
        &fill,
        PartialOutput::new(fill.lender_script.clone(), params.terms.collateral_amount - 1, params.collateral_asset_id),
    )?;
    ft.add_output(PartialOutput::new(lender.get_address().script_pubkey(), 1, params.collateral_asset_id));
    assert!(lender.finalize(&ft).is_err(), "lapse must pay the whole remaining collateral");
    Ok(())
}

#[simplex::test]
fn claim_fails_without_the_lender_token(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    let params = fill.parameters;

    let stranger = context.random_signer();
    fund_lender(&context, &stranger, params.cash_asset_id, 1)?;

    context.get_network_utils().mine_until_height(u64::from(params.terms.expiry_height) + 1)?;
    let ft = sweep_tx(&context, &fill)?;
    stranger.broadcast(&ft)?.wait()?;

    // The stranger puts one of its own coins at input 0 and tries to take the claim.
    let own = stranger.get_utxos_asset(params.collateral_asset_id)?[0].clone();
    let claim_utxo = claim_utxos(&context, &fill)?[0].clone();
    let amount = claim_utxo.explicit_amount();
    let mut ft = FinalTransaction::new();
    ft.add_input(PartialInput::new(own.clone()), RequiredSignature::NativeEcdsa);
    fill.claim.attach_withdraw(&mut ft, claim_utxo);
    ft.add_output(PartialOutput::new(stranger.get_address().script_pubkey(), own.explicit_amount(), params.collateral_asset_id));
    ft.add_output(PartialOutput::new(stranger.get_address().script_pubkey(), amount, params.collateral_asset_id));
    assert!(stranger.finalize(&ft).is_err(), "the claim must need the lender token at input 0");
    assert_eq!(claim_balance(&context, &fill, params.collateral_asset_id)?, params.terms.collateral_amount);
    Ok(())
}
