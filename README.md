# lending_contracts

Simplicity covenants for Swaption lending, on the Simplex toolchain.

- `simf/swaption_lending.simf` — **the Swaption position covenant** (v1): a
  sale with a buyback right. Exercise (full or partial, cash paid straight
  to the lender script fixed at fill) and lapse (lender takes what is left
  after `EXPIRY_HEIGHT`). One storage slot: remaining debt. Rust wrapper in
  `src/programs/swaption_lending/`. Spec: Dropbox `Swaption/Lending/06 Product
  spec v1 - sale and buyback.md`; design note `docs/LENDING-DESIGN.md`.
- `simf/lending.simf`, `asset_auth*.simf`, `issuance_factory.simf`,
  `script_auth.simf` — the Blockstream reference, vendored 2026-09-02 from
  `BlockstreamResearch/simplicity-lending` (MIT OR Apache-2.0), kept for
  comparison and for the helper wrappers. Not used by the Swaption server.

## Build

`src/artifacts/` is GENERATED and gitignored. Regenerate after any `.simf`
change (Simplex 0.0.5 binary at `~/toolchain/simplex/simplex` on the build host):

    cd lending_contracts && ~/toolchain/simplex/simplex build
    cargo build -p lending-contracts
    cargo test -p lending-contracts --lib          # unit tests, no node needed

## Regtest scenarios (`tests/swaption_lending/`)

Need `elementsd` and `electrs` (esplora/liquid flavour). On the build host they
are in `~/toolchain/regtest-bin/` (same releases the reference CI pins:
elements 23.3.1, electrs `027e38d3` liquid). Then, from `lending_contracts/`:

    ELEMENTSD_EXEC=~/toolchain/regtest-bin/elements-23.3.1/bin/elementsd \
    ELECTRS_LIQUID_EXEC=~/toolchain/regtest-bin/electrs \
    ~/toolchain/simplex/simplex test --target swaption_lending

Scenarios: full exercise; three odd partial exercises then full (the case
the reference contract fails on); wrong payout script; short payout;
over-release on partial; exercise without the borrower NFT; lapse after
expiry; lapse after a partial; lapse before expiry; lapse without burning
the lender NFT.

The build host runs all of this inside `rust:1-slim` with a memory cap and
the shared `~/dev/swaption_be/target`; see `deploy/lc-testnet/README.md` for
the docker invocation.
