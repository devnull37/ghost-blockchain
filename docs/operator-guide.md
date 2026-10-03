# Operator Guide — Validators and Miners

How to run a Ghost node in either role. All on-chain parameter values are in
`docs/economic-parameters.md`.

## Roles

- **Miner**: produces PoW blocks. Needs `--mine` and a `--miner-coinbase`
  account. **No session keys, no stake, no committee membership required** —
  anyone can mine; attribution comes from the `pow_` pre-runtime digest.
- **Validator**: votes in the GRANDPA committee that finalizes blocks.
  Needs bonded stake ≥ `MinStake`, session keys, a registered ML-DSA-87 key
  (`RequirePqcKey`), and a `validate()` call to join the candidate set.

The two roles are independent: a node can mine, validate, both, or neither
(a plain full node). In `--chain local`, Alice and Bob are pre-seeded as
staked validators with generated session keys.

## Running a miner

```bash
ghost-node \
  --chain local \
  --base-path /var/lib/ghost/miner1 \
  --mine --mining-threads 4 \
  --miner-coinbase <YOUR SS58 ADDRESS> \
  --bootnodes /ip4/<BOOTNODE IP>/tcp/<PORT>/p2p/<BOOTNODE PEER ID> \
  --rpc-port 9944 --port 30333
```

- `--mining-threads` is per-node grind threads; do not oversubscribe the
  box (a 2-vCPU box should use ≤2).
- Block authoring is self-scheduling: the worker grinds until a seal meets
  the current `next_difficulty` (see `ghost_getConsensusMode` RPC).
- Your 40% author share mints straight to the coinbase address — verify with
  `system_account` / `account_nextIndex` RPCs. No claim transaction needed.

## Running a validator

Four steps, in order. Missing any one silently keeps you out of the set —
`select_validators` filters on every boundary.

### 1. Bond stake

```text
GhostConsensus::bond(amount)      // amount >= MinStake on first bond
GhostConsensus::bond_extra(more)  // top up later, any amount
```

The bond is a `pallet_balances` hold on your account — funds stay yours but
locked. `unbond` queues chunks; `withdraw_unbonded` releases them after
`UnbondingPeriod` (14 days).

### 2. Register session keys

The committee needs GRANDPA + im-online keys under your account:

```bash
# Preferred: let the node generate both key types into its own keystore
curl -H "Content-Type: application/json" \
  -d '{"id":1,"jsonrpc":"2.0","method":"author_rotateKeys","params":[]}' \
  http://localhost:9944
# -> 0x... (concatenated public keys); submit as
#    Session::set_keys(keys, proof=0x) or ghost-specific helper UI
```

Or inject known keys: `author_insertKey` with `keyType` `gran`/`imon`.
`set_keys` writes `NextKeys`; they take effect at the next session boundary
(20 blocks) — selection reads `NextKeys` via `SessionKeysLookup`.

**Never run two nodes holding the same authority keys.** That is a GRANDPA
equivocation: peers auto-report it on-chain and `on_offence` slashes your
bond + chills you (`e2e-faults.sh` scenario C proves this end to end).

### 3. Register a PQC key (`RequirePqcKey = true`)

```text
GhostPqc::register_pqc_key(public_key, proof_of_possession)
```

- `public_key`: your ML-DSA-87 (FIPS-204) public key.
- `proof_of_possession`: ML-DSA-87 signature over
  `b"GHOST-PQC-POP" || your_account_id` from the matching secret key.
- Rotating: `revoke_pqc_key` then `register_pqc_key` again. **Warning:** a
  seated validator who revokes keeps their seat only until the next
  selection boundary — register the new key immediately.
- `pqc_attest(block_hash, signature)` is available to bonded validators for
  informational block attestations.

### 4. Join the candidate set

```text
GhostConsensus::validate()
```

Requires: bonded ≥ `MinStake`, session keys registered, PQC key registered.
Then wait for the next session boundary — `select_validators` picks the top
`MaxValidators` candidates by stake (tie-broken by account id).

### Leaving

`GhostConsensus::chill()` — drops you from `Candidates`. The seated
committee keeps you until the next session boundary by design (mid-session
removal would desync GRANDPA); you stop being selected afterward. Then
`unbond` + `withdraw_unbonded` to recover stake.

## What gets you slashed

| Offence | How it's detected | Consequence |
|---|---|---|
| GRANDPA equivocation (two nodes, same keys) | peers auto-report via offchain unsigned tx | fraction of bond + all unbonding chunks burned; chilled |
| im-online unresponsiveness | missing heartbeats/authored blocks in a session | same handler, per-offence fraction |

Unbonding does **not** dodge a pending slash — chunks stay slashable until
`withdraw_unbonded`.

## Operational checklist before calling a validator "production"

- [ ] `--node-key` persisted (stable libp2p identity) and backed up.
- [ ] Keystore on the validator host only; never duplicated to a second
      live node (equivocation = slash).
- [ ] `--prometheus-port` scraped; alert on `finalized` stalling while
      `best` advances (that's a committee problem — scenario B semantics).
- [ ] Base path on persistent storage; restart-resume is designed-in but
      the DB must survive.
- [ ] Bond ≥ MinStake + buffer; a partial slash below `MinStake` drops you
      from the next selection.
- [ ] PQC key registered before `validate()`; rotation plan documented.
