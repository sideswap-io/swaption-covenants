//! Fills against an escrowed offer: the borrower signs alone, the covenant
//! checks the position it creates, keeps the rest, and takes the fee.

use simplex::simplicityhl::elements::Script;

use lending_contracts::programs::program::SimplexProgram;
use lending_contracts::programs::swaption_lending_v5::script_hash;

use super::setup::{
    FEE_MIN, MIN_SIZE, build_fill_tx, claim_balance, claim_utxos, exercise, fill, lapse, offer_utxo, plan, position_utxo, post_offer,
    refused, wallet_balance, withdraw_claims,
};

#[simplex::test]
fn partial_fill_continues_the_offer_and_pays_the_fee(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let mut posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let cash = posted.cash_asset_id;
    let venue_before = wallet_balance(&posted.venue, cash)?;
    let borrower_before = wallet_balance(borrower, cash)?;

    // 3,000 sats at 3 per unit: 9,000 out, 16,000 left, fee 60.
    let position = fill(&context, &mut posted, 0, 3_000)?;

    assert!(posted.open);
    let cont = offer_utxo(&context, &posted.offer)?.expect("the offer continues");
    assert_eq!(cont.explicit_amount(), 16_000);
    assert_eq!(posted.offer.get_remaining(), 16_000);
    let pos = position_utxo(&context, &position)?.expect("position created");
    assert_eq!(pos.explicit_amount(), 3_000);
    assert_eq!(position.terms.buyback_amount, 12_000);
    assert_eq!(wallet_balance(&posted.venue, cash)? - venue_before, 60);
    assert_eq!(wallet_balance(borrower, cash)? - borrower_before, 9_000 - 60);
    Ok(())
}

#[simplex::test]
fn fills_from_the_continuation_until_the_leftover_goes_to_the_claim(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut posted = post_offer(&context, &lender, 25_000, 80, 60)?;
    let cash = posted.cash_asset_id;

    fill(&context, &mut posted, 0, 3_000)?; // 16,000 left
    fill(&context, &mut posted, 0, 4_000)?; // 4,000 left, still one minimum fill
    assert!(posted.open);
    assert_eq!(offer_utxo(&context, &posted.offer)?.expect("continues").explicit_amount(), 4_000);

    // 1,000 sats takes 3,000; the 1,000 left is under a minimum fill.
    let last = fill(&context, &mut posted, 0, 1_000)?;
    assert!(!posted.open);
    assert!(offer_utxo(&context, &posted.offer)?.is_none(), "no continuation");
    assert!(position_utxo(&context, &last)?.is_some());
    assert_eq!(claim_balance(&context, &posted, cash)?, 1_000, "the leftover sits at the claim script");

    // The lender collects it with its token.
    let before = wallet_balance(&lender, cash)?;
    withdraw_claims(&context, &lender, &posted)?;
    assert!(claim_utxos(&context, &posted)?.is_empty());
    assert_eq!(wallet_balance(&lender, cash)? - before, 1_000);
    assert_eq!(wallet_balance(&lender, posted.lender_token)?, 1, "the token is kept");
    Ok(())
}

#[simplex::test]
fn exact_fill_leaves_nothing_and_moves_the_fee_up(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut posted = post_offer(&context, &lender, 9_000, 60, 40)?;
    let cash = posted.cash_asset_id;
    let venue_before = wallet_balance(&posted.venue, cash)?;

    let position = fill(&context, &mut posted, 0, 3_000)?;

    assert!(!posted.open);
    assert!(offer_utxo(&context, &posted.offer)?.is_none());
    assert!(position_utxo(&context, &position)?.is_some());
    assert_eq!(claim_balance(&context, &posted, cash)?, 0);
    assert_eq!(wallet_balance(&posted.venue, cash)? - venue_before, 60);
    Ok(())
}

#[simplex::test]
fn the_fee_floor_binds_on_a_small_fill(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let cash = posted.cash_asset_id;
    let venue_before = wallet_balance(&posted.venue, cash)?;

    // 1,000 sats: 20 by the rate, so the floor of 50 applies.
    fill(&context, &mut posted, 0, MIN_SIZE)?;
    assert_eq!(wallet_balance(&posted.venue, cash)? - venue_before, FEE_MIN);
    Ok(())
}

#[simplex::test]
fn a_created_position_pays_the_lender_claim_on_exercise_and_on_lapse(context: simplex::TestContext) -> anyhow::Result<()> {
    let lender = context.random_signer();
    let mut posted = post_offer(&context, &lender, 25_000, 40, 30)?;
    let cash = posted.cash_asset_id;
    let policy = context.get_network().policy_asset();

    let first = fill(&context, &mut posted, 0, 3_000)?;
    let second = fill(&context, &mut posted, 0, 2_000)?;

    // The borrower buys the first one back in full: 12,000 to the claim.
    exercise(&context, &posted, &first, first.terms.buyback_amount, first.terms.buyback_amount)?;
    assert!(position_utxo(&context, &first)?.is_none());
    assert_eq!(claim_balance(&context, &posted, cash)?, 12_000);

    // The second lapses: 2,000 sats of collateral to the same claim.
    context.get_network_utils().mine_until_height(u64::from(second.terms.expiry_height) + 1)?;
    lapse(&context, &posted, &second, &posted.venue)?;
    assert!(position_utxo(&context, &second)?.is_none());
    assert_eq!(claim_balance(&context, &posted, policy)?, 2_000);

    // One withdrawal, one token, both assets.
    let cash_before = wallet_balance(&lender, cash)?;
    let policy_before = wallet_balance(&lender, policy)?;
    withdraw_claims(&context, &lender, &posted)?;
    assert!(claim_utxos(&context, &posted)?.is_empty());
    assert_eq!(wallet_balance(&lender, cash)? - cash_before, 12_000);
    assert!(wallet_balance(&lender, policy)? > policy_before);
    Ok(())
}

// Refusals. Each bends one thing in an otherwise honest fill and expects the
// covenant (or the node) to refuse it, leaving the offer coin in place.

#[simplex::test]
fn fill_fails_when_the_lender_token_term_is_tampered(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let mut plan = plan(&context, &posted, 0, 3_000);
    plan.position.lender_nft_asset_id = posted.borrower_token;
    let (ft, _) = build_fill_tx(&context, &posted, &plan)?;
    assert!(refused(&context, borrower, &ft), "the position must name the lender token");
    assert!(offer_utxo(&context, &posted.offer)?.is_some());
    Ok(())
}

#[simplex::test]
fn fill_fails_when_the_payout_is_not_the_claim_script(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let mut plan = plan(&context, &posted, 0, 3_000);
    plan.position.lender_payout_script_hash = script_hash(&borrower.get_address().script_pubkey());
    let (ft, _) = build_fill_tx(&context, &posted, &plan)?;
    assert!(refused(&context, borrower, &ft), "the position must pay the claim script");
    assert!(offer_utxo(&context, &posted.offer)?.is_some());
    Ok(())
}

#[simplex::test]
fn fill_fails_when_the_expiry_is_not_the_rows(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let mut plan = plan(&context, &posted, 0, 3_000);
    plan.position.terms.expiry_height += 1_000;
    let (ft, _) = build_fill_tx(&context, &posted, &plan)?;
    assert!(refused(&context, borrower, &ft), "the position must carry the row's expiry");
    Ok(())
}

#[simplex::test]
fn fill_fails_when_the_buyback_is_below_the_rate(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let mut plan = plan(&context, &posted, 0, 3_000);
    plan.position.terms.buyback_amount -= 1;
    let (ft, _) = build_fill_tx(&context, &posted, &plan)?;
    assert!(refused(&context, borrower, &ft), "the borrower must owe at least the rate");
    Ok(())
}

#[simplex::test]
fn fill_fails_when_the_position_holds_less_than_n(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let mut plan = plan(&context, &posted, 0, 3_000);
    plan.position_amount = 2_999;
    let (ft, _) = build_fill_tx(&context, &posted, &plan)?;
    assert!(refused(&context, borrower, &ft), "the position must hold n");
    Ok(())
}

#[simplex::test]
fn fill_fails_when_the_fee_is_short(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let mut plan = plan(&context, &posted, 0, 3_000);
    plan.fee -= 1;
    let (ft, _) = build_fill_tx(&context, &posted, &plan)?;
    assert!(refused(&context, borrower, &ft), "the fee must be at least what is due");
    Ok(())
}

#[simplex::test]
fn fill_fails_when_the_fee_goes_elsewhere(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let mut plan = plan(&context, &posted, 0, 3_000);
    plan.fee_script = borrower.get_address().script_pubkey();
    let (ft, _) = build_fill_tx(&context, &posted, &plan)?;
    assert!(refused(&context, borrower, &ft), "the fee must go to the fee script");
    Ok(())
}

#[simplex::test]
fn fill_fails_below_the_minimum_size(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let plan = plan(&context, &posted, 0, MIN_SIZE - 1);
    let (ft, _) = build_fill_tx(&context, &posted, &plan)?;
    assert!(refused(&context, borrower, &ft), "a fill must be at least the minimum");
    Ok(())
}

#[simplex::test]
fn fill_fails_when_it_takes_more_than_the_offer_holds(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    // 10,000 sats would take 30,000 of the 25,000 escrowed.
    let plan = plan(&context, &posted, 0, 10_000);
    let (ft, _) = build_fill_tx(&context, &posted, &plan)?;
    assert!(refused(&context, borrower, &ft), "the offer cannot release more than it holds");
    Ok(())
}

#[simplex::test]
fn fill_fails_when_the_continuation_is_short(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let mut plan = plan(&context, &posted, 0, 3_000);
    // 15,999 back into the offer instead of 16,000; the sat goes to the borrower.
    let short = lending_contracts::programs::swaption_offer::SwaptionOffer::new(posted.params, 15_999);
    plan.rest_override = Some((short.get_script_pubkey(), 15_999));
    let (ft, _) = build_fill_tx(&context, &posted, &plan)?;
    assert!(refused(&context, borrower, &ft), "the continuation must keep every remaining sat");
    Ok(())
}

#[simplex::test]
fn fill_fails_when_the_continuation_is_not_the_offer_script(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let mut plan = plan(&context, &posted, 0, 3_000);
    plan.rest_override = Some((borrower.get_address().script_pubkey(), 16_000));
    let (ft, _) = build_fill_tx(&context, &posted, &plan)?;
    assert!(refused(&context, borrower, &ft), "the rest must stay in the offer");
    Ok(())
}

#[simplex::test]
fn fill_fails_when_the_leftover_goes_elsewhere(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 4_000, 60, 40)?;
    let mut plan = plan(&context, &posted, 0, 1_000);
    // 3,000 taken, 1,000 left under a minimum fill: it must go to the claim.
    plan.rest_override = Some((borrower.get_address().script_pubkey(), 1_000));
    let (ft, _) = build_fill_tx(&context, &posted, &plan)?;
    assert!(refused(&context, borrower, &ft), "a leftover must go to the claim script");
    Ok(())
}

#[simplex::test]
fn fill_fails_on_an_empty_row(context: simplex::TestContext) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let lender = context.random_signer();
    let posted = post_offer(&context, &lender, 25_000, 60, 40)?;
    let mut plan = plan(&context, &posted, 0, 3_000);
    plan.row = 1;
    // Row 1 is all zero: expiry 0, prices 0. Keep the position as row 0 built it.
    let (ft, _) = build_fill_tx(&context, &posted, &plan)?;
    assert!(refused(&context, borrower, &ft), "an empty row cannot be filled");
    let _unused: Option<Script> = None;
    Ok(())
}
