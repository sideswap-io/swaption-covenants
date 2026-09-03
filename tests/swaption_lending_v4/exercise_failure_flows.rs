//! Exercise transactions the covenant must reject. Failures surface at
//! `finalize`, where Simplex evaluates the program locally.

use simplex::simplicityhl::elements::Script;
use simplex::transaction::{FinalTransaction, PartialInput, PartialOutput, RequiredSignature};

use lending_contracts::programs::program::SimplexProgram;
use lending_contracts::programs::swaption_lending_v4::{
    PositionTerms, WitnessBranchV4,
};

use super::setup::{Fill, fill_position, position_utxo};

fn default_fill(context: &simplex::TestContext, lender: &simplex::signer::Signer) -> anyhow::Result<Fill> {
    let current_height = context.get_default_provider().fetch_tip_height()?;
    fill_position(
        context,
        lender,
        PositionTerms {
            collateral_amount: 3000,
            buyback_amount: 11_000,
            expiry_height: current_height + 60,
        },
    )
}

/// Builds a full exercise by hand: NFT burn at output 0, payout at output 1,
/// then the given `payout` (script, amount) instead of the honest one.
fn full_exercise_with_payout(
    context: &simplex::TestContext,
    fill: &Fill,
    payout_script: Script,
    payout_amount: u64,
) -> anyhow::Result<FinalTransaction> {
    let borrower = context.get_default_signer();
    let params = fill.parameters;
    let debt = fill.position.get_remaining_debt();

    let utxo = position_utxo(context, &fill.position)?.expect("position utxo");
    let borrower_nft_utxo = borrower.get_utxos_asset(params.borrower_nft_asset_id)?[0].clone();
    let cash_utxo = borrower.get_utxos_filter(
        &|u| u.explicit_asset() == params.cash_asset_id && u.explicit_amount() >= debt,
        &|_| true,
    )?[0]
        .clone();
    let cash_amount = cash_utxo.explicit_amount();

    let mut ft = FinalTransaction::new();

    ft.add_input(PartialInput::new(borrower_nft_utxo), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(
        Script::new_op_return(b"burn"),
        1,
        params.borrower_nft_asset_id,
    ));

    fill.position.add_program_input(
        &mut ft,
        utxo,
        WitnessBranchV4::Exercise {
            terms: fill.parameters.witness_terms(),
            current_debt: debt,
            amount: debt,
        }
        .build_witness(),
    );

    ft.add_output(PartialOutput::new(payout_script, payout_amount, params.cash_asset_id));

    ft.add_input(PartialInput::new(cash_utxo), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(
        borrower.get_address().script_pubkey(),
        params.terms.collateral_amount,
        params.collateral_asset_id,
    ));
    if cash_amount > payout_amount {
        ft.add_output(PartialOutput::new(
            borrower.get_address().script_pubkey(),
            cash_amount - payout_amount,
            params.cash_asset_id,
        ));
    }

    Ok(ft)
}

#[simplex::test]
fn exercise_fails_when_payout_goes_to_the_wrong_script(
    context: simplex::TestContext,
) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;

    let ft = full_exercise_with_payout(
        &context,
        &fill,
        borrower.get_address().script_pubkey(), // pays himself
        fill.parameters.terms.buyback_amount,
    )?;

    assert!(borrower.finalize(&ft).is_err(), "payout to a non-lender script must fail");
    Ok(())
}

#[simplex::test]
fn exercise_fails_when_payout_is_short(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;

    let ft = full_exercise_with_payout(
        &context,
        &fill,
        fill.lender_script.clone(),
        fill.parameters.terms.buyback_amount - 1,
    )?;

    assert!(borrower.finalize(&ft).is_err(), "underpaying the buyback must fail");
    Ok(())
}

#[simplex::test]
fn partial_exercise_fails_when_too_much_collateral_is_released(
    context: simplex::TestContext,
) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    let debt = fill.position.get_remaining_debt();
    let amount = debt / 3;

    let utxo = position_utxo(&context, &fill.position)?.expect("position utxo");
    let borrower_nft_utxo = borrower.get_utxos_asset(params.borrower_nft_asset_id)?[0].clone();
    let cash_utxo = borrower.get_utxos_filter(
        &|u| u.explicit_asset() == params.cash_asset_id && u.explicit_amount() >= amount,
        &|_| true,
    )?[0]
        .clone();

    let honest_remaining = params.terms.remaining_collateral(debt - amount);

    // Continue the position with one satoshi less collateral than owed.
    let mut continued = lending_contracts::programs::swaption_lending_v4::SwaptionPositionV4::new(
        params,
        debt - amount,
    );

    let mut ft = FinalTransaction::new();
    ft.add_input(PartialInput::new(borrower_nft_utxo), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(
        borrower.get_address().script_pubkey(),
        1,
        params.borrower_nft_asset_id,
    ));
    fill.position.add_program_input(
        &mut ft,
        utxo,
        WitnessBranchV4::Exercise {
            terms: fill.parameters.witness_terms(),
            current_debt: debt,
            amount,
        }
        .build_witness(),
    );
    continued.add_program_output(&mut ft, params.collateral_asset_id, honest_remaining - 1);
    ft.add_output(PartialOutput::new(
        fill.lender_script.clone(),
        amount,
        params.cash_asset_id,
    ));
    ft.add_input(PartialInput::new(cash_utxo.clone()), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(
        borrower.get_address().script_pubkey(),
        params.terms.collateral_amount - (honest_remaining - 1),
        params.collateral_asset_id,
    ));
    ft.add_output(PartialOutput::new(
        borrower.get_address().script_pubkey(),
        cash_utxo.explicit_amount() - amount,
        params.cash_asset_id,
    ));

    assert!(borrower.finalize(&ft).is_err(), "releasing extra collateral must fail");
    let _ = &mut continued;
    Ok(())
}

#[simplex::test]
fn exercise_fails_without_the_borrower_nft(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let fill = default_fill(&context, &lender)?;
    let params = fill.parameters;
    let debt = fill.position.get_remaining_debt();

    let utxo = position_utxo(&context, &fill.position)?.expect("position utxo");
    let cash_utxo = borrower.get_utxos_filter(
        &|u| u.explicit_asset() == params.cash_asset_id && u.explicit_amount() >= debt,
        &|_| true,
    )?[0]
        .clone();
    let cash_amount = cash_utxo.explicit_amount();

    // Input 0 is a plain cash coin, not the borrower NFT; output 0 "burns" cash.
    let mut ft = FinalTransaction::new();
    ft.add_input(PartialInput::new(cash_utxo), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(
        Script::new_op_return(b"burn"),
        1,
        params.cash_asset_id,
    ));
    fill.position.add_program_input(
        &mut ft,
        utxo,
        WitnessBranchV4::Exercise {
            terms: fill.parameters.witness_terms(),
            current_debt: debt,
            amount: debt,
        }
        .build_witness(),
    );
    ft.add_output(PartialOutput::new(fill.lender_script.clone(), debt, params.cash_asset_id));
    ft.add_output(PartialOutput::new(
        borrower.get_address().script_pubkey(),
        params.terms.collateral_amount,
        params.collateral_asset_id,
    ));
    ft.add_output(PartialOutput::new(
        borrower.get_address().script_pubkey(),
        cash_amount - debt - 1,
        params.cash_asset_id,
    ));

    assert!(borrower.finalize(&ft).is_err(), "exercise without the borrower NFT must fail");
    Ok(())
}
