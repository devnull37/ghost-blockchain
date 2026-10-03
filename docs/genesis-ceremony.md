# Genesis Ceremony — Public Testnet

What a real (non-dev) Ghost chainspec must contain and how it's produced.
This is the checklist for cutting `chainspecs/testnet.json` when the launch
gate in `docs/testnet-readiness.md` is green.

## Differences from `chainspecs/local.json`

| Field | `local.json` (dev) | Public testnet spec |
|---|---|---|
| `id` | `ghost-local` | `ghost-testnet-N` (increment per relaunch) |
| `chainType` | `Local` | `Live` |
| `bootNodes` | localhost placeholder | ≥2 operator-controlled bootnodes with DNS + stable peer ids |
| `telemetryEndpoints` | none | org telemetry sink |
| `balances` | dev accounts with ~1.15e18 | the allocation table below |
| `ghostConsensus.stakers` | Alice+Bob @1 000 GHOST | launch validators, real keys |
| `initial_validators` | Alice+Bob | same accounts as stakers |
| `session.keys` | dev seeds | generated per launch validator (or empty — validators `set_keys` after genesis; an empty key set means the M-4 fallback seats `initial_validators` with no keys, so pre-seeding is preferred) |
| `sudo` | Alice | **absent** or a launch-ops multisig — never a dev key |
| `properties` | none | `ss58Format`, `tokenSymbol`, `tokenDecimals` — see below |

## Allocation table (template)

Must be decided before the spec is cut; every entry is
`(ss58, planck)`. Suggested buckets for a testnet (unit = 10^12 planck):

| Bucket | Share | Rationale |
|---|---|---|
| Launch validators (bonded) | each ≥ `MinStake` × margin (recommend ≥ 10× to survive a partial slash) | committee must reach `MaxValidators`-bounded quorum at genesis |
| Faucet reserve | large (e.g. 20–30% of supply) | public onboarding |
| Treasury/ops | per org policy | infra + rewards reserve |

## SS58 prefix decision

The runtime currently uses the Substrate default prefix (42 — "generic
Substrate" address space shared with every unprefixed chain). A real
network registers a unique prefix in the SS58 registry
(`w3f/ss58-registry`) and sets it via `properties.ss58Format` in the spec
plus the runtime's `SS58Prefix`. Options:

1. **Keep 42 for testnet**, document that addresses are shared-namespace —
   acceptable for a disposable testnet, wrong for mainnet.
2. **Register a prefix** before the public testnet (trivial process, ~1 PR
   to the registry); then keys/addresses are unambiguous.

Recommendation: register before mainnet; acceptable to launch testnet on
42 with the decision documented here.

## Cutting the spec — `scripts/genesis-generate.sh`

The launcher is `scripts/genesis-generate.sh` (no compiled-in preset —
keys are ceremony output, never constants in the binary):

1. Each launch validator generates session keys offline and publishes
   `(account sr25519, grandpa ed25519, im_online sr25519)` + bonded
   account to the coordinator. Addresses/keys are *public* material; the
   secrets stay with each operator.
2. Coordinator fills `chainspecs/ghost-testnet-params.example.json` into
   a real params file (64-hex public keys; stakes in planck; `root` =
   launch-ops key or `null`).
3. `GENESIS_PARAMS=<params.json> rtk scripts/genesis-generate.sh chainspecs`
   produces `<id>.json` + `<id>-raw.json` + `SHA256SUMS.txt`. The script
   embeds the built runtime wasm, converts keys to SS58, and fails early
   on malformed input — `build-spec --raw` executing the genesis builder
   is itself the instantiation check.
4. Every participant recomputes the raw spec hash locally and attests it
   matches before the network boots — that hash IS the chain.
5. Commit the params + specs; publish bootnode peer ids.

## Hard requirements before a public spec is real

- No `pallet-template` anywhere (done — removed at spec 102).
- `sudo` either absent or a documented launch-ops key with a published
  removal plan (never a dev seed).
- Every `stakers` entry = an account the operator actually controls; a
  genesis validator whose key is lost just burns a committee seat.
- `spec_version` in the embedded Wasm matches `VERSION` at cut time.
