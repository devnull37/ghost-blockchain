# Adversarial Failure-Mode Report

Results of `scripts/e2e-faults.sh` — the adversarial gate covering the three
failure modes that matter for a PoW + committee chain: loss of all mining,
loss of committee members, and a same-authority equivocation.

Each scenario lists the *expected* behavior from the design docs, then what
was *observed* on this box (spec_version 102 binary).

## Scenario A — all miners halt

**Setup:** Alice + Bob validate (committee, no `--mine`); one keyless miner
authors all blocks. Baseline reached (best ≥ 6, finalized ≥ 3). Miner killed.

**Expected:** best head freezes. GRANDPA finalizes up to the frozen best and
stops — never past it (nothing to finalize). Miner restart → both resume.

**Observed:** _pending first full run_

## Scenario B — half the committee offline

**Setup:** Alice validates + mines; Bob absent. N=2 committee requires both
votes (⌈2N/3⌉ = 2).

**Expected:** PoW best keeps advancing (mining is committee-independent);
finality cannot move past genesis. Bob joins → finality resumes.

**Observed:** _pending first full run_

## Scenario C — equivocation → slash

**Setup:** two processes both hold Alice's GRANDPA keys (`--alice` ×2).
Their divergent votes are provably attributable to the same authority —
peers' offchain workers auto-submit equivocation reports.

**Expected:** `GhostConsensus::SlashRecords` non-empty naming Alice;
`Bonded(Alice)` decreases by the offence's slash fraction; Alice leaves the
candidate set at the next selection (deferred removal by design — the
seated committee is never mutated mid-session). Liveness continues.

**Observed:** _pending first full run_

## Known limitations of this gate

- Localhost network — no packet loss/latency injection; partition testing
  is a separate exercise (soak covers restart churn, not netsplit).
- Scenario A's "frozen best" read has a ~3 s settling window; a late-arriving
  in-flight block could in principle trip the overshoot assert — on
  localhost this has not been observed.
- The equivocation report lands via the honest nodes' offchain workers;
  a network where *every* node is malicious would not be covered by this
  scenario (threat-model: honest-majority assumption).
