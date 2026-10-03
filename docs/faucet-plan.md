# Testnet Faucet Plan

How public testnet users get GHOST without mining. Documented for launch;
implementation is a separate small project — no chain code changes needed.

## Requirement

A new user must be able to fund an address enough to:
1. cover `EXISTENTIAL_DEPOSIT` (0.001 GHOST),
2. bond `MinStake` (1 GHOST) if they want to validate,
3. pay transaction fees while experimenting.

## Recommended design — standalone drip bot (no pallet)

A small service (bots are the standard Substrate approach, e.g. the
`substrate-faucet-bot` pattern):

- Holds a faucet key funded from the testnet allocation's *faucet reserve*
  bucket (`docs/genesis-ceremony.md`).
- Endpoint (matrix/HTTP) takes an SS58 address, rate-limits per
  address + per identity (e.g. 100 GHOST / 24 h / address), submits a
  `balances.transfer_keep_alive`.
- Rate-limit state is off-chain (the pallet needs no drip logic — keeps the
  runtime clean and the policy tunable without a runtime upgrade).
- Abuse handling: captcha or proof-of-work-of-the-mild-kind (e.g. the
  requester mines one PoW-sealed block worth of hashes — thematic, and
  proves nothing except burn); faucet empty = operator top-up runbook
  item, not a chain event.

## Why not a faucet pallet

A `faucet.drip` extrinsic would put free-issuance rules into consensus —
an unnecessary attack surface (Sybil draining, weight accounting) for what
is a pure off-chain ops problem. The pallet route only earns its keep if
drips must be auditable on-chain, which for a testnet they don't.

## Alternative covered: mine your own

Because mining is permissionless (no stake/keys needed — only
`--mine` + `--miner-coinbase`), testnet users can always self-fund by
mining. The faucet exists for UX; the chain does not depend on it.
