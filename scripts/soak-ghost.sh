#!/usr/bin/env bash
# soak-ghost.sh — long-run multi-node soak for the Ghost local testnet.
#
# Pre-testnet gate per docs/production-plan.md (P1 ops maturity): prove the
# chain stays healthy over time, across node restarts, and under mild
# adversity. Unlike scripts/e2e-local.sh (a boot-and-finality smoke test),
# this harness keeps a multi-node network running for a sustained duration
# while continuously checking liveness and finality invariants.
#
# Topology (--chain local; the GRANDPA committee is exactly {Alice, Bob} —
# N=2 so BOTH voters are required for finality, which is why only the
# keyless miners are ever restarted):
#
#   alice    — --alice --validator + --mine (session keys + PoW authoring)
#   bob      — --bob   --validator + --mine (session keys + PoW authoring)
#   miner1   — --mine only, coinbase Charlie (no session keys; PoW authoring
#              is open — committee membership is only needed to VOTE)
#   miner2   — --mine only, coinbase Dave   (full runs only; QUICK uses one)
#   rpcnode  — no --validator, no --mine; sync + RPC observer
#
# Alice is the bootnode (fixed --node-key); every other node bootnodes to
# her. All state lives under soak-data/ (wiped on each run): per-node
# base-paths, logs/, report.csv, summary.md.
#
# Invariants checked every TICK seconds; any violation fails fast with a
# diagnostic dump to soak-data/diagnostic-dump.txt:
#   1. the finalized head keeps advancing — no stall beyond the computed
#      finality-stall limit: 3x the 5s target when the observed cadence is
#      faster than target (near-deterministic production), and ~9x the
#      observed mean cadence (Poisson-fair, capped at 90s) once the
#      retargeted block interval is >= target. A ~60s grace applies after
#      each scheduled restart.
#   2. best-head spread across live nodes <= HEAD_SPREAD_MAX (5)
#   3. every live node keeps >= PEERS_MIN (1) peers, and its RPC answers
#   4. no two nodes report different block hashes at the same finalized
#      height (finality violation) — verified incrementally via
#      chain_getBlockHash at each newly-finalized height
#
# Adversity schedule: at ~25% and ~60% of the duration one random KEYLESS
# miner is SIGKILLed for ~30s, then restarted on its existing base-path; it
# must resync and resume authoring (asserted via the invariants plus
# post-restart block production: fresh "submitted winning seal" log lines
# and/or freshly-authored blocks attributed on-chain via the
# PreRuntime(pow_) miner digest).
#
# Usage:
#   scripts/soak-ghost.sh [duration_secs]
#   QUICK=1 scripts/soak-ghost.sh 300        # ~5min CI mode (1 keyless miner)
#
# Env:
#   QUICK=1              short defaults (300s duration, one keyless miner)
#   GHOST_NODE_BIN       path to a prebuilt ghost-node binary
#   SOAK_BUILD=always    rebuild the release binary even if present
#   SOAK_MINING_THREADS  PoW grind threads per miner (default 1)
#   SOAK_DIR             output dir (default <repo>/soak-data)
#
# Requires: curl, jq, python3. Exit 0 only if every invariant held for the
# whole duration.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QUICK="${QUICK:-0}"

if [[ "${1:-}" == "--quick" || "${1:-}" == "-q" ]]; then
	QUICK=1
	shift
fi

DURATION="${1:-}"
if [[ -z "$DURATION" ]]; then
	if [[ "$QUICK" == "1" ]]; then
		DURATION=300 # ~5min CI soak
	else
		DURATION=2400 # meaningful pre-testnet gate: 40min
	fi
fi
if ! [[ "$DURATION" =~ ^[0-9]+$ ]] || (( DURATION < 60 )); then
	echo "usage: QUICK=1 scripts/soak-ghost.sh [duration_secs>=60]" >&2
	exit 2
fi

if [[ "$QUICK" == "1" ]]; then
	KEYLESS_MINERS=("miner1")
else
	KEYLESS_MINERS=("miner1" "miner2")
fi
ALL_NODES=(alice bob rpcnode)
for m in "${KEYLESS_MINERS[@]}"; do
	ALL_NODES+=("$m")
done

SOAK_DIR="${SOAK_DIR:-$ROOT_DIR/soak-data}"
LOG_DIR="$SOAK_DIR/logs"
REPORT_CSV="$SOAK_DIR/report.csv"
SUMMARY_MD="$SOAK_DIR/summary.md"
DIAG_DUMP="$SOAK_DIR/diagnostic-dump.txt"

TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
BIN="${GHOST_NODE_BIN:-$TARGET_DIR/release/ghost-node}"
MINING_THREADS="${SOAK_MINING_THREADS:-1}"

# Chain constants (runtime/src/lib.rs + runtime/src/configs/mod.rs).
TARGET_BLOCK_SECS=5 # MILLI_SECS_PER_BLOCK = 5000

TICK=10                 # monitor cadence
HEAD_SPREAD_MAX=5       # max allowed best-head difference across nodes
PEERS_MIN=1
RPC_DEAD_TICKS=3        # consecutive RPC failures before we call the node hung
RESTART_DOWN_SECS=30    # how long a killed miner stays down
RESTART_GRACE_SECS=60   # invariant grace after a restart
WARMUP_MAX=180          # deadline for the first finalized block

# Dev-account coinbases (SS58 checked against the genesis pubkeys in
# runtime/src/genesis_config_presets.rs; distinct per miner so authored
# blocks attribute unambiguously).
ALICE_SS58="5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"
BOB_SS58="5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"
CHARLIE_SS58="5FLSigC9HGRKVhB9FiEo4Y3koPsNmBmLJbpXg2mp1hXcS59Y"
DAVE_SS58="5DAAnrj7VHTznn2AWBemMuyBwZWs6FNFjdyVXUeYum3PTXFy"

ALICE_NODE_KEY=0000000000000000000000000000000000000000000000000000000000000001
BOB_NODE_KEY=0000000000000000000000000000000000000000000000000000000000000002
MINER1_NODE_KEY=0000000000000000000000000000000000000000000000000000000000000003
MINER2_NODE_KEY=0000000000000000000000000000000000000000000000000000000000000004
RPCNODE_NODE_KEY=0000000000000000000000000000000000000000000000000000000000000005
ALICE_PEER=12D3KooWEyoppNCUx8Yx66oV9fJnriXwCcXwDDUA2kj6vnc6iDEp
BOOTNODE="/ip4/127.0.0.1/tcp/31440/p2p/$ALICE_PEER"

# Per-node runtime state.
declare -A NODE_PID=()
declare -A NODE_RPC=([alice]=19980 [bob]=19981 [miner1]=19982 [miner2]=19983 [rpcnode]=19984)
declare -A NODE_STATE=()        # up | dead-scheduled
declare -A NODE_RESTART_DUE=()
declare -A NODE_GRACE_UNTIL=()
declare -A NODE_DOWN_STREAK=()
declare -A SEAL_BASELINE_AT_RESTART=()
declare -A RESTARTED_AT=()
declare -A LAST_ROW=()

LAST_FIN=0
LAST_FIN_CHANGE=0
ALICE_BEST=0
ALICE_FIN=0
VERIFIED_HEIGHT=0
GRACE_UNTIL_GLOBAL=0
MAX_FIN_STALL=0
RECENT_GAPS=()
LAST_BEST_REF=0
LAST_BEST_REF_T=0
KILLED_MINERS=()

node_args() {
	local name="$1"
	local args=(
		--chain local
		--base-path "$SOAK_DIR/$name"
		--no-telemetry
		-l "warn,pow=info"
	)
	case "$name" in
		alice)
			args+=(--alice --validator --mine --miner-coinbase "$ALICE_SS58"
				--mining-threads "$MINING_THREADS" --node-key "$ALICE_NODE_KEY"
				--port 31440 --rpc-port 19980 --prometheus-port 19640)
			;;
		bob)
			args+=(--bob --validator --mine --miner-coinbase "$BOB_SS58"
				--mining-threads "$MINING_THREADS" --node-key "$BOB_NODE_KEY"
				--port 31441 --rpc-port 19981 --prometheus-port 19641
				--bootnodes "$BOOTNODE")
			;;
		miner1)
			args+=(--mine --miner-coinbase "$CHARLIE_SS58"
				--mining-threads "$MINING_THREADS" --node-key "$MINER1_NODE_KEY"
				--port 31442 --rpc-port 19982 --prometheus-port 19642
				--bootnodes "$BOOTNODE")
			;;
		miner2)
			args+=(--mine --miner-coinbase "$DAVE_SS58"
				--mining-threads "$MINING_THREADS" --node-key "$MINER2_NODE_KEY"
				--port 31443 --rpc-port 19983 --prometheus-port 19643
				--bootnodes "$BOOTNODE")
			;;
		rpcnode)
			args+=(--node-key "$RPCNODE_NODE_KEY"
				--port 31444 --rpc-port 19984 --prometheus-port 19644
				--bootnodes "$BOOTNODE")
			;;
	esac
	printf '%s\n' "${args[@]}"
}

rpc() {
	local port="$1" method="$2" params="${3:-[]}"
	curl -fsS --max-time 5 \
		-H 'Content-Type: application/json' \
		-d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":$params}" \
		"http://127.0.0.1:$port"
}

# Hex block number from a chain_getHeader response, as a decimal integer.
header_number() {
	local port="$1" hash="${2:-}"
	local params='[]'
	[[ -n "$hash" ]] && params="[\"$hash\"]"
	local n
	n="$(rpc "$port" chain_getHeader "$params" | jq -r '.result.number')" || return 1
	echo $((n))
}

start_node() {
	local name="$1"
	mapfile -t args < <(node_args "$name")
	# append: restarts must not erase the pre-kill history used for
	# authored-block accounting and post-restart assertions
	"$BIN" "${args[@]}" >>"$LOG_DIR/$name.log" 2>&1 &
	NODE_PID[$name]=$!
	NODE_STATE[$name]=up
	NODE_DOWN_STREAK[$name]=0
	echo "[$(date -u +%H:%M:%S)] started $name pid=${NODE_PID[$name]}"
}

cleanup() {
	for name in "${!NODE_PID[@]}"; do
		kill "${NODE_PID[$name]}" >/dev/null 2>&1 || true
	done
	wait >/dev/null 2>&1 || true
}
trap cleanup EXIT

wait_rpc() {
	local port="$1" tries="${2:-90}"
	for _ in $(seq 1 "$tries"); do
		if rpc "$port" system_health >/dev/null 2>&1; then
			return 0
		fi
		sleep 1
	done
	echo "RPC port $port did not become ready" >&2
	return 1
}

collect_row() {
	# Append one CSV row for a node and echo its "best fin peers syncing"
	# metrics (or "dead" / "unreachable") for the caller to consume.
	local name="$1" now="$2" elapsed="$3" port="${NODE_RPC[$name]}"
	if [[ "${NODE_STATE[$name]}" == "dead-scheduled" ]]; then
		echo "$now,$elapsed,$name,,,,,dead" >>"$REPORT_CSV"
		echo "dead"
		return 0
	fi
	local health
	if ! health="$(rpc "$port" system_health 2>/dev/null)"; then
		echo "$now,$elapsed,$name,,,,,unreachable" >>"$REPORT_CSV"
		echo "unreachable"
		return 0
	fi
	local peers syncing best fin fin_hash
	peers="$(printf '%s' "$health" | jq -r '.result.peers')"
	syncing="$(printf '%s' "$health" | jq -r '.result.isSyncing')"
	best="$(header_number "$port" || echo 0)"
	fin_hash="$(rpc "$port" chain_getFinalizedHead | jq -r '.result' 2>/dev/null || true)"
	if [[ -n "$fin_hash" && "$fin_hash" != "null" ]]; then
		fin="$(header_number "$port" "$fin_hash" || echo 0)"
	else
		fin=0
	fi
	echo "$now,$elapsed,$name,$best,$fin,$peers,$syncing,up" >>"$REPORT_CSV"
	echo "$best $fin $peers $syncing"
}

submitted_seals() {
	# Winning PoW seals this node has submitted; the worker logs a line only
	# after the seal imports successfully, so this counts authored blocks.
	grep -c "submitted winning seal" "$LOG_DIR/$1.log" 2>/dev/null || echo 0
}

diag_dump() {
	{
		echo "=== soak diagnostic dump $(date -uIs) ==="
		echo "--- last 15 report.csv rows"
		tail -n 15 "$REPORT_CSV" 2>/dev/null || true
		for name in "${ALL_NODES[@]}"; do
			echo "--- $name state=${NODE_STATE[$name]:-?} pid=${NODE_PID[$name]:-?} last 40 log lines"
			tail -n 40 "$LOG_DIR/$name.log" 2>/dev/null || echo "(no log)"
		done
	} >"$DIAG_DUMP" 2>&1 || true
	cat "$DIAG_DUMP" >&2
}

fail() {
	echo "SOAK FAIL: $*" >&2
	diag_dump
	write_summary "FAILED: $*"
	exit 1
}

# Rolling mean (integer seconds) of gaps between alice best-head advances.
RECENT_GAPS_SUM=0
record_best_advance() {
	local now="$1" best="$2"
	if (( LAST_BEST_REF > 0 && best > LAST_BEST_REF && now > LAST_BEST_REF_T )); then
		local gap=$(( (now - LAST_BEST_REF_T) / (best - LAST_BEST_REF) ))
		RECENT_GAPS+=("$gap")
		RECENT_GAPS_SUM=$((RECENT_GAPS_SUM + gap))
		if (( ${#RECENT_GAPS[@]} > 20 )); then
			RECENT_GAPS_SUM=$((RECENT_GAPS_SUM - RECENT_GAPS[0]))
			RECENT_GAPS=("${RECENT_GAPS[@]:1}")
		fi
	fi
	if (( best > LAST_BEST_REF )); then
		LAST_BEST_REF=$best
		LAST_BEST_REF_T=$now
	fi
}

recent_block_interval() {
	local n="${#RECENT_GAPS[@]}"
	(( n == 0 )) && { echo 0; return; }
	echo $((RECENT_GAPS_SUM / n))
}

stall_limit() {
	# See the header comment: hard 3x-target below ~4s cadence, Poisson-fair
	# 9x-mean (cap 90s) once the retargeted cadence is at/above target.
	local avg="$1"
	if (( avg < 4 )); then
		echo $((3 * TARGET_BLOCK_SECS))
	else
		local lim=$((avg * 9))
		(( lim > 90 )) && lim=90
		echo "$lim"
	fi
}

check_invariants() {
	local now="$1"
	local max_best=0 min_best=0 have_best=0

	for name in "${ALL_NODES[@]}"; do
		[[ "${NODE_STATE[$name]}" != "up" ]] && continue

		# A process that vanished outside the kill schedule is a node crash —
		# a real bug; fail fast.
		if ! kill -0 "${NODE_PID[$name]}" 2>/dev/null; then
			fail "node $name died unexpectedly (see $LOG_DIR/$name.log)"
		fi

		local row="${LAST_ROW[$name]:-}"
		if [[ "$row" == "unreachable" ]]; then
			NODE_DOWN_STREAK[$name]=$(( ${NODE_DOWN_STREAK[$name]:-0} + 1 ))
			if (( NODE_DOWN_STREAK[$name] > RPC_DEAD_TICKS )) && (( now >= ${NODE_GRACE_UNTIL[$name]:-0} )); then
				fail "$name RPC unresponsive for ${NODE_DOWN_STREAK[$name]} consecutive ticks"
			fi
			continue
		elif [[ "$row" != "dead" && -n "$row" ]]; then
			NODE_DOWN_STREAK[$name]=0
		else
			continue
		fi

		local best peers
		best="$(echo "$row" | awk '{print $1}')"
		peers="$(echo "$row" | awk '{print $3}')"

		# The just-restarted node is exempt from head/peers checks during its
		# grace window.
		if (( now >= ${NODE_GRACE_UNTIL[$name]:-0} )); then
			if (( have_best == 0 )); then
				max_best=$best; min_best=$best; have_best=1
			else
				(( best > max_best )) && max_best=$best
				(( best < min_best )) && min_best=$best
			fi
			if (( peers < PEERS_MIN )); then
				fail "$name has $peers peers (< $PEERS_MIN)"
			fi
		fi
	done

	if (( have_best == 1 && max_best - min_best > HEAD_SPREAD_MAX )); then
		fail "best-head spread $((max_best - min_best)) > $HEAD_SPREAD_MAX across live nodes"
	fi

	# Finality progress, referenced on alice (never restarted).
	local stall
	stall=$((now - LAST_FIN_CHANGE))
	(( stall > MAX_FIN_STALL )) && MAX_FIN_STALL=$stall
	if (( ALICE_FIN != LAST_FIN )); then
		LAST_FIN=$ALICE_FIN
		LAST_FIN_CHANGE=$now
	else
		local limit
		limit="$(stall_limit "$(recent_block_interval)")"
		if (( now < GRACE_UNTIL_GLOBAL )); then
			local grace_left=$((GRACE_UNTIL_GLOBAL - now))
			(( grace_left > limit )) && limit=$grace_left
		fi
		if (( stall > limit )); then
			fail "finalized head stalled at #$LAST_FIN for ${stall}s (limit ${limit}s, recent block interval $(recent_block_interval)s)"
		fi
	fi

	verify_finalized_hashes || fail "finality violation: nodes disagree on a finalized block hash"
}

verify_finalized_hashes() {
	# Compare chain_getBlockHash across all up nodes for each height that
	# became finalized on every one of them since the last check.
	local min_fin=0 have=0
	for name in "${ALL_NODES[@]}"; do
		[[ "${NODE_STATE[$name]}" != "up" ]] && continue
		local row="${LAST_ROW[$name]:-}"
		[[ "$row" == "dead" || "$row" == "unreachable" || -z "$row" ]] && continue
		local fin
		fin="$(echo "$row" | awk '{print $2}')"
		if (( have == 0 )); then min_fin=$fin; have=1; else (( fin < min_fin )) && min_fin=$fin; fi
	done
	(( have == 0 || min_fin <= VERIFIED_HEIGHT )) && return 0

	local h hash ref_hash ref_node port
	for (( h = VERIFIED_HEIGHT + 1; h <= min_fin; h++ )); do
		ref_hash=""; ref_node=""
		local hparam
		hparam="$(printf '0x%x' "$h")"
		for name in "${ALL_NODES[@]}"; do
			[[ "${NODE_STATE[$name]}" != "up" ]] && continue
			local row="${LAST_ROW[$name]:-}"
			[[ "$row" == "dead" || "$row" == "unreachable" || -z "$row" ]] && continue
			port="${NODE_RPC[$name]}"
			hash="$(rpc "$port" chain_getBlockHash "[\"$hparam\"]" 2>/dev/null | jq -r '.result' 2>/dev/null || true)"
			[[ -z "$hash" || "$hash" == "null" ]] && continue
			if [[ -z "$ref_hash" ]]; then
				ref_hash="$hash"; ref_node="$name"
			elif [[ "$hash" != "$ref_hash" ]]; then
				{
					echo "FINALITY VIOLATION at height $h:"
					echo "  $ref_node: $ref_hash"
					echo "  $name: $hash"
				} >>"$DIAG_DUMP" 2>/dev/null || true
				return 1
			fi
		done
	done
	VERIFIED_HEIGHT=$min_fin
	return 0
}

# --- adversity --------------------------------------------------------------

pick_keyless_miner() {
	local live=()
	for name in "${KEYLESS_MINERS[@]}"; do
		[[ "${NODE_STATE[$name]:-}" == "up" ]] && live+=("$name")
	done
	(( ${#live[@]} == 0 )) && { echo ""; return; }
	echo "${live[$((RANDOM % ${#live[@]}))]}"
}

kill_miner_event() {
	local now="$1" name
	name="$(pick_keyless_miner)"
	[[ -z "$name" ]] && return 0
	echo "[$(date -u +%H:%M:%S)] adversity: SIGKILL keyless miner $name (down ${RESTART_DOWN_SECS}s)"
	kill -9 "${NODE_PID[$name]}" >/dev/null 2>&1 || true
	wait "${NODE_PID[$name]}" >/dev/null 2>&1 || true
	NODE_STATE[$name]=dead-scheduled
	NODE_RESTART_DUE[$name]=$((now + RESTART_DOWN_SECS))
	KILLED_MINERS+=("$name:$now")
}

restart_due_miners() {
	local now="$1"
	for name in "${KEYLESS_MINERS[@]}"; do
		if [[ "${NODE_STATE[$name]:-}" == "dead-scheduled" ]] && (( ${NODE_RESTART_DUE[$name]:-0} <= now )); then
			start_node "$name"
			wait_rpc "${NODE_RPC[$name]}" 60 || fail "$name RPC never came back after restart"
			NODE_GRACE_UNTIL[$name]=$((now + RESTART_GRACE_SECS))
			GRACE_UNTIL_GLOBAL=$((now + RESTART_GRACE_SECS))
			RESTARTED_AT[$name]=$now
			SEAL_BASELINE_AT_RESTART[$name]="$(submitted_seals "$name")"
			echo "[$(date -u +%H:%M:%S)] $name restarted; invariant grace until +${RESTART_GRACE_SECS}s"
		fi
	done
}

# --- summary ----------------------------------------------------------------

write_summary() {
	local result="${1:-PASSED}"
	local kills="${KILLED_MINERS[*]:-none}"
	python3 - "$SOAK_DIR" "$result" "$DURATION" "$TARGET_BLOCK_SECS" "$TICK" \
		"${NODE_RPC[alice]}" "${NODE_RPC[rpcnode]}" "$kills" "$MAX_FIN_STALL" <<'PYEOF'
import csv, json, os, re, sys, time, urllib.request

soak_dir, result, duration, target_secs, tick = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
alice_rpc, restarts, max_fin_stall = int(sys.argv[6]), sys.argv[8], int(sys.argv[9])
report = os.path.join(soak_dir, "report.csv")
out = os.path.join(soak_dir, "summary.md")
log_dir = os.path.join(soak_dir, "logs")

def rpc(port, method, params=None):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method,
                       "params": params or []}).encode()
    req = urllib.request.Request(f"http://127.0.0.1:{port}", data=body,
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.load(r)["result"]

PUBKEY_TO_NODE = {
    "d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d": "alice",
    "8eaf04151687736326c9fea17e25fc5287613693c912909cb226aa4794f26a48": "bob",
    "90b5ab205c6974c9ea841be688864633dc9ca8a357843eeacf2314649965fe22": "miner1",
    "306721211d5404bd9da88e0204360a1a9ab8b87c66c1bc2fcdd37f3c2222cc20": "miner2",
}

def miner_of_header(header):
    """Coinbase pubkey from the PreRuntime(b'pow_', AccountId32) digest item."""
    for log_hex in header.get("digest", {}).get("logs", []):
        b = bytes.fromhex(log_hex[2:] if log_hex.startswith("0x") else log_hex)
        if len(b) < 5 or b[0] != 0x06 or b[1:5] != b"pow_":
            continue
        # SCALE Bytes: compact length then payload; miner = last 32 bytes.
        i = 5
        mode = b[i] & 0b11
        if mode == 0: n, i = b[i] >> 2, i + 1
        elif mode == 1: n, i = int.from_bytes(b[i:i+2], "little") >> 2, i + 2
        elif mode == 2: n, i = int.from_bytes(b[i:i+4], "little") >> 2, i + 4
        else: n, i = 0, i + 1 + ((b[i] >> 2) + 4)
        payload = b[i:i+n]
        if len(payload) >= 32:
            return payload[-32:].hex()
    return None

rows = []
if os.path.exists(report):
    with open(report) as f:
        rows = list(csv.DictReader(f))

max_best = max([int(r["best"]) for r in rows if r["best"]], default=0)
max_fin = max([int(r["finalized"]) for r in rows if r["finalized"]], default=0)
lags = [int(r["best"]) - int(r["finalized"]) for r in rows if r["best"] and r["finalized"]]
avg_lag = sum(lags) / len(lags) if lags else 0
lags_sorted = sorted(lags)
p95_lag = lags_sorted[int(0.95 * (len(lags_sorted) - 1))] if lags_sorted else 0
max_lag = max(lags) if lags else 0
observed = max([int(r["elapsed"]) for r in rows], default=0)

# Per-node authored counts: log-derived (seals submitted and imported) plus
# canonical attribution decoded from alice's chain DB.
def seal_count(node):
    p = os.path.join(log_dir, node + ".log")
    if not os.path.exists(p):
        return 0
    with open(p, errors="replace") as f:
        return f.read().count("submitted winning seal")

def log_count(node, needle):
    p = os.path.join(log_dir, node + ".log")
    if not os.path.exists(p):
        return 0
    with open(p, errors="replace") as f:
        return f.read().count(needle)

nodes = sorted({r["node"] for r in rows})
canonical = {n: 0 for n in nodes}
canonical_total = 0
try:
    fin_hash = rpc(alice_rpc, "chain_getFinalizedHead")
    fin_num = int(rpc(alice_rpc, "chain_getHeader", [fin_hash])["number"], 16)
    for h in range(1, fin_num + 1):
        bh = rpc(alice_rpc, "chain_getBlockHash", [hex(h)])
        hdr = rpc(alice_rpc, "chain_getHeader", [bh])
        who = miner_of_header(hdr)
        if who:
            canonical[PUBKEY_TO_NODE.get(who, who[:12] + "…")] = canonical.get(
                PUBKEY_TO_NODE.get(who, who[:12] + "…"), 0) + 1
            canonical_total += 1
except Exception as e:
    print(f"note: canonical attribution unavailable: {e}", file=sys.stderr)

L = []
L.append("# Ghost local-network soak report")
L.append("")
L.append(f"- result: **{result}**")
L.append(f"- configured duration: {duration}s (observed {observed}s)")
L.append(f"- blocks produced (max best head seen): {max_best}")
L.append(f"- max finalized height: {max_fin}")
L.append(f"- finality lag (best - finalized): avg {avg_lag:.1f} / p95 {p95_lag} / max {max_lag}")
L.append(f"- max observed finalized-head stall: {max_fin_stall}s (tick {tick}s)")
def fmt_restarts(s):
    if s in ("", "none"):
        return "none"
    out = []
    for tok in s.split():
        name, _, ts = tok.partition(":")
        try:
            out.append(f"{name}@{time.strftime('%H:%M:%S', time.gmtime(int(ts)))}Z")
        except ValueError:
            out.append(tok)
    return " ".join(out)

L.append(f"- keyless-miner restarts: {fmt_restarts(restarts)}")
L.append("")
L.append("## Authored blocks per node")
L.append("")
L.append("| node | role | seals submitted (log) | canonical blocks (pow_ digest) |")
L.append("| --- | --- | --- | --- |")
for n in nodes:
    role = "observer" if n == "rpcnode" else ("committee+miner" if n in ("alice", "bob") else "keyless miner")
    L.append(f"| {n} | {role} | {seal_count(n)} | {canonical.get(n, 0)} |")
L.append("")
L.append(f"Canonical authored blocks decoded: {canonical_total} (heights 1..{max_fin} on alice).")
L.append("")
L.append("## Node-side anomalies (log-derived)")
L.append("")
L.append("| node | `TooFarInFuture` rejects | `Unable to import` | panics |")
L.append("| --- | --- | --- | --- |")
for n in nodes:
    L.append(f"| {n} | {log_count(n, 'too far in the future')} | {log_count(n, 'Unable to import mined block')} | {log_count(n, 'panic')} |")
L.append("")
L.append("`TooFarInFuture` counts mined blocks dropped by the timestamp inherent")
L.append("check (`pallet_timestamp` rejects a block whose `now` exceeds the")
L.append("verifier's wall clock + 30s). Nonzero counts mean seal work is being")
L.append("wasted whenever the chain's `Now` outruns wall time (blocks faster")
L.append("than `MinimumPeriod = SLOT_DURATION/2`).")
L.append("")
L.append("Timeline evidence is in `report.csv`; per-node logs in `logs/`; on")
L.append("failure a diagnostic snapshot is appended to `diagnostic-dump.txt`.")
with open(out, "w") as f:
    f.write("\n".join(L) + "\n")
print(f"wrote {out}")
PYEOF
}

# --- main -------------------------------------------------------------------

for tool in curl jq python3; do
	command -v "$tool" >/dev/null || { echo "missing dependency: $tool" >&2; exit 2; }
done

if [[ ! -x "$BIN" || "${SOAK_BUILD:-auto}" == "always" ]]; then
	echo "==> Building ghost-node (release)"
	export WASM_BUILD_WORKSPACE_HINT="${WASM_BUILD_WORKSPACE_HINT:-$ROOT_DIR}"
	cargo build --release --bin ghost-node
	BIN="$TARGET_DIR/release/ghost-node"
fi
echo "==> Using binary: $BIN ($("$BIN" --version))"

echo "==> Cleaning $SOAK_DIR"
rm -rf "$SOAK_DIR"
mkdir -p "$LOG_DIR"
echo "ts,elapsed,node,best,finalized,peers,syncing,state" >"$REPORT_CSV"

echo "==> Starting nodes (duration=${DURATION}s quick=$QUICK keyless=${KEYLESS_MINERS[*]})"
for name in "${ALL_NODES[@]}"; do
	start_node "$name"
	wait_rpc "${NODE_RPC[$name]}" || fail "$name RPC never came up"
done

START_TS="$(date +%s)"

# Warmup: the network must start finalizing within WARMUP_MAX seconds.
echo "==> Warmup: waiting for first finalized block"
while :; do
	now="$(date +%s)"
	fin_hash="$(rpc "${NODE_RPC[alice]}" chain_getFinalizedHead 2>/dev/null | jq -r '.result' 2>/dev/null || true)"
	fin_now=0
	if [[ -n "$fin_hash" && "$fin_hash" != "null" ]]; then
		fin_now="$(header_number "${NODE_RPC[alice]}" "$fin_hash" || echo 0)"
	fi
	(( fin_now > 0 )) && break
	(( now - START_TS > WARMUP_MAX )) && fail "no finalized block within ${WARMUP_MAX}s of startup"
	sleep 2
done
LAST_FIN="$fin_now"
LAST_FIN_CHANGE="$(date +%s)"
GRACE_UNTIL_GLOBAL=$START_TS
echo "==> Finality live at #$LAST_FIN"

KILL1_AT=$(( DURATION / 4 ))
KILL2_AT=$(( DURATION * 3 / 5 ))
kill1_done=0 kill2_done=0
END_TS=$((START_TS + DURATION))
echo "==> Soaking until $(date -u -d "@$END_TS" +%H:%M:%S) UTC (kills at ~${KILL1_AT}s and ~${KILL2_AT}s elapsed)"

while (( $(date +%s) < END_TS )); do
	now="$(date +%s)"
	elapsed=$((now - START_TS))

	for name in "${ALL_NODES[@]}"; do
		row="$(collect_row "$name" "$now" "$elapsed")"
		LAST_ROW[$name]="$row"
	done
	ALICE_BEST="$(echo "${LAST_ROW[alice]}" | awk '{print $1}')"
	ALICE_FIN="$(echo "${LAST_ROW[alice]}" | awk '{print $2}')"
	record_best_advance "$now" "${ALICE_BEST:-0}"

	check_invariants "$now"

	if (( kill1_done == 0 && elapsed >= KILL1_AT )); then
		kill_miner_event "$now"; kill1_done=1
	fi
	if (( kill2_done == 0 && elapsed >= KILL2_AT )); then
		kill_miner_event "$now"; kill2_done=1
	fi
	restart_due_miners "$now"

	sleep "$TICK"
done

echo "==> Duration complete; post-run checks"

# Post-restart authoring: every restarted keyless miner must have produced at
# least one new winning seal since its restart (the invariants above already
# covered resync/peers; this is the block-author-diversity check).
for name in "${!RESTARTED_AT[@]}"; do
	before="${SEAL_BASELINE_AT_RESTART[$name]:-0}"
	after="$(submitted_seals "$name")"
	if (( after <= before )); then
		fail "$name submitted no new seals after restart ($before -> $after)"
	fi
	echo "$name authored $((after - before)) new block(s) post-restart"
done

write_summary "PASSED"
echo "==> SOAK PASSED (${DURATION}s): report=$REPORT_CSV summary=$SUMMARY_MD"
