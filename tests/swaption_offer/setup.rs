//! Regtest setup for the offer covenant (offer v1 creating v5 positions).
//!
//! The default signer is the BORROWER: it holds the borrower token and funds
//! the collateral of every fill. A random signer is the LENDER: it posts the
//! offer from its own cash and holds the lender token. Another random signer
//! is the VENUE: its address is the fee script, and it doubles as the
//! stranger who sweeps expired offers and lapsed positions.

use simplex::signer::Signer;
use simplex::simplicityhl::elements::{AssetId, Script, Txid};
use simplex::transaction::partial_input::IssuanceInput;
use simplex::transaction::{FinalTransaction, PartialInput, PartialOutput, RequiredSignature, UTXO};

use lending_contracts::programs::program::SimplexProgram;
use lending_contracts::programs::swaption_claim::SwaptionClaim;
use lending_contracts::programs::swaption_lending_v5::{
    PositionParametersV5, SwaptionPositionV5, script_hash, tapleaf_hash_of_program as position_leaf,
};
use lending_contracts::programs::swaption_offer::{OFFER_ROWS, OfferParameters, OfferRow, SwaptionOffer, WitnessBranchOffer};
use lending_contracts::utils::get_random_seed;

use super::common::issuance::issue_asset;
use super::common::wallet::split_first_signer_utxo;

pub(super) const CASH_SUPPLY: u64 = 1_000_000;
pub(super) const LAST_LOOK_BLOCKS: u32 = 10;

/// Test prices per whole unit of collateral: 3 cash released, 4 owed back,
/// a venue fee of 0.02 per unit or 50 per fill, fills of at least 1,000 sats
/// (so one minimum fill takes 3,000 of cash).
pub(super) const PRICE_OUT: u64 = 300_000_000;
pub(super) const BUYBACK_PRICE: u64 = 400_000_000;
pub(super) const FEE_PER_UNIT: u64 = 2_000_000;
pub(super) const FEE_MIN: u64 = 50;
pub(super) const MIN_SIZE: u64 = 1_000;

pub(super) struct Posted {
    #[allow(dead_code)]
    pub txid: Txid,
    pub venue: Signer,
    pub cash_asset_id: AssetId,
    pub lender_token: AssetId,
    pub borrower_token: AssetId,
    pub params: OfferParameters,
    /// The live coin's wrapper; replaced by the continuation after a fill.
    pub offer: SwaptionOffer,
    /// False once a fill took the last minimum fill (leftover or exact).
    pub open: bool,
    pub claim: SwaptionClaim,
    pub claim_script: Script,
    pub fee_script: Script,
}

/// Issue one unit of a fresh asset straight to `recipient` (the default
/// signer pays), the way the server's token pool delivers tokens.
pub(super) fn issue_unit_to(context: &simplex::TestContext, recipient: Script) -> anyhow::Result<AssetId> {
    let signer = context.get_default_signer();
    let policy = context.get_network().policy_asset();
    let utxo = signer.get_utxos_asset(policy)?[0].clone();

    let mut ft = FinalTransaction::new();
    let issued = ft.add_issuance_input(
        PartialInput::new(utxo.clone()),
        IssuanceInput::new_issuance(1, 0, get_random_seed()),
        RequiredSignature::NativeEcdsa,
    );
    ft.add_output(PartialOutput::new(recipient, 1, issued.asset_id));
    ft.add_output(PartialOutput::new(signer.get_address().script_pubkey(), utxo.explicit_amount(), policy));
    signer.broadcast(&ft)?.wait()?;
    Ok(issued.asset_id)
}

/// Give `who` some cash and half of one of the default signer's L-BTC coins.
pub(super) fn fund(context: &simplex::TestContext, who: &Signer, cash_asset_id: AssetId, cash_to_send: u64) -> anyhow::Result<()> {
    let signer = context.get_default_signer();
    let policy = context.get_network().policy_asset();

    let cash_utxo = signer.get_utxos_asset(cash_asset_id)?[0].clone();
    let policy_utxo = signer.get_utxos_asset(policy)?[0].clone();
    let cash_utxo_amount = cash_utxo.explicit_amount();
    let policy_to_send = policy_utxo.explicit_amount() / 2;

    let mut ft = FinalTransaction::new();
    ft.add_input(PartialInput::new(cash_utxo), RequiredSignature::NativeEcdsa);
    ft.add_input(PartialInput::new(policy_utxo), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(who.get_address().script_pubkey(), cash_to_send, cash_asset_id));
    ft.add_output(PartialOutput::new(who.get_address().script_pubkey(), policy_to_send, policy));
    if cash_utxo_amount > cash_to_send {
        ft.add_output(PartialOutput::new(signer.get_address().script_pubkey(), cash_utxo_amount - cash_to_send, cash_asset_id));
    }
    signer.broadcast(&ft)?.wait()?;
    Ok(())
}

/// Issue the cash, fund the lender and the venue, mint the two tokens, and
/// post an offer of `cash_amount` with one row (L-BTC collateral, expiry
/// `expiry_in` blocks out, cutoff `cutoff_in` blocks out).
pub(super) fn post_offer(
    context: &simplex::TestContext,
    lender: &Signer,
    cash_amount: u64,
    expiry_in: u32,
    cutoff_in: u32,
) -> anyhow::Result<Posted> {
    let signer = context.get_default_signer();
    let network = *context.get_network();
    let policy = network.policy_asset();

    split_first_signer_utxo(context, vec![5000, 10000, 20000, 20000, 20000, 20000]);

    let cash_asset_id = issue_asset(context, CASH_SUPPLY)?;
    fund(context, lender, cash_asset_id, CASH_SUPPLY / 10)?;
    let venue = context.random_signer();
    fund(context, &venue, cash_asset_id, 1)?;

    let lender_token = issue_unit_to(context, lender.get_address().script_pubkey())?;
    let borrower_token = issue_unit_to(context, signer.get_address().script_pubkey())?;

    let claim = SwaptionClaim::new(lender_token, network);
    let claim_script = claim.get_script_pubkey();
    let fee_script = venue.get_address().script_pubkey();

    let tip = context.get_default_provider().fetch_tip_height()?;
    let mut rows = [OfferRow::empty(); OFFER_ROWS];
    rows[0] = OfferRow {
        collateral_asset_id: policy,
        expiry_height: tip + expiry_in,
        price_out: PRICE_OUT,
        buyback_price: BUYBACK_PRICE,
        fee_per_unit: FEE_PER_UNIT,
        min_size: MIN_SIZE,
    };
    let params = OfferParameters {
        cash_asset_id,
        lender_token_asset_id: lender_token,
        claim_script_hash: script_hash(&claim_script),
        fee_script_hash: script_hash(&fee_script),
        position_leaf: position_leaf(),
        fee_min: FEE_MIN,
        cutoff_height: tip + cutoff_in,
        rows,
        network,
    };
    let offer = SwaptionOffer::new(params, cash_amount);

    // The post: the lender's cash in, the offer out, change back.
    let cash_utxo = lender.get_utxos_filter(
        &|u| u.explicit_asset() == cash_asset_id && u.explicit_amount() >= cash_amount,
        &|_| true,
    )?[0]
        .clone();
    let mut ft = FinalTransaction::new();
    ft.add_input(PartialInput::new(cash_utxo.clone()), RequiredSignature::NativeEcdsa);
    offer.attach_post(&mut ft);
    if cash_utxo.explicit_amount() > cash_amount {
        ft.add_output(PartialOutput::new(
            lender.get_address().script_pubkey(),
            cash_utxo.explicit_amount() - cash_amount,
            cash_asset_id,
        ));
    }
    let receipt = lender.broadcast(&ft)?;
    receipt.wait()?;

    Ok(Posted {
        txid: receipt.txid(),
        venue,
        cash_asset_id,
        lender_token,
        borrower_token,
        params,
        offer,
        open: true,
        claim,
        claim_script,
        fee_script,
    })
}

pub(super) fn offer_utxo(context: &simplex::TestContext, offer: &SwaptionOffer) -> anyhow::Result<Option<UTXO>> {
    Ok(context
        .get_default_provider()
        .fetch_scripthash_utxos(&offer.get_script_pubkey())?
        .first()
        .cloned())
}

pub(super) fn position_utxo(context: &simplex::TestContext, position: &PositionParametersV5) -> anyhow::Result<Option<UTXO>> {
    Ok(context
        .get_default_provider()
        .fetch_scripthash_utxos(&SwaptionPositionV5::new_filled(*position).get_script_pubkey())?
        .first()
        .cloned())
}

/// The honest v5 position for a fill of `n` on `row`, owing `buyback_amount`.
pub(super) fn position_for(context: &simplex::TestContext, posted: &Posted, row: u8, n: u64, buyback_amount: u64) -> PositionParametersV5 {
    let borrower = context.get_default_signer();
    let expiry = posted.params.row(row).expiry_height;
    posted.params.position_parameters(
        row,
        n,
        buyback_amount,
        posted.borrower_token,
        script_hash(&borrower.get_address().script_pubkey()),
        script_hash(&posted.venue.get_address().script_pubkey()),
        expiry.saturating_sub(LAST_LOOK_BLOCKS),
    )
}

/// A fill transaction built by hand so a scenario can bend any part of it.
pub(super) struct FillPlan {
    pub row: u8,
    pub position: PositionParametersV5,
    /// Amount of the position output (the covenant expects `n`).
    pub position_amount: u64,
    pub fee: u64,
    pub fee_script: Script,
    /// Replace output 2 (continuation or leftover) with this.
    pub rest_override: Option<(Script, u64)>,
}

/// The honest plan for `n` on `row`.
pub(super) fn plan(context: &simplex::TestContext, posted: &Posted, row: u8, n: u64) -> FillPlan {
    let buyback = posted.params.row(row).buyback_floor(n);
    FillPlan {
        row,
        position: position_for(context, posted, row, n, buyback),
        position_amount: n,
        fee: posted.params.fee_due(row, n),
        fee_script: posted.fee_script.clone(),
        rest_override: None,
    }
}

/// inputs: 0 the offer, 1 the borrower token, 2 the borrower's L-BTC;
/// outputs: 0 position, 1 token, [2 continuation | leftover], fee, proceeds.
/// Returns the transaction and the continued offer the honest layout implies.
pub(super) fn build_fill_tx(context: &simplex::TestContext, posted: &Posted, plan: &FillPlan) -> anyhow::Result<(FinalTransaction, Option<SwaptionOffer>)> {
    let borrower = context.get_default_signer();
    let policy = context.get_network().policy_asset();
    let params = posted.params;
    let cash = params.cash_asset_id;
    let offer = &posted.offer;
    let remaining = offer.get_remaining();
    let n = plan.position.terms.collateral_amount;
    let row = params.row(plan.row);

    let utxo = offer_utxo(context, offer)?.ok_or_else(|| anyhow::anyhow!("offer utxo missing"))?;
    let witness = WitnessBranchOffer::fill(&params, remaining, plan.row, &plan.position).build_witness();

    let mut ft = FinalTransaction::new();
    offer.add_program_input(&mut ft, utxo, witness);

    let created = SwaptionPositionV5::new_filled(plan.position);
    ft.add_output(PartialOutput::new(created.get_script_pubkey(), plan.position_amount, plan.position.collateral_asset_id));
    ft.add_output(PartialOutput::new(borrower.get_address().script_pubkey(), 1, plan.position.borrower_nft_asset_id));

    // Cash accounting: what the fill takes, what is left, who gets it.
    let out = row.cash_out(n);
    let taken = out.min(remaining);
    let rest = remaining - taken;
    let continues = rest >= row.min_cash();
    let next = match &plan.rest_override {
        Some((script, amount)) => {
            ft.add_output(PartialOutput::new(script.clone(), *amount, cash));
            None
        }
        None if continues => {
            let next = SwaptionOffer::new(params, rest);
            next.add_program_output(&mut ft, cash, rest);
            Some(next)
        }
        None => {
            if rest > 0 {
                ft.add_output(PartialOutput::new(posted.claim_script.clone(), rest, cash));
            }
            None
        }
    };
    ft.add_output(PartialOutput::new(plan.fee_script.clone(), plan.fee, cash));

    let token_utxo = borrower.get_utxos_asset(posted.borrower_token)?[0].clone();
    ft.add_input(PartialInput::new(token_utxo), RequiredSignature::NativeEcdsa);
    let collateral_utxo = borrower.get_utxos_filter(
        &|u| u.explicit_asset() == policy && u.explicit_amount() >= plan.position_amount + 2_000,
        &|_| true,
    )?[0]
        .clone();
    ft.add_input(PartialInput::new(collateral_utxo), RequiredSignature::NativeEcdsa);

    // The borrower takes what the offer released beyond the fee (and beyond
    // any override that kept more in the rest output).
    let kept = plan.rest_override.as_ref().map(|(_, a)| *a).unwrap_or(rest);
    let proceeds = remaining.saturating_sub(kept).saturating_sub(plan.fee);
    if proceeds > 0 {
        ft.add_output(PartialOutput::new(borrower.get_address().script_pubkey(), proceeds, cash));
    }

    Ok((ft, next))
}

/// Broadcast the honest fill of `n` on `row`; the offer's wrapper follows
/// the coin. Returns the created position.
pub(super) fn fill(context: &simplex::TestContext, posted: &mut Posted, row: u8, n: u64) -> anyhow::Result<PositionParametersV5> {
    let borrower = context.get_default_signer();
    let plan = plan(context, posted, row, n);
    let (ft, next) = build_fill_tx(context, posted, &plan)?;
    borrower.broadcast(&ft)?.wait()?;
    match next {
        Some(next) => posted.offer = next,
        None => posted.open = false,
    }
    Ok(plan.position)
}

/// True when the signer cannot finalise the transaction or the node refuses it.
pub(super) fn refused(context: &simplex::TestContext, signer: &Signer, ft: &FinalTransaction) -> bool {
    match signer.finalize(ft) {
        Err(_) => true,
        Ok((tx, _)) => context.get_default_provider().broadcast_transaction(&tx).is_err(),
    }
}

/// The lender withdraws with its token: input 0 the token, then the offer.
pub(super) fn cancel_tx(context: &simplex::TestContext, posted: &Posted, lender: &Signer, token_input: UTXO) -> anyhow::Result<FinalTransaction> {
    let utxo = offer_utxo(context, &posted.offer)?.ok_or_else(|| anyhow::anyhow!("offer utxo missing"))?;
    let mut ft = FinalTransaction::new();
    ft.add_input(PartialInput::new(token_input.clone()), RequiredSignature::NativeEcdsa);
    posted.offer.attach_cancel(&mut ft, utxo);
    ft.add_output(PartialOutput::new(lender.get_address().script_pubkey(), token_input.explicit_amount(), token_input.explicit_asset()));
    ft.add_output(PartialOutput::new(lender.get_address().script_pubkey(), posted.offer.get_remaining(), posted.cash_asset_id));
    Ok(ft)
}

/// The sweep after the cutoff: the offer in, all the cash to the claim script.
pub(super) fn expire_tx(context: &simplex::TestContext, posted: &Posted) -> anyhow::Result<FinalTransaction> {
    let utxo = offer_utxo(context, &posted.offer)?.ok_or_else(|| anyhow::anyhow!("offer utxo missing"))?;
    let mut ft = FinalTransaction::new();
    posted.offer.attach_expire(&mut ft, utxo, posted.claim_script.clone());
    Ok(ft)
}

/// Borrower exercises `amount` of the position, paying the claim script.
pub(super) fn exercise(context: &simplex::TestContext, posted: &Posted, position: &PositionParametersV5, current_debt: u64, amount: u64) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let mut created = SwaptionPositionV5::new(*position, current_debt);
    let utxo = position_utxo_at(context, &created)?.ok_or_else(|| anyhow::anyhow!("position utxo missing"))?;
    let token_utxo = borrower.get_utxos_asset(position.borrower_nft_asset_id)?[0].clone();
    let cash_utxo = borrower.get_utxos_filter(
        &|u| u.explicit_asset() == position.cash_asset_id && u.explicit_amount() >= amount,
        &|_| true,
    )?[0]
        .clone();
    let released = position.terms.released_by(current_debt, amount);
    let is_partial = amount < current_debt;

    let mut ft = FinalTransaction::new();
    ft.add_input(PartialInput::new(token_utxo), RequiredSignature::NativeEcdsa);
    if is_partial {
        ft.add_output(PartialOutput::new(borrower.get_address().script_pubkey(), 1, position.borrower_nft_asset_id));
    }
    created.attach_exercise(&mut ft, utxo, amount, posted.claim_script.clone());
    ft.add_input(PartialInput::new(cash_utxo.clone()), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(borrower.get_address().script_pubkey(), released, position.collateral_asset_id));
    if cash_utxo.explicit_amount() > amount {
        ft.add_output(PartialOutput::new(borrower.get_address().script_pubkey(), cash_utxo.explicit_amount() - amount, position.cash_asset_id));
    }
    borrower.broadcast(&ft)?.wait()?;
    Ok(())
}

pub(super) fn position_utxo_at(context: &simplex::TestContext, position: &SwaptionPositionV5) -> anyhow::Result<Option<UTXO>> {
    Ok(context
        .get_default_provider()
        .fetch_scripthash_utxos(&position.get_script_pubkey())?
        .first()
        .cloned())
}

/// Anyone sweeps a lapsed position (full debt outstanding) into the claim.
pub(super) fn lapse(context: &simplex::TestContext, posted: &Posted, position: &PositionParametersV5, sweeper: &Signer) -> anyhow::Result<()> {
    let created = SwaptionPositionV5::new_filled(*position);
    let utxo = position_utxo_at(context, &created)?.ok_or_else(|| anyhow::anyhow!("position utxo missing"))?;
    let mut ft = FinalTransaction::new();
    created.attach_lapse(&mut ft, utxo, posted.claim_script.clone());
    sweeper.broadcast(&ft)?.wait()?;
    Ok(())
}

pub(super) fn claim_utxos(context: &simplex::TestContext, posted: &Posted) -> anyhow::Result<Vec<UTXO>> {
    Ok(context.get_default_provider().fetch_scripthash_utxos(&posted.claim_script)?)
}

pub(super) fn claim_balance(context: &simplex::TestContext, posted: &Posted, asset: AssetId) -> anyhow::Result<u64> {
    Ok(claim_utxos(context, posted)?
        .iter()
        .filter(|u| u.explicit_asset() == asset)
        .map(|u| u.explicit_amount())
        .sum())
}

/// The lender empties its claim script with the lender token at input 0.
pub(super) fn withdraw_claims(context: &simplex::TestContext, lender: &Signer, posted: &Posted) -> anyhow::Result<()> {
    let token_utxo = lender.get_utxos_asset(posted.lender_token)?[0].clone();
    let utxos = claim_utxos(context, posted)?;
    anyhow::ensure!(!utxos.is_empty(), "nothing to claim");

    let mut ft = FinalTransaction::new();
    ft.add_input(PartialInput::new(token_utxo), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(lender.get_address().script_pubkey(), 1, posted.lender_token));
    for utxo in utxos {
        let asset = utxo.explicit_asset();
        let amount = utxo.explicit_amount();
        posted.claim.attach_withdraw(&mut ft, utxo);
        ft.add_output(PartialOutput::new(lender.get_address().script_pubkey(), amount, asset));
    }
    lender.broadcast(&ft)?.wait()?;
    Ok(())
}

pub(super) fn wallet_balance(signer: &Signer, asset: AssetId) -> anyhow::Result<u64> {
    Ok(signer.get_utxos_asset(asset)?.iter().map(|u| u.explicit_amount()).sum())
}
