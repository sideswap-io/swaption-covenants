//! Lapse: after expiry the lender (holder of the lender NFT) takes the
//! remaining collateral. Before expiry the transaction is non-final.

use simplex::simplicityhl::elements::Script;
use simplex::transaction::{FinalTransaction, PartialInput, PartialOutput, RequiredSignature};

use lending_contracts::programs::program::SimplexProgram;
use lending_contracts::programs::swaption_lending::{
    PositionTerms, SwaptionPositionWitnessBranch,
};

use super::setup::{Fill, exercise, fill_position, position_utxo};

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

fn lapse_tx(
    context: &simplex::TestContext,
    fill: &Fill,
    lender: &simplex::signer::Signer,
) -> anyhow::Result<FinalTransaction> {
    let params = fill.parameters;
    let utxo = position_utxo(context, &fill.position)?.expect("position utxo");
    let lender_nft_utxo = lender.get_utxos_asset(params.lender_nft_asset_id)?[0].clone();

    let mut ft = FinalTransaction::new();
    fill.position.attach_lapse(&mut ft, utxo);
    ft.add_input(PartialInput::new(lender_nft_utxo), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(
        lender.get_address().script_pubkey(),
        fill.position.get_remaining_collateral(),
        params.collateral_asset_id,
    ));
    Ok(ft)
}

#[simplex::test]
fn lapse_succeeds_after_expiry(context: simplex::TestContext) -> anyhow::Result<()> {
    let provider = context.get_default_provider();
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    let expiry = fill.parameters.terms.expiry_height;

    context
        .get_network_utils()
        .mine_until_height(u64::from(expiry) + 1)?;
    assert!(provider.fetch_tip_height()? >= expiry);

    let ft = lapse_tx(&context, &fill, &lender)?;
    lender.broadcast(&ft)?.wait()?;

    assert!(position_utxo(&context, &fill.position)?.is_none());
    Ok(())
}

#[simplex::test]
fn lapse_after_partial_exercise_takes_only_what_is_left(
    context: simplex::TestContext,
) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut fill = default_fill(&context, &lender)?;
    let terms = fill.parameters.terms;

    exercise(&context, &mut fill, terms.buyback_amount / 4)?;
    let left = fill.position.get_remaining_collateral();
    assert!(left < terms.collateral_amount && left > 0);

    context
        .get_network_utils()
        .mine_until_height(u64::from(terms.expiry_height) + 1)?;

    let ft = lapse_tx(&context, &fill, &lender)?;
    lender.broadcast(&ft)?.wait()?;

    assert!(position_utxo(&context, &fill.position)?.is_none());
    Ok(())
}

#[simplex::test]
fn lapse_fails_before_expiry(context: simplex::TestContext) -> anyhow::Result<()> {
    let provider = context.get_default_provider();
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    assert!(provider.fetch_tip_height()? < fill.parameters.terms.expiry_height);

    let ft = lapse_tx(&context, &fill, &lender)?;

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
fn lapse_fails_without_burning_the_lender_nft(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    let params = fill.parameters;

    context
        .get_network_utils()
        .mine_until_height(u64::from(params.terms.expiry_height) + 1)?;

    let utxo = position_utxo(&context, &fill.position)?.expect("position utxo");
    let lender_nft_utxo = lender.get_utxos_asset(params.lender_nft_asset_id)?[0].clone();

    // Same shape as attach_lapse, but the NFT goes back to the lender instead
    // of an OP_RETURN burn at output 0.
    let mut ft = FinalTransaction::new();
    let locktime = simplex::simplicityhl::elements::LockTime::from_height(params.terms.expiry_height)?;
    let input = PartialInput::new(utxo)
        .with_sequence(simplex::simplicityhl::elements::Sequence::ENABLE_LOCKTIME_NO_RBF)
        .with_locktime(locktime);
    fill.position.add_program_input_from_partial_input(
        &mut ft,
        input,
        SwaptionPositionWitnessBranch::Lapse {
            current_debt: fill.position.get_remaining_debt(),
        }
        .build_witness(),
    );
    ft.add_output(PartialOutput::new(
        lender.get_address().script_pubkey(),
        1,
        params.lender_nft_asset_id,
    ));
    ft.add_input(PartialInput::new(lender_nft_utxo), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(
        lender.get_address().script_pubkey(),
        params.terms.collateral_amount,
        params.collateral_asset_id,
    ));
    let _ = Script::new_op_return(b"unused");

    assert!(lender.finalize(&ft).is_err(), "keeping the lender NFT must fail");
    Ok(())
}
