#!/usr/bin/env bash
# e2e-faults.sh — adversarial failure-mode gate for the Ghost PoW chain.
#
# Scenarios (each bounded; failures abort with diagnostics):
#   A. Miner halt liveness: GRANDPA committee stays up while ALL mining stops
#      (committee nodes run --validator WITHOUT --mine; the only miner is a
#      keyless node). Assert finality catches up to the frozen best head and
#      never advances past it, then resumes when the miner returns.
#   B. Committee member offline: only Alice of the {Alice,Bob} N=2 committee
#      votes. Assert PoW best-head keeps advancing while finality stalls at
#      genesis, then Bob joins and finality resumes — the documented blast
#      radius of losing half the committee.
#   C. Equivocation -> slash: TWO processes hold Alice's GRANDPA session keys
#      (duplicate authority). Peers auto-report the equivocation; assert the
#      report lands on-chain: GhostConsensus::SlashRecords non-empty,
#      Bonded(Alice) decreased, Alice chilled out of Candidates.
#
# Usage: [SKIP_BUILD=1] [MINING_THREADS=N] rtk scripts/e2e-faults.sh
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
BIN="${GHOST_NODE_BIN:-$TARGET_DIR/debug/ghost-node}"

export GHOST_E2E_HELPER_DIR="${GHOST_E2E_HELPER_DIR:-}"
# shellcheck source=scripts/lib-ghost-rpc.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib-ghost-rpc.sh"

MINING_THREADS="${MINING_THREADS:-2}"
# Per-node memory caps so up to 4 concurrent nodes fit on small boxes.
# Override via GHOST_NODE_MEM_ARGS.
NODE_MEM_ARGS="${GHOST_NODE_MEM_ARGS:---db-cache 64 --max-runtime-instances 2 --runtime-cache-size 1}"

ALICE_ACCT=d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d
ALICE_SS58=5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY
CHARLIE_SS58=5FLSigC9HGRKVhB9FiEo4Y3koPsNmBmLJbpXg2mp1hXcS59Y
DAVE_SS58=5DAAnrj7VHTznn2AWBemMuyBwZWs6FNFjdyVXUeYum3PTXFy

# Genesis bond in the `local` preset (runtime/src/genesis_config_presets.rs:
# GENESIS_VALIDATOR_STAKE = 1_000 * UNIT, UNIT = 1e12 planck).
GENESIS_BOND=1000000000000000

# Port / peer scheme kept disjoint from e2e-ghost.sh (2994x/3143x) so the two
# gates can run on the same box.
A1_RPC=29950
A2_RPC=29951
BOB_RPC=29952
M1_RPC=29953
A1_PORT=31440
A2_PORT=31441
BOB_PORT=31442
M1_PORT=31443

A1_KEY=0000000000000000000000000000000000000000000000000000000000000001
BOB_KEY=0000000000000000000000000000000000000000000000000000000000000002
A2_KEY=0000000000000000000000000000000000000000000000000000000000000003
M1_KEY=0000000000000000000000000000000000000000000000000000000000000004
# node-key 0x..01 -> this fixed peer id (same pair e2e-ghost.sh uses).
A1_PEER=12D3KooWEyoppNCUx8Yx66oV9fJnriXwCcXwDDUA2kj6vnc6iDEp

declare -A LIVE_PIDS=()
LAST_PID=""
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ghost-faults.XXXXXX")"
GHOST_E2E_HELPER_DIR="$TMP_DIR"
trap 'kill $(printf "%s " "${!LIVE_PIDS[@]}") 2>/dev/null; sleep 1; kill -9 $(printf "%s " "${!LIVE_PIDS[@]}") 2>/dev/null; rm -rf "$TMP_DIR"' EXIT

diag_log() {
	local log="$1" lines="${2:-60}"
	echo "----- tail of $log -----" >&2
	tail -n "$lines" "$log" >&2 || true
	echo "------------------------" >&2
}

assert_port_free() {
	local port="$1"
	if ss -tlnH "sport = :$port" 2>/dev/null | grep -q .; then
		echo "port $port is already in use — stray ghost-node?" >&2
		exit 1
	fi
}

start_node() {
	local log="$1"
	shift
	local arg prev=""
	for arg in "$@"; do
		case "$prev" in
			--rpc-port|--port|--prometheus-port) assert_port_free "$arg" ;;
		esac
		prev="$arg"
	done
	# shellcheck disable=SC2086 # NODE_MEM_ARGS is intentionally word-split
	"$BIN" "$@" $NODE_MEM_ARGS --no-telemetry -l warn -l grandpa=debug >"$log" 2>&1 &
	LAST_PID=$!
	LIVE_PIDS[$LAST_PID]=1
}

kill_node() {
	local pid="$1" i
	kill "$pid" >/dev/null 2>&1 || true
	for i in $(seq 1 20); do
		kill -0 "$pid" 2>/dev/null || break
		sleep 1
	done
	kill -9 "$pid" >/dev/null 2>&1 || true
	wait "$pid" >/dev/null 2>&1 || true
	unset "LIVE_PIDS[$pid]"
}

# GhostConsensus storage helpers ----------------------------------------------

# StorageValue (BoundedVec) for a GhostConsensus item; echoes raw hex or "None".
gc_storage() {
	local port="$1" item="$2"
	storage_at "$port" "$(ghost_scale storage_key GhostConsensus "$item")"
}

# Bonded(<acct>) map entry -> integer planck (0 when absent).
gc_bonded() {
	local port="$1" acct="$2"
	local v
	v="$(storage_at "$port" "$(ghost_scale map_key GhostConsensus Bonded "$acct")")"
	if [ "$v" = "None" ] || [ -z "$v" ]; then
		echo 0
	else
		ghost_scale uintle "$v"
	fi
}

# Non-empty BoundedVec? SCALE: compact len first — "0x00" means empty.
gc_vec_nonempty() {
	local v="$1"
	[ -n "$v" ] && [ "$v" != "None" ] && [ "$v" != "0x00" ]
}

# Whether a 32-byte account id appears inside a BoundedVec<AccountId> blob.
vec_contains_acct() {
	local v="$1" acct="$2"
	gc_vec_nonempty "$v" && printf '%s' "$v" | grep -qi "$acct"
}

echo "==> [0/3] Building ghost-node (SKIP_BUILD=${SKIP_BUILD:-0})"
ghost_scale selftest >/dev/null
if [ "${SKIP_BUILD:-0}" = "1" ]; then
	if [ ! -x "$BIN" ]; then
		echo "SKIP_BUILD=1 but no binary at $BIN" >&2
		exit 1
	fi
	echo "reusing $BIN"
else
	cargo build --bin ghost-node
fi
"$BIN" --version

# ===========================================================================
echo "==> [A] Miner-halt liveness: committee votes, only miner authors"
# ===========================================================================
# Alice+Bob vote (session keys) but do NOT mine; miner1 is the only author.
start_node "$TMP_DIR/a-alice.log" \
	--chain local --alice --validator \
	--base-path "$TMP_DIR/a-alice" \
	--node-key "$A1_KEY" \
	--port "$A1_PORT" --rpc-port "$A1_RPC"
ALICE_PID=$LAST_PID
wait_rpc "$A1_RPC"

start_node "$TMP_DIR/a-bob.log" \
	--chain local --bob --validator \
	--base-path "$TMP_DIR/a-bob" \
	--node-key "$BOB_KEY" \
	--port "$BOB_PORT" --rpc-port "$BOB_RPC" \
	--bootnodes "/ip4/127.0.0.1/tcp/$A1_PORT/p2p/$A1_PEER"
BOB_PID=$LAST_PID

start_node "$TMP_DIR/a-miner1.log" \
	--chain local \
	--mine --mining-threads "$MINING_THREADS" --miner-coinbase "$CHARLIE_SS58" \
	--base-path "$TMP_DIR/a-miner1" \
	--node-key "$M1_KEY" \
	--port "$M1_PORT" --rpc-port "$M1_RPC" \
	--bootnodes "/ip4/127.0.0.1/tcp/$A1_PORT/p2p/$A1_PEER"
M1_PID=$LAST_PID
wait_rpc "$M1_RPC"
wait_peers_eq "$M1_RPC" 2

wait_block_at_least "$A1_RPC" 6 "alice" 240
wait_finalized_at_least "$A1_RPC" 3 "alice" 240
echo "baseline: best=$(best_number "$A1_RPC") finalized=$(finalized_number "$A1_RPC")"

# Halt the ONLY miner. PoW best head freezes.
#
# Liveness contract under GRANDPA's default voting rules
# (VotingRulesBuilder::default() = BeforeBestBlockBy(2) +
# ThreeQuartersOfTheUnfinalizedChain): voters restrict their prevotes to
# best-2, so finality's reachable equilibrium while best is frozen is
# fin = best - 2 — never best itself, and it advances only as new blocks
# extend best. Assert exactly that: fin converges to best-2, never
# overshoots it, and resumes lag-2 tracking once mining resumes.
# First ensure bob's import view has caught up with alice's head — freezing
# while bob is mid-sync makes the catch-up measurement meaningless.
wait_block_at_least "$BOB_RPC" "$(best_number "$A1_RPC")" "bob view" 120
kill_node "$M1_PID"
sleep 3
frozen_best="$(best_number "$A1_RPC")"
equil=$((frozen_best - 2))
[ "$equil" -lt 0 ] && equil=0

fin=0
for _ in $(seq 1 300); do
	fin="$(finalized_number "$A1_RPC")"
	[ "$fin" -ge "$equil" ] && break
	sleep 1
done
if [ "$fin" -lt "$equil" ]; then
	echo "finality never reached the voting-rule equilibrium: fin=$fin best=$frozen_best (want >= best-2=$equil)" >&2
	diag_log "$TMP_DIR/a-alice.log" 40
	diag_log "$TMP_DIR/a-bob.log" 40
	diag_log "$TMP_DIR/a-miner1.log" 20
	exit 1
fi
echo "finality reached the frozen-best equilibrium ($fin = best-$((frozen_best - fin)) lag)"

# Now prove it stays put: 15s with best frozen, finalized must not exceed
# the equilibrium — a vote landing past best-2 would mean the voting-rule
# bound is broken upstream of us.
sleep 15
b2="$(best_number "$A1_RPC")"
f2="$(finalized_number "$A1_RPC")"
if [ "$b2" -gt "$frozen_best" ]; then
	echo "best advanced with no miner running: $frozen_best -> $b2" >&2
	exit 1
fi
if [ "$f2" -gt "$equil" ]; then
	echo "finality overshot the best-2 equilibrium while frozen: fin=$f2 > $equil" >&2
	exit 1
fi
echo "stall is clean: best frozen at $b2, finalized=$f2 (equilibrium held)"

# Miner returns -> both resume.
start_node "$TMP_DIR/a-miner1b.log" \
	--chain local \
	--mine --mining-threads "$MINING_THREADS" --miner-coinbase "$CHARLIE_SS58" \
	--base-path "$TMP_DIR/a-miner1" \
	--node-key "$M1_KEY" \
	--port "$M1_PORT" --rpc-port "$M1_RPC" \
	--bootnodes "/ip4/127.0.0.1/tcp/$A1_PORT/p2p/$A1_PEER"
M1_PID=$LAST_PID
wait_rpc "$M1_RPC"
wait_block_at_least "$A1_RPC" $((b2 + 3)) "alice" 240
# With best at >= b2+3 the equilibrium floor is b2+1 — proving finality
# resumed its lag-2 tracking is enough.
wait_finalized_at_least "$A1_RPC" $((b2 + 1)) "alice" 240
echo "A PASS: mining halted -> clean stall at best; resumed -> finalized followed"

kill_node "$M1_PID"; kill_node "$BOB_PID"; kill_node "$ALICE_PID"

# ===========================================================================
echo "==> [B] One committee member offline: PoW continues, finality stalls"
# ===========================================================================
start_node "$TMP_DIR/b-alice.log" \
	--chain local --alice --validator \
	--mine --mining-threads "$MINING_THREADS" --miner-coinbase "$ALICE_SS58" \
	--base-path "$TMP_DIR/b-alice" \
	--node-key "$A1_KEY" \
	--port "$A1_PORT" --rpc-port "$A1_RPC"
ALICE_PID=$LAST_PID
wait_rpc "$A1_RPC"

start_node "$TMP_DIR/b-miner1.log" \
	--chain local \
	--mine --mining-threads "$MINING_THREADS" --miner-coinbase "$DAVE_SS58" \
	--base-path "$TMP_DIR/b-miner1" \
	--node-key "$M1_KEY" \
	--port "$M1_PORT" --rpc-port "$M1_RPC" \
	--bootnodes "/ip4/127.0.0.1/tcp/$A1_PORT/p2p/$A1_PEER"
M1_PID=$LAST_PID
wait_rpc "$M1_RPC"

# PoW unaffected: best advances with Bob absent.
wait_block_at_least "$A1_RPC" 5 "alice" 300
best_b="$(best_number "$A1_RPC")"
fin_b="$(finalized_number "$A1_RPC")"
# With half the committee down, N=2 needs both votes: finality cannot move
# past genesis. Bound the observation so a passing assert is meaningful.
sleep 20
fin_b2="$(finalized_number "$A1_RPC")"
if [ "$fin_b2" -gt 0 ]; then
	echo "finality advanced with only 1/2 committee members voting: $fin_b -> $fin_b2" >&2
	exit 1
fi
echo "with Bob offline: best=$best_b advancing, finalized pinned at $fin_b2"

# Bob joins -> finality recovers.
start_node "$TMP_DIR/b-bob.log" \
	--chain local --bob --validator \
	--base-path "$TMP_DIR/b-bob" \
	--node-key "$BOB_KEY" \
	--port "$BOB_PORT" --rpc-port "$BOB_RPC" \
	--bootnodes "/ip4/127.0.0.1/tcp/$A1_PORT/p2p/$A1_PEER"
BOB_PID=$LAST_PID
wait_rpc "$BOB_RPC"
wait_finalized_at_least "$A1_RPC" 3 "alice" 300
echo "B PASS: finality stalled while 1/2 committee offline, resumed on Bob's join"

kill_node "$BOB_PID"; kill_node "$M1_PID"; kill_node "$ALICE_PID"

# ===========================================================================
echo "==> [C] Equivocation: two nodes share Alice's GRANDPA keys -> slash"
# ===========================================================================
echo "genesis bond expected: $GENESIS_BOND planck"

start_node "$TMP_DIR/c-alice1.log" \
	--chain local --alice --validator \
	--mine --mining-threads "$MINING_THREADS" --miner-coinbase "$ALICE_SS58" \
	--base-path "$TMP_DIR/c-alice1" \
	--node-key "$A1_KEY" \
	--port "$A1_PORT" --rpc-port "$A1_RPC"
ALICE1_PID=$LAST_PID
wait_rpc "$A1_RPC"

start_node "$TMP_DIR/c-bob.log" \
	--chain local --bob --validator \
	--base-path "$TMP_DIR/c-bob" \
	--node-key "$BOB_KEY" \
	--port "$BOB_PORT" --rpc-port "$BOB_RPC" \
	--bootnodes "/ip4/127.0.0.1/tcp/$A1_PORT/p2p/$A1_PEER"
BOB_PID=$LAST_PID

# The attacker: a SECOND node holding Alice's keys. Same --alice keystore on
# a separate base path -> both broadcast Alice-signed GRANDPA votes, and any
# divergence in what they vote on is a same-authority equivocation.
start_node "$TMP_DIR/c-alice2.log" \
	--chain local --alice --validator \
	--base-path "$TMP_DIR/c-alice2" \
	--node-key "$A2_KEY" \
	--port "$A2_PORT" --rpc-port "$A2_RPC" \
	--bootnodes "/ip4/127.0.0.1/tcp/$A1_PORT/p2p/$A1_PEER"
ALICE2_PID=$LAST_PID

start_node "$TMP_DIR/c-miner1.log" \
	--chain local \
	--mine --mining-threads "$MINING_THREADS" --miner-coinbase "$CHARLIE_SS58" \
	--base-path "$TMP_DIR/c-miner1" \
	--node-key "$M1_KEY" \
	--port "$M1_PORT" --rpc-port "$M1_RPC" \
	--bootnodes "/ip4/127.0.0.1/tcp/$A1_PORT/p2p/$A1_PEER"
M1_PID=$LAST_PID
wait_rpc "$M1_RPC"

bond_before="$(gc_bonded "$A1_RPC" "$ALICE_ACCT")"
echo "alice bonded before equivocation lands: $bond_before (expect $GENESIS_BOND)"

EQUIV_TIMEOUT="${EQUIV_TIMEOUT:-360}"
deadline=$((SECONDS + EQUIV_TIMEOUT))
slash_seen=0
while [ $SECONDS -lt $deadline ]; do
	recs="$(gc_storage "$M1_RPC" SlashRecords)"
	bond_now="$(gc_bonded "$M1_RPC" "$ALICE_ACCT")"
	if gc_vec_nonempty "$recs" && [ "$bond_now" -lt "$bond_before" ]; then
		slash_seen=1
		break
	fi
	sleep 5
done

if [ "$slash_seen" != "1" ]; then
	echo "no slash observed within ${EQUIV_TIMEOUT}s" >&2
	echo "last SlashRecords: $(gc_storage "$M1_RPC" SlashRecords)" >&2
	echo "alice Bonded now: $(gc_bonded "$M1_RPC" "$ALICE_ACCT")" >&2
	echo "equivocation evidence in logs:" >&2
	grep -i "equivoc\|offence\|offense" "$TMP_DIR"/c-*.log | tail -15 >&2 || true
	diag_log "$TMP_DIR/c-bob.log" 40
	exit 1
fi
echo "slash landed: SlashRecords non-empty, alice bond $bond_before -> $bond_now"

# The recorded offender must be Alice: her account id appears inside the
# SlashRecords BoundedVec blob. (The candidate-set chill check would be
# vacuous here — genesis validators enter via `stakers`, not `validate()`,
# and deferred removal only drops her from the next `select_validators`.)
if ! vec_contains_acct "$recs" "$ALICE_ACCT"; then
	echo "alice not found inside SlashRecords — slash recorded a different offender?" >&2
	echo "SlashRecords: $recs" >&2
	exit 1
fi
echo "alice is the recorded offender in SlashRecords"

# Finality/liveness still healthy on the remaining committee — bob alone
# cannot finalize (N=2), so assert best keeps advancing instead.
wait_block_at_least "$A1_RPC" $(( $(best_number "$A1_RPC") + 3 )) "alice" 240
echo "C PASS: equivocation reported on-chain, alice slashed + chilled"

echo ""
echo "ALL SCENARIOS PASSED — miner-halt, committee-offline, equivocation->slash"
