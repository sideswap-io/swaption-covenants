//! Withdrawing an offer (the lender token at input 0) and sweeping one past
//! its cutoff (anyone, into the claim script).

use super::setup::{
    cancel_tx, claim_balance, claim_utxos, expire_tx, fill, fund, offer_utxo, post_offer, refused, wallet_balance, withdraw_claims,
};

#[simplex::test]
fn lender_withdraws_with_its_token(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let cash = posted.cash_asset_id;
    fill(&context, &mut posted, 0, 3_000)?; // 16,000 left in the continuation

    let before = wallet_balance(&lender, cash)?;
    let token = lender.get_utxos_asset(posted.lender_token)?[0].clone();
    let ft = cancel_tx(&context, &posted, &lender, token)?;
    lender.broadcast(&ft)?.wait()?;

    assert!(offer_utxo(&context, &posted.offer)?.is_none());
    assert_eq!(wallet_balance(&lender, cash)? - before, 16_000);
    assert_eq!(wallet_balance(&lender, posted.lender_token)?, 1, "the token is kept");
    Ok(())
}

#[simplex::test]
fn cancel_fails_without_the_token_at_input_zero(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let stranger = context.random_signer();
    fund(&context, &stranger, posted.cash_asset_id, 1)?;

    // The stranger puts one of its own coins at input 0.
    let own = stranger.get_utxos_asset(posted.cash_asset_id)?[0].clone();
    let ft = cancel_tx(&context, &posted, &stranger, own)?;
    assert!(refused(&context, &stranger, &ft), "cancel needs the lender token");
    assert!(offer_utxo(&context, &posted.offer)?.is_some());
    Ok(())
}

#[simplex::test]
fn cancel_fails_with_another_token(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;

    // The borrower token is a one-unit asset too, but not the lender's.
    let token = borrower.get_utxos_asset(posted.borrower_token)?[0].clone();
    let ft = cancel_tx(&context, &posted, borrower, token)?;
    assert!(refused(&context, borrower, &ft), "only the lender token cancels");
    assert!(offer_utxo(&context, &posted.offer)?.is_some());
    Ok(())
}

#[simplex::test]
fn anyone_sweeps_an_expired_offer_into_the_claim(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut posted = post_offer(&context, &lender, 25_000, 60, 20)?;
    let cash = posted.cash_asset_id;
    fill(&context, &mut posted, 0, 3_000)?; // 16,000 left

    let stranger = context.random_signer();
    fund(&context, &stranger, cash, 1)?;
    context.get_network_utils().mine_until_height(u64::from(posted.params.cutoff_height) + 1)?;

    let ft = expire_tx(&context, &posted)?;
    stranger.broadcast(&ft)?.wait()?;

    assert!(offer_utxo(&context, &posted.offer)?.is_none());
    assert_eq!(claim_balance(&context, &posted, cash)?, 16_000);

    let before = wallet_balance(&lender, cash)?;
    withdraw_claims(&context, &lender, &posted)?;
    assert!(claim_utxos(&context, &posted)?.is_empty());
    assert_eq!(wallet_balance(&lender, cash)? - before, 16_000);
    Ok(())
}

#[simplex::test]
fn expire_fails_before_the_cutoff(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    assert!(context.get_default_provider().fetch_tip_height()? < posted.params.cutoff_height);

    let ft = expire_tx(&context, &posted)?;
    assert!(refused(&context, &posted.venue, &ft), "expire before the cutoff must be refused");
    assert!(offer_utxo(&context, &posted.offer)?.is_some());
    Ok(())
}

#[simplex::test]
fn expire_fails_when_the_cash_goes_elsewhere(context: simplex::TestContext) -> anyhow::Result<()> {
    use lending_contracts::programs::program::SimplexProgram;
    use lending_contracts::programs::swaption_offer::WitnessBranchOffer;
    use simplex::simplicityhl::elements::{LockTime, Sequence};
    use simplex::transaction::{FinalTransaction, PartialInput, PartialOutput};

    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 20)?;
    context.get_network_utils().mine_until_height(u64::from(posted.params.cutoff_height) + 1)?;

    // The sweeper pays itself instead of the claim script.
    let utxo = offer_utxo(&context, &posted.offer)?.expect("offer utxo");
    let mut ft = FinalTransaction::new();
    let input = PartialInput::new(utxo)
        .with_sequence(Sequence::ENABLE_LOCKTIME_NO_RBF)
        .with_locktime(LockTime::from_height(posted.params.cutoff_height)?);
    posted.offer.add_program_input_from_partial_input(
        &mut ft,
        input,
        WitnessBranchOffer::Expire {
            terms: posted.params.witness_terms(),
            remaining: posted.offer.get_remaining(),
        }
        .build_witness(),
    );
    ft.add_output(PartialOutput::new(posted.venue.get_address().script_pubkey(), 25_000, posted.cash_asset_id));
    assert!(refused(&context, &posted.venue, &ft), "expire must pay the claim script");
    assert!(offer_utxo(&context, &posted.offer)?.is_some());
    Ok(())
}
