//! Regtest setup for the Swaption position covenant, v4.
//!
//! One default signer plays issuer + borrower (it funds the fill and holds the
//! borrower NFT); a random signer is the lender (holds the lender NFT). The
//! lender is paid at the CLAIM script of its token — exercise cash and lapsed
//! collateral both land there and the lender withdraws with the token.

use simplex::signer::Signer;
use simplex::simplicityhl::elements::{AssetId, Script, Txid};
use simplex::transaction::partial_input::IssuanceInput;
use simplex::transaction::{
    FinalTransaction, PartialInput, PartialOutput, RequiredSignature, UTXO,
};

use lending_contracts::programs::program::SimplexProgram;
use lending_contracts::programs::swaption_claim::SwaptionClaim;
use lending_contracts::programs::swaption_lending_v4::{
    PositionParametersV4, PositionTerms, SwaptionPositionV4, script_hash,
};
use lending_contracts::utils::get_random_seed;

use super::common::issuance::issue_asset;
use super::common::wallet::split_first_signer_utxo;

pub(super) const CASH_SUPPLY: u64 = 1_000_000;
/// Last look opens this many blocks before expiry in the scenarios.
pub(super) const LAST_LOOK_BLOCKS: u32 = 10;

pub(super) struct Fill {
    pub txid: Txid,
    /// The venue's last-look signer (its address script is in the terms).
    pub venue: Signer,
    pub position: SwaptionPositionV4,
    pub parameters: PositionParametersV4,
    /// The lender's claim script — where the position pays the lender.
    pub lender_script: Script,
    pub claim: SwaptionClaim,
}

/// Issue the cash asset, give the lender some cash + L-BTC for fees, and
/// fill one position with `terms`. Returns the filled position.
pub(super) fn fill_position(
    context: &simplex::TestContext,
    lender: &Signer,
    terms: PositionTerms,
) -> anyhow::Result<Fill> {
    let signer = context.get_default_signer();
    let network = *context.get_network();

    split_first_signer_utxo(context, vec![5000, 10000, 20000]);

    let cash_asset_id = issue_asset(context, CASH_SUPPLY)?;
    fund_lender(context, lender, cash_asset_id, CASH_SUPPLY / 10)?;
    // The venue: cash to pay the lender on a last look, L-BTC for fees.
    let venue = context.random_signer();
    fund_lender(context, &venue, cash_asset_id, CASH_SUPPLY / 10)?;

    let collateral_asset_id = network.policy_asset();

    let collateral_utxo = signer.get_utxos_filter(
        &|utxo| {
            utxo.explicit_asset() == collateral_asset_id
                && utxo.explicit_amount() >= terms.collateral_amount
        },
        &|_| true,
    )?[0]
        .clone();
    let issuance_utxo = signer.get_utxos_filter(
        &|utxo| {
            utxo.explicit_asset() == collateral_asset_id
                && utxo.outpoint != collateral_utxo.outpoint
        },
        &|_| true,
    )?[0]
        .clone();

    let nfts_entropy = get_random_seed();

    let mut ft = FinalTransaction::new();

    let borrower_nft = ft.add_issuance_input(
        PartialInput::new(collateral_utxo),
        IssuanceInput::new_issuance(1, 0, nfts_entropy),
        RequiredSignature::NativeEcdsa,
    );
    let lender_nft = ft.add_issuance_input(
        PartialInput::new(issuance_utxo),
        IssuanceInput::new_issuance(1, 0, nfts_entropy),
        RequiredSignature::NativeEcdsa,
    );

    let claim = SwaptionClaim::new(lender_nft.asset_id, network);
    let lender_script = claim.get_script_pubkey();

    let parameters = PositionParametersV4 {
        collateral_asset_id,
        cash_asset_id,
        borrower_nft_asset_id: borrower_nft.asset_id,
        lender_nft_asset_id: lender_nft.asset_id,
        lender_payout_script_hash: script_hash(&lender_script),
        borrower_payout_script_hash: script_hash(&signer.get_address().script_pubkey()),
        last_look_script_hash: script_hash(&venue.get_address().script_pubkey()),
        last_look_height: terms.expiry_height - LAST_LOOK_BLOCKS,
        terms,
        network,
    };

    let position = SwaptionPositionV4::new_filled(parameters);

    position.attach_fill(
        &mut ft,
        signer.get_address().script_pubkey(),
        lender.get_address().script_pubkey(),
    );

    let receipt = signer.broadcast(&ft)?;
    receipt.wait()?;

    Ok(Fill {
        txid: receipt.txid(),
        venue,
        position,
        parameters,
        lender_script,
        claim,
    })
}

pub(super) fn fund_lender(
    context: &simplex::TestContext,
    lender: &Signer,
    cash_asset_id: AssetId,
    cash_to_send: u64,
) -> anyhow::Result<()> {
    let signer = context.get_default_signer();

    let cash_utxo = signer.get_utxos_asset(cash_asset_id)?[0].clone();
    let policy_utxo = signer.get_utxos_asset(context.get_network().policy_asset())?[0].clone();

    let cash_utxo_amount = cash_utxo.explicit_amount();
    let policy_amount_to_send = policy_utxo.explicit_amount() / 2;

    let mut ft = FinalTransaction::new();

    ft.add_input(PartialInput::new(cash_utxo), RequiredSignature::NativeEcdsa);
    ft.add_input(PartialInput::new(policy_utxo), RequiredSignature::NativeEcdsa);

    ft.add_output(PartialOutput::new(
        lender.get_address().script_pubkey(),
        cash_to_send,
        cash_asset_id,
    ));
    ft.add_output(PartialOutput::new(
        lender.get_address().script_pubkey(),
        policy_amount_to_send,
        context.get_network().policy_asset(),
    ));
    if cash_utxo_amount > cash_to_send {
        ft.add_output(PartialOutput::new(
            signer.get_address().script_pubkey(),
            cash_utxo_amount - cash_to_send,
            cash_asset_id,
        ));
    }

    signer.broadcast(&ft)?.wait()?;

    Ok(())
}

pub(super) fn position_utxo(
    context: &simplex::TestContext,
    position: &SwaptionPositionV4,
) -> anyhow::Result<Option<UTXO>> {
    let provider = context.get_default_provider();
    Ok(provider
        .fetch_scripthash_utxos(&position.get_script_pubkey())?
        .first()
        .cloned())
}

/// Borrower (default signer) exercises `amount`, paying the lender's claim
/// script. Returns Ok(()) once confirmed.
pub(super) fn exercise(
    context: &simplex::TestContext,
    fill: &mut Fill,
    amount: u64,
) -> anyhow::Result<()> {
    let borrower = context.get_default_signer();
    let params = fill.parameters;

    let utxo = position_utxo(context, &fill.position)?.expect("position utxo must exist");
    let borrower_nft_utxo = borrower.get_utxos_asset(params.borrower_nft_asset_id)?[0].clone();
    let cash_utxo = borrower.get_utxos_filter(
        &|u| u.explicit_asset() == params.cash_asset_id && u.explicit_amount() >= amount,
        &|_| true,
    )?[0]
        .clone();
    let cash_utxo_amount = cash_utxo.explicit_amount();

    let current_debt = fill.position.get_remaining_debt();
    let released = params.terms.released_by(current_debt, amount);
    let is_partial = amount < current_debt;

    let mut ft = FinalTransaction::new();

    ft.add_input(PartialInput::new(borrower_nft_utxo), RequiredSignature::NativeEcdsa);
    if is_partial {
        ft.add_output(PartialOutput::new(
            borrower.get_address().script_pubkey(),
            1,
            params.borrower_nft_asset_id,
        ));
    }

    fill.position
        .attach_exercise(&mut ft, utxo, amount, fill.lender_script.clone());

    ft.add_input(PartialInput::new(cash_utxo), RequiredSignature::NativeEcdsa);

    ft.add_output(
        PartialOutput::new(
            borrower.get_address().script_pubkey(),
            released,
            params.collateral_asset_id,
        )
        .with_blinding_key(borrower.get_blinding_public_key()),
    );
    if cash_utxo_amount > amount {
        ft.add_output(PartialOutput::new(
            borrower.get_address().script_pubkey(),
            cash_utxo_amount - amount,
            params.cash_asset_id,
        ));
    }

    borrower.broadcast(&ft)?.wait()?;

    Ok(())
}

/// Every coin sitting at the lender's claim script.
pub(super) fn claim_utxos(context: &simplex::TestContext, fill: &Fill) -> anyhow::Result<Vec<UTXO>> {
    Ok(context
        .get_default_provider()
        .fetch_scripthash_utxos(&fill.claim.get_script_pubkey())?)
}

/// What the claim script holds of `asset`.
pub(super) fn claim_balance(context: &simplex::TestContext, fill: &Fill, asset: AssetId) -> anyhow::Result<u64> {
    Ok(claim_utxos(context, fill)?
        .iter()
        .filter(|u| u.explicit_asset() == asset)
        .map(|u| u.explicit_amount())
        .sum())
}

/// The lender empties its claim script: the lender token is input 0, every
/// claim coin follows, everything goes back to the lender's wallet.
pub(super) fn withdraw_claims(context: &simplex::TestContext, lender: &Signer, fill: &Fill) -> anyhow::Result<()> {
    let params = fill.parameters;
    let nft_utxo = lender.get_utxos_asset(params.lender_nft_asset_id)?[0].clone();
    let utxos = claim_utxos(context, fill)?;
    anyhow::ensure!(!utxos.is_empty(), "nothing to claim");

    let mut ft = FinalTransaction::new();
    ft.add_input(PartialInput::new(nft_utxo), RequiredSignature::NativeEcdsa);
    ft.add_output(PartialOutput::new(
        lender.get_address().script_pubkey(),
        1,
        params.lender_nft_asset_id,
    ));
    for utxo in utxos {
        let asset = utxo.explicit_asset();
        let amount = utxo.explicit_amount();
        fill.claim.attach_withdraw(&mut ft, utxo);
        ft.add_output(PartialOutput::new(lender.get_address().script_pubkey(), amount, asset));
    }

    lender.broadcast(&ft)?.wait()?;
    Ok(())
}

pub(super) fn wallet_balance(signer: &Signer, asset: AssetId) -> anyhow::Result<u64> {
    Ok(signer
        .get_utxos_asset(asset)?
        .iter()
        .map(|u| u.explicit_amount())
        .sum())
}
