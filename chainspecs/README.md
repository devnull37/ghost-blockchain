# Chainspecs

Published chain specifications. Regenerate from the matching binary:

```bash
ghost-node build-spec --chain local > chainspecs/local.json
```

## `local.json` — development / CI spec

- `name`: Ghost Local Testnet, `id`: ghost-local, `chainType`: Local.
- Genesis committee: Alice + Bob (well-known dev keys, `stakers` 1 000 GHOST
  each, `initial_validators` both).
- `sudo`: Alice — **dev only, must not appear in any public spec**.
- Session keys: dev seed keys for Alice/Bob.
- Used by `e2e-ghost.sh`, `e2e-faults.sh`, `soak-ghost.sh`, and CI.

**This spec is for disposable chains only.** Its keys are public knowledge —
any real network must be generated from a different preset with its own
allocation, committee, and no dev-key sudo. See `docs/genesis-ceremony.md`
for what a public launch spec requires.
