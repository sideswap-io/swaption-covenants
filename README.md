# Swaption covenants

The Simplicity covenants behind [Swaption](https://swaption.io) lending on
Liquid: a **sale with a buyback right**, settled on chain, with no platform key.
A borrower sells L-BTC (or USDt, or DePix) to a lender now and keeps the right
to buy it back at a fixed price until expiry. There is no oracle, no margin
call, no liquidation and no default: if the right is not used, the lender
simply keeps what was sold.

This repository is the source of the programs a SideSwap wallet verifies before
it signs. The wallet-side verifier lives in
[`liquidconnect/liquidconnect-sdk`](https://github.com/liquidconnect/liquidconnect-sdk)
(`lc-wallet-core/src/lending.rs`) and pins the tapleaf hashes listed below; anyone
can rebuild them from this source and compare.

**Status: testnet.** Live on `paper.swaption.io` against Liquid testnet. Not yet
externally reviewed; do not use on mainnet before that review is published.

## The three programs

| program | source | tapleaf hash | what it holds |
|---|---|---|---|
| Position **v5** | [`simf/swaption_lending_v5.simf`](simf/swaption_lending_v5.simf) | `da43157b075c4c18c1f7354ada37455fdfbd7eb481d6762a9f1843896d659725` | the sold collateral, until it is bought back or lapses |
| Claim | [`simf/swaption_claim.simf`](simf/swaption_claim.simf) | `51f916310d382610bf7efd7b345d9641b3dc93917d2afb77289add911f3403e1` | a coin spendable by whoever spends one unit of the lender token as input 0 |
| Offer v1 | [`simf/swaption_offer.simf`](simf/swaption_offer.simf) | `881b1416b04e9fb47b6ecde9701880470713c37b5b99388da973d4e0d508f41f` | a lender's cash escrowed at post time, fillable by any borrower alone |

Earlier position programs still have live testnet positions and are kept for
their wrappers and tests: v1 (`swaption_lending.simf`), v2
(`41d218d4…`), v3 (`880d441e…`), v4 (`939c233f…`).

### Position v5 — sale with buyback right

State: one storage slot, the **remaining debt** (cash still owed to buy back
everything). The collateral the covenant must hold at any debt is derived from
cumulative paid debt, so a sequence of odd-sized partial buybacks never drifts
(the reference contract this started from fails on the third odd partial).

Spending paths:

- **Exercise** (full or partial): the holder of the borrower token pays
  `amount ≤ debt` of cash to the *lender's payout script fixed at fill*, the
  covenant releases collateral in proportion and continues at the new debt;
  a full exercise burns the token.
- **Lapse**: after `EXPIRY_HEIGHT`, anyone may sweep what is left to the
  lender's script.
- **Last look**: from `LAST_LOOK_HEIGHT` (500 blocks before expiry on
  mainnet), the venue whose coin is named in the terms may exercise a
  forgotten in-the-money right *in rounds*, paying the lender and the
  borrower's surplus less the venue fee; a partial round continues the
  position, the last one closes it.

Everything the covenant commits to is in the terms digest; the wallet rebuilds
the digest from the memo and refuses a template that does not match.

### Claim

Cash and collateral owed to a lender land here: whoever spends one unit of the
lender token as input 0 may take the coin. One lender token per wallet is the
claim key of every position it lends on and the cancel authority of its offers.

### Offer v1 — a lend offer escrowed on chain

A lender puts cash into this covenant with up to four quote rows (collateral,
expiry, sale price, buyback price). Slot 0 holds the terms digest, slot 1 the
remaining cash. Paths: **fill** (a borrower alone creates a v5 position from
the coin, pays the venue fee fixed in the terms, the coin continues with what
is left, or the leftover goes to the lender's claim), **cancel** (the lender
token at input 0 takes the cash back), **expire** (after the cutoff anyone may
return the cash to the lender's claim).

## Verify the hashes yourself

Toolchain: [Simplex](https://github.com/BlockstreamResearch/simplicity-contracts)
0.0.5 (`simplicityhl` 0.5, `simplicity-lang` 0.7) and stable Rust.

    simplex build                                     # writes src/artifacts/ (gitignored)
    cargo test --lib -- print_program_leaf --nocapture   # prints each program's tapleaf hash

Compare with the constants in `liquidconnect-sdk` `lc-wallet-core/src/lending.rs`.

## Regtest scenarios

`tests/swaption_lending_v5/` (21 scenarios), `tests/swaption_offer/` (24) and the
earlier versions run against a live Elements regtest through `simplex test`.
They need `elementsd` and an esplora-flavoured `electrs`:

    ELEMENTSD_EXEC=/path/to/elementsd ELECTRS_LIQUID_EXEC=/path/to/electrs \
    simplex test --target swaption_lending_v5

The scenarios cover every path and every refusal: full and partial exercise,
three odd partials to zero without drift, over-release, wrong or short payout,
lapse before expiry, last look before the window, lender paid short, venue coin
missing, partial last look over-releasing, offer fills below the minimum, fee
short or elsewhere, continuation tampered, cancel without the token, expire
before the cutoff.

## Lineage and licence

Started 2026-09-02 from Blockstream Research's
[`simplicity-lending`](https://github.com/BlockstreamResearch/simplicity-lending)
`crates/contracts` and the
[`simplicity-contracts`](https://github.com/BlockstreamResearch/simplicity-contracts)
reference crate (both MIT OR Apache-2.0). The reference programs
(`lending.simf`, `asset_auth*.simf`, `issuance_factory.simf`,
`script_auth.simf`) and their tests are kept verbatim for comparison; Swaption
does not use them.

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. Copyright (c) 2026 SideSwap Limited (Swaption) and contributors;
reference material copyright Blockstream Research.

## Security

Found something? Please write to security@sideswap.io before publishing. A
public review and bounty will be announced before any mainnet deployment.
