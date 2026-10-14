# Security Policy

> **Status: draft skeleton.** Items marked `PLACEHOLDER` are intentionally
> unresolved until the maintainers confirm them; do not treat any address,
> key, or SLA below as live yet.

## Reporting a vulnerability

**Do not open a public GitHub issue, PR, or discussion for an undisclosed
security vulnerability.**

Report privately to:

- **Email:** `security@PLACEHOLDER.invalid` *(PLACEHOLDER — replace with the
  project's monitored security inbox before public disclosure is invited)*
- **PGP:** `PLACEHOLDER` *(no key published yet — ask for an encrypted channel
  in your initial report if the content is sensitive)*

Please include: affected component/commit, impact, a minimal reproduction or
proof-of-concept, and whether you believe the issue is exploitable on a live
network vs. only in a development configuration.

## Scope

**In scope**

- Consensus and block production: PoW seal verification, difficulty
  retargeting, fork choice, PoS validator selection, GRANDPA finality.
- Runtime logic in `runtime/` and pallets in `pallets/` (balances, staking,
  slashing, rewards, authoring).
- P2P/network layer (peer handling, sync, DoS resistance).
- JSON-RPC surface exposed by `ghost-node`.
- Key management, session keys, and on-chain PQC key registration.
- This repository's build and release pipeline (`Dockerfile`, CI, scripts).

**Out of scope**

- Vulnerabilities in upstream dependencies — report them upstream
  (polkadot-sdk, parity crates); we track advisories via `cargo audit`.
- Attacks requiring physical access, social engineering, or compromise of a
  reporter-controlled node.
- The `--dev` / `--chain local` development configurations, which are
  intentionally insecure (dev keys, `--unsafe-*` flags, sudo).
- Denial-of-service against public infrastructure you do not operate.

## Response targets

*(PLACEHOLDER SLAs — to be ratified by maintainers)*

| Step | Target |
| --- | --- |
| Acknowledge receipt | within 72 hours |
| Severity triage + first assessment | within 7 days |
| Fix or mitigation ETA communicated | within 14 days |
| Coordinated public disclosure | after a fix ships, or 90 days, whichever is first |

We will credit reporters in release notes unless they prefer to remain
anonymous.

## Safe harbor

We support good-faith security research. If you:

- make a good-faith effort to avoid privacy violations, data destruction, and
  service degradation,
- test only against nodes and networks you operate (e.g. a local devnet —
  `docker compose up` gives you one), and
- give us a reasonable window to remediate before public disclosure,

then we will not pursue or support legal action against you for your
research, and we will work with you to understand and resolve the issue
quickly.

## Bug bounty

`PLACEHOLDER` — no bounty program exists yet. Reports are still welcome and
will be handled under the policy above.
