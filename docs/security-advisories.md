# Security advisories — accepted-risk register

`cargo audit` runs in CI (`.github/workflows/ci.yml`, `audit` job) and must
stay green. Advisories we cannot fix with `cargo update` — because the
vulnerable version is pinned by the `polkadot-sdk` `stable2412` dependency
tree — are documented here and ignored with justification in
`.cargo/audit.toml`. This register is re-evaluated on every SDK bump.

## Pinning paths

| crate@version | pinned by | advisories |
|---|---|---|
| `h2 0.3.27` | `hyper 0.14` → `sc-offchain` | RUSTSEC-2026-0258 |
| `idna 0.2.3` / `0.4.0` | `url` chains in `sc-*`/`libp2p` | RUSTSEC-2024-0421 |
| `quinn-proto 0.10.6` | `libp2p-quic` / `litep2p` → `sc-network` | RUSTSEC-2026-0037, -0185, -0338, -0339, -0342 |
| `ring 0.16.20` | `rustls`/`webpki`/`libp2p-noise` | RUSTSEC-2025-0009, -0010 |
| `rustls-webpki 0.101.7` | `rustls 0.21` → `libp2p`/`quinn` | RUSTSEC-2026-0098, -0099, -0104 |
| `wasmtime 8.0.1` | `sc-executor-wasmtime` | 20 advisories, see below |

## Exposure assessment

- **quinn-proto DoS family** — the highest real exposure: crafted QUIC
  packets against a public P2P listener can panic/exhaust a node. These are
  DoS (availability), not remote code execution. Mitigations available now:
  keep the QUIC port behind rate-limiting (the operator guide documents
  `--listen-addr` and firewall posture); partially cleared by the stable2412 bump; full fix needs a later SDK line.
- **wasmtime 8.0.1** — the Substrate executor runs core-wasm MVP guests with
  Cranelift only: no component model, no WASI, no Winch, no shared memory,
  single engine. Most advisories (component transcoding, WASI, Winch,
  pooling allocator, jit-debug) never reach our path. Executor-relevant:
  - RUSTSEC-2026-0087 (Cranelift x86-64 `f64x2.splat` OOB/segfault) and
    RUSTSEC-2023-0091 (`i64x2.shr_s` miscompilation) — the guest runtime is
    a deterministic, reviewed artifact we produce ourselves; there is no
    untrusted-wasm input path, so exploitation requires a malicious runtime
    already trusted by the chain. Still tracked: the fix is the SDK bump.
  - RUSTSEC-2026-0096 (aarch64 miscompile) — documented x86_64-only support.
- **h2 0.3 / rustls 0.20 / rustls-webpki 0.101 / idna** — exposure only via
  offchain-worker HTTP/TLS and URL parsing; the runtime has no OCWs. P2P TLS
  uses noise keys, not X.509.
- **ring 0.16.20** — AES overflow-check panic requires a debug/overflow-
  checking build profile; release nodes are unaffected by construction.

## Revisit policy

1. SDK branch bumps are the single fix for this whole
   register — tracked as a P1 roadmap item in `docs/production-plan.md`.
   Re-run `cargo update` + clear entries that no longer apply at each bump.
2. If an entry's pinning crate releases a semver-compatible fix, remove the
   ignore and `cargo update` immediately — CI will catch a stale ignore.
3. New ignores require: the pinning path identified, an exposure note here,
   and a reviewer ack on the PR (don't blanket-ignore).

## stable2412 bump (2026-10)

Moving `stable2412` → `stable2412` dropped ~35 flagged IDs — the whole
rustls-0.20/webpki-0.22/quinn-0.9 families plus old `idna`/`h2` duplicates.
New accepted entries added for the 2412 tree: hickory-proto 0.25 DNS
(litep2p → sc-network; production listener), rustls-webpki 0.102
(subxt/jsonrpsee — dev/codegen path only), ruzstd 0.5 (smoldot codegen,
build-time only), plus ~15 unmaintained-crate warnings (bincode, derivative,
fxhash, instant, libsecp256k1, ...).
