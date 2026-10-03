#!/usr/bin/env bash
# e2e-ghost.sh — Ghost-consensus (PoW + GRANDPA) smoke gate.
#
# Sister gate to e2e-local.sh; proves the LIVE Ghost path, not the retired
# Aura path (docs/ghost-consensus-design.md §12 gate 2). What it asserts, in
# order:
#
#   1. ghost-node builds (or $GHOST_NODE_BIN / target/debug/ghost-node when
#      SKIP_BUILD=1).
#   2. A single `--dev --mine` node authors blocks whose headers carry BOTH
#      the `Seal(*b"pow_", GhostSeal)` log (hex contains 05706f775f) and the
#      `PreRuntime(*b"pow_", AccountId32)` miner log (06706f775f); a peered
#      non-mining dev node imports them and authors nothing itself.
#   3. Two `--chain local` miners (Alice/Bob session committee) each produce
#      pow_-sealed blocks — the pre-runtime digest decodes to two distinct
#      coinbase accounts over time. (Never two --dev nodes as peers: both
#      hold Alice's session keys → GRANDPA self-equivocation.)
#   4. GRANDPA finalized head tracks best head (lag bounded) and advances on
#      both nodes; peers == 1 each.
#   5. Retarget: at the first real retarget boundary (RetargetInterval=100;
#      block 100 stores the baseline timestamp, block 200 applies the first
#      adjustment), `GhostPowApi_next_difficulty` (state_call) and the
#      GhostConsensus::Difficulty storage item both equal the value the
#      pallet's own formula predicts from its own LastRetargetTime window
#      (read at the boundary block and the block before it), and
#      RetargetsDone has incremented — i.e. the retarget provably ran,
#      whether or not it moved the value.
#   6. Bob is killed and restarted on the same base path: resyncs, keeps
#      importing sealed blocks, authors again, finality advances.
#   7. Rewards: System::Account free balance of both miner coinbase
#      accounts (also genesis-staked session validators on --chain local)
#      grows over ~30+ blocks.
#
# Env:
#   SKIP_BUILD=1     reuse a prebuilt binary (GHOST_NODE_BIN or
#                    $CARGO_TARGET_DIR/debug/ghost-node)
#   GHOST_NODE_BIN   path to a ghost-node binary (overrides default)
#   MINING_THREADS   grinding threads per miner (default 4)
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
BIN="${GHOST_NODE_BIN:-$TARGET_DIR/debug/ghost-node}"

# Build env fallbacks — resolved dynamically instead of hardcoding a
# distribution's llvm/gcc versions (AGENTS.md documents llvm-18/gcc-13 but
# the sandbox's rtk ships llvm-14/gcc-11; both are correct in context).
if [ -z "${LIBCLANG_PATH:-}" ]; then
	LIBCLANG_PATH="$(dirname "$(find /usr/lib -name 'libclang.so*' 2>/dev/null | sort | head -n1)")"
fi
BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:--I$(gcc -print-file-name=include) -I/usr/include/x86_64-linux-gnu -I/usr/include}"
export LIBCLANG_PATH BINDGEN_EXTRA_CLANG_ARGS
export WASM_BUILD_WORKSPACE_HINT="${WASM_BUILD_WORKSPACE_HINT:-$ROOT_DIR}"

# shellcheck source=scripts/lib-ghost-rpc.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib-ghost-rpc.sh"

# Well-known dev accounts (sr25519 public == AccountId32).
ALICE_ACCT=d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d
BOB_ACCT=8eaf04151687736326c9fea17e25fc5287613693c912909cb226aa4794f26a48
ALICE_SS58=5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY
BOB_SS58=5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty

MINING_THREADS="${MINING_THREADS:-4}"

# Ports are offset from e2e-local.sh so the two gates can coexist.
DEV_RPC=29944
DEV_PORT=31433
DEV_FULL_RPC=29945
DEV_FULL_PORT=31434
ALICE_RPC=29946
ALICE_PORT=31435
BOB_RPC=29947
BOB_PORT=31436
DEV_PROM=29615
DEV_FULL_PROM=29616
ALICE_PROM=29617
BOB_PROM=29618

# node-key 0x..01 -> this fixed peer id (same pair e2e-local.sh uses).
BOOT_NODE_KEY=0000000000000000000000000000000000000000000000000000000000000001
BOB_NODE_KEY=0000000000000000000000000000000000000000000000000000000000000002
BOOT_PEER=12D3KooWEyoppNCUx8Yx66oV9fJnriXwCcXwDDUA2kj6vnc6iDEp

# Finality may trail the fast PoW best head by several GRANDPA rounds.
FINALITY_MAX_LAG=64
# Retarget model — these mirror runtime/src/configs/mod.rs
# (RetargetInterval=100, TargetBlockTimeMs=5000, MinDifficulty=1_000_000) and
# pallets/pallet-ghost-consensus/src/lib.rs::retarget. The gate recomputes the
# expected post-retarget difficulty from the measured window, so it proves the
# pallet ran the retarget rather than merely hoping difficulty moved.
RETARGET_INTERVAL=100
RETARGET_BASELINE=100   # first boundary: only stores the baseline timestamp
RETARGET_ADJUST=200     # second boundary: first real adjustment
TARGET_BLOCK_TIME_MS=5000
MIN_DIFFICULTY=1000000
RETARGET_BLOCK=205
# ~4.5s/blk debug pace => ~15 min to the boundary; bound generously since a
# timeout this late in the gate wastes the entire run. Overridable for
# slower/shared hardware.
RETARGET_TIMEOUT="${RETARGET_TIMEOUT:-1200}"

declare -A LIVE_PIDS=()
LAST_PID=""
TMP_DIR="$(mktemp -d /tmp/ghost-e2e-ghost.XXXXXX)"
# ghost_scale writes its python helper here on first use; cleaned up below.
GHOST_E2E_HELPER_DIR="$TMP_DIR"

cleanup() {
	local pid i
	for pid in "${!LIVE_PIDS[@]}"; do
		kill "$pid" >/dev/null 2>&1 || true
	done
	# Grace for graceful exit, then SIGKILL stragglers — a node left alive
	# keeps its p2p/RPC ports bound and silently cross-talks into the next
	# run (same node-keys => same peer ids, same ghost-local genesis).
	for i in $(seq 1 20); do
		local alive=0
		for pid in "${!LIVE_PIDS[@]}"; do
			kill -0 "$pid" 2>/dev/null && alive=1
		done
		(( alive == 0 )) && break
		sleep 1
	done
	for pid in "${!LIVE_PIDS[@]}"; do
		kill -9 "$pid" >/dev/null 2>&1 || true
	done
	wait >/dev/null 2>&1 || true
	if [ "${KEEP_TMP:-0}" = "1" ]; then
		echo "KEEP_TMP=1 — node logs and chain dirs left at $TMP_DIR" >&2
	else
		rm -rf "$TMP_DIR"
	fi
}
trap cleanup EXIT

# Dump the tail of a node log on stderr — used at failure points so a flake
# leaves its evidence behind even though cleanup removes $TMP_DIR.
dump_log_tail() {
	local log="$1" lines="${2:-60}"
	echo "----- tail of $log -----" >&2
	tail -n "$lines" "$log" >&2 || true
	echo "------------------------" >&2
}

# Fail fast if a port a node is about to bind is already held — otherwise a
# stray ghost-node from an earlier run (same node-keys => same peer ids,
# same genesis) silently absorbs the new node's dials and the gate can
# "pass" against a mixed old+new network.
assert_port_free() {
	local port="$1"
	if ss -tlnH "sport = :$port" 2>/dev/null | grep -q .; then
		echo "port $port is already in use — stray ghost-node? (check: ss -tlnp | grep ':$port')" >&2
		exit 1
	fi
}

start_node() {
	# start_node <logfile> <node args...>; sets LAST_PID, tracks it.
	local log="$1"
	shift
	local arg
	local prev=""
	for arg in "$@"; do
		case "$prev" in
			--rpc-port|--port|--prometheus-port) assert_port_free "$arg" ;;
		esac
		prev="$arg"
	done
	"$BIN" "$@" --no-telemetry -l warn >"$log" 2>&1 &
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

# Assert every header in 1..best on port $1 carries a pow_ seal + pow_
# pre-runtime. Progress goes to stderr; stdout carries the sorted distinct
# set of pre-runtime author account ids.
assert_all_pow_sealed() {
	local port="$1"
	local best total sealed authored
	best="$(best_number "$port")"
	scan_pow_headers "$port" 1 "$best" >"$TMP_DIR/scan.$port"
	total="$(wc -l <"$TMP_DIR/scan.$port")"
	if (( total != best )); then
		echo "header scan on :$port incomplete ($total of $best blocks)" >&2
		return 1
	fi
	sealed="$(awk '$3=="True"' "$TMP_DIR/scan.$port" | wc -l)"
	authored="$(awk '$4=="True"' "$TMP_DIR/scan.$port" | wc -l)"
	if (( sealed != total || authored != total )); then
		echo "non-pow block(s) on :$port — $sealed sealed, $authored authored out of $total" >&2
		awk '$3!="True" || $4!="True"' "$TMP_DIR/scan.$port" >&2
		return 1
	fi
	echo ":$port — all $total blocks carry Seal(pow_) + PreRuntime(pow_)" >&2
	awk '{print $2}' "$TMP_DIR/scan.$port" | sort -u
}

# Scan new headers on port $1 between cursor $2 and the node's best block;
# appends "<num> <author> <seal> <pre>" lines to the file named in $3.
# Returns non-zero if the batch came back incomplete (caller should retry
# the same range). Advances nothing itself.
scan_new_headers() {
	local port="$1" cursor="$2" best out rows want
	best="$(best_number "$port")"
	(( cursor > best )) && return 0
	out="$(scan_pow_headers "$port" "$cursor" "$best")" || true
	rows="$(printf '%s' "$out" | grep -c . || true)"
	want=$((best - cursor + 1))
	if (( rows != want )); then
		echo "partial header scan on :$port ($rows of $want); retrying range" >&2
		return 1
	fi
	printf '%s\n' "$out" >>"$3"
	echo "$best"
}

echo "==> [1/7] Building ghost-node (SKIP_BUILD=${SKIP_BUILD:-0})"
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

echo "==> [2/7] Single --dev miner authors pow_-sealed blocks; non-mining peer only imports"
start_node "$TMP_DIR/dev-miner.log" \
	--dev --tmp \
	--mine --mining-threads "$MINING_THREADS" --miner-coinbase "$ALICE_SS58" \
	--node-key "$BOOT_NODE_KEY" \
	--rpc-port "$DEV_RPC" --port "$DEV_PORT" --prometheus-port "$DEV_PROM" \
	--rpc-methods Unsafe
DEV_PID="$LAST_PID"
wait_rpc "$DEV_RPC"
wait_block_at_least "$DEV_RPC" 3 "dev miner"

DEV_MODE="$(rpc "$DEV_RPC" ghost_getConsensusMode | json_field "data['result']")"
echo "dev node reports: $DEV_MODE"
printf '%s' "$DEV_MODE" | grep -q "ghost-pow" || {
	echo "ghost_getConsensusMode does not report ghost-pow: $DEV_MODE" >&2
	exit 1
}

# Every authored header must contain 05706f775f (Seal) and 06706f775f
# (PreRuntime miner); the author must decode to the --miner-coinbase.
DEV_AUTHORS="$(assert_all_pow_sealed "$DEV_RPC")"
if [ "$DEV_AUTHORS" != "$ALICE_ACCT" ]; then
	echo "expected only coinbase author $ALICE_ACCT; got:" >&2
	echo "$DEV_AUTHORS" >&2
	exit 1
fi
echo "dev miner authors all decode to coinbase account $ALICE_ACCT"

# Non-mining peer: same dev chain (ghost-dev genesis), no keys, never an
# authority — imports the miner's sealed blocks, authors none. (A second
# --dev node would also hold Alice's session keys and self-equivocate on
# GRANDPA; --chain dev keeps it a clean full node.)
start_node "$TMP_DIR/dev-full.log" \
	--chain dev --tmp \
	--port "$DEV_FULL_PORT" --rpc-port "$DEV_FULL_RPC" --prometheus-port "$DEV_FULL_PROM" \
	--bootnodes "/ip4/127.0.0.1/tcp/$DEV_PORT/p2p/$BOOT_PEER" \
	--rpc-methods Unsafe
DEV_FULL_PID="$LAST_PID"
wait_rpc "$DEV_FULL_RPC"
wait_block_at_least "$DEV_FULL_RPC" 3 "dev full node"
wait_peers_eq "$DEV_FULL_RPC" 1 60 || {
	echo "non-mining peer never connected" >&2
	exit 1
}
DEV_FULL_PEERS=1
wait_block_at_least "$DEV_FULL_RPC" "$(best_number "$DEV_RPC")" "dev full node sync" 60
FULL_AUTHORS="$(assert_all_pow_sealed "$DEV_FULL_RPC")"
if [ "$FULL_AUTHORS" != "$ALICE_ACCT" ]; then
	echo "non-mining peer should only import the miner's blocks; got authors:" >&2
	echo "$FULL_AUTHORS" >&2
	exit 1
fi
echo "non-mining peer (peers=$DEV_FULL_PEERS) imported only $ALICE_ACCT-authored sealed blocks"

kill_node "$DEV_PID"
kill_node "$DEV_FULL_PID"
echo "dev miner + non-mining peer stopped"

echo "==> [3/7] Two --chain local miners both author pow_-sealed blocks"
start_node "$TMP_DIR/alice.log" \
	--chain local --alice --validator \
	--mine --mining-threads "$MINING_THREADS" --miner-coinbase "$ALICE_SS58" \
	--base-path "$TMP_DIR/alice" \
	--node-key "$BOOT_NODE_KEY" \
	--rpc-port "$ALICE_RPC" --port "$ALICE_PORT" --prometheus-port "$ALICE_PROM" \
	--rpc-methods Unsafe
ALICE_PID="$LAST_PID"
wait_rpc "$ALICE_RPC"

start_node "$TMP_DIR/bob.log" \
	--chain local --bob --validator \
	--mine --mining-threads "$MINING_THREADS" --miner-coinbase "$BOB_SS58" \
	--base-path "$TMP_DIR/bob" \
	--node-key "$BOB_NODE_KEY" \
	--rpc-port "$BOB_RPC" --port "$BOB_PORT" --prometheus-port "$BOB_PROM" \
	--bootnodes "/ip4/127.0.0.1/tcp/$ALICE_PORT/p2p/$BOOT_PEER" \
	--rpc-methods Unsafe
BOB_PID="$LAST_PID"
wait_rpc "$BOB_RPC"

# Difficulty "before" sample for the retarget check — well ahead of the
# first boundary at block 100 (baseline only) and block 200 (adjustment).
DIFF_BEFORE="$(ghost_next_difficulty "$ALICE_RPC")"
echo "next_difficulty at early height: $DIFF_BEFORE"

# Collect distinct pre-runtime authors until both coinbases appear; every
# scanned block must be pow_-sealed.
HEADERS_FILE="$TMP_DIR/local-headers"
: >"$HEADERS_FILE"
SCAN_CURSOR=1
TWO_MINERS_DEADLINE=$((SECONDS + 300))
while true; do
	if BEST_SEEN="$(scan_new_headers "$ALICE_RPC" "$SCAN_CURSOR" "$HEADERS_FILE")"; then
		[ -n "$BEST_SEEN" ] && SCAN_CURSOR=$((BEST_SEEN + 1))
	fi
	if awk -v a="$ALICE_ACCT" -v b="$BOB_ACCT" \
		'$2==a {x=1} $2==b {y=1} END {exit !(x&&y)}' "$HEADERS_FILE"; then
		echo "both coinbase accounts authored pow_-sealed blocks by height $((SCAN_CURSOR - 1))"
		break
	fi
	if (( SECONDS >= TWO_MINERS_DEADLINE )); then
		echo "timed out waiting for both miners to author; authors seen:" >&2
		awk '{print $2}' "$HEADERS_FILE" | sort -u >&2
		exit 1
	fi
	sleep 2
done
if awk '$3!="True" || $4!="True"' "$HEADERS_FILE" | grep -q .; then
	echo "local-testnet block(s) missing pow_ seal/preruntime:" >&2
	awk '$3!="True" || $4!="True"' "$HEADERS_FILE" >&2
	exit 1
fi

echo "==> [4/7] GRANDPA finality tracks the PoW best head on both nodes"
wait_finalized_at_least "$ALICE_RPC" 3 "alice"
wait_finalized_at_least "$BOB_RPC" 3 "bob"

# peers == 1 each — with a grace window for transient drops under load.
if ! wait_peers_eq "$ALICE_RPC" 1 90; then
	echo "alice lost its peer" >&2
	dump_log_tail "$TMP_DIR/alice.log"
	exit 1
fi
if ! wait_peers_eq "$BOB_RPC" 1 90; then
	echo "bob lost its peer" >&2
	dump_log_tail "$TMP_DIR/bob.log"
	exit 1
fi
echo "peers: alice=1 bob=1"

for port in "$ALICE_RPC" "$BOB_RPC"; do
	best="$(best_number "$port")"
	fin="$(finalized_number "$port")"
	lag=$((best - fin))
	echo ":$port best=$best finalized=$fin lag=$lag"
	if (( lag > FINALITY_MAX_LAG )); then
		echo "finality lag $lag exceeds bound $FINALITY_MAX_LAG on :$port" >&2
		exit 1
	fi
	# Finalized head must keep advancing — poll rather than fixed-sleep, since
	# a single GRANDPA round under two-miner fork contention can exceed a
	# short sleep window on a debug build.
	wait_finalized_at_least "$port" "$((fin + 1))" ":$port finalized advances" 120
done
echo "finality advances on both nodes with bounded lag"

echo "==> [5/7] Difficulty retarget executes on-chain"
if ! wait_block_at_least "$ALICE_RPC" "$RETARGET_BLOCK" "alice" "$RETARGET_TIMEOUT"; then
	cat >&2 <<-EOF
		chain did not reach retarget block $RETARGET_BLOCK in ${RETARGET_TIMEOUT}s.
		The first real adjustment is at block 200 (block 100 only stores the
		baseline timestamp). If this times out on slower hardware raise
		RETARGET_TIMEOUT; the check itself is sound.
	EOF
	exit 1
fi
# Prove the retarget ran and computed correctly — not just "did it move".
# The pallet retargets in on_initialize at every RetargetInterval boundary:
#   new = old * expected/elapsed, clamped to [old/4, old*4], floored at
#   MinDifficulty, where elapsed = Timestamp::Now - LastRetargetTime.
# Both operands it used are recoverable exactly: LastRetargetTime at the
# pre-boundary block still holds the previous baseline, and at the
# boundary block it holds the `now` the pallet just captured (written by
# on_initialize before the block's timestamp inherent runs). So instead
# of guessing timestamps we read LastRetargetTime at b$((n-1)) (baseline)
# and b$n (`now`), recompute the pallet's own formula, and require the
# runtime API, the Difficulty storage item, and RetargetsDone to agree.
B1_HASH="$(block_hash_by_number "$ALICE_RPC" "$((RETARGET_ADJUST - 1))")"
B2_HASH="$(block_hash_by_number "$ALICE_RPC" "$RETARGET_ADJUST")"
LAST_KEY="$(ghost_scale storage_key GhostConsensus LastRetargetTime)"
DIFF_KEY="$(ghost_scale storage_key GhostConsensus Difficulty)"
DONE_KEY="$(ghost_scale storage_key GhostConsensus RetargetsDone)"

LAST_PREV_HEX="$(storage_at "$ALICE_RPC" "$LAST_KEY" "$B1_HASH")"
LAST_NOW_HEX="$(storage_at "$ALICE_RPC" "$LAST_KEY" "$B2_HASH")"
DIFF_OLD_HEX="$(storage_at "$ALICE_RPC" "$DIFF_KEY" "$B1_HASH")"
DONE_HEX="$(storage_at "$ALICE_RPC" "$DONE_KEY" "$B2_HASH")"
if [ "$LAST_PREV_HEX" = "None" ] || [ "$LAST_NOW_HEX" = "None" ] || [ "$DIFF_OLD_HEX" = "None" ]; then
	echo "could not read GhostConsensus retarget state at boundary blocks" >&2
	exit 1
fi
LAST_PREV="$(ghost_scale uintle "$LAST_PREV_HEX")"
LAST_NOW="$(ghost_scale uintle "$LAST_NOW_HEX")"
ELAPSED_MS=$((LAST_NOW - LAST_PREV))
DIFF_OLD="$(ghost_scale u256le "$DIFF_OLD_HEX")"
if [ "$DONE_HEX" = "None" ]; then
	DONE_N=0
else
	DONE_N="$(ghost_scale uintle "$DONE_HEX")"
fi
EXPECTED_MS=$((RETARGET_INTERVAL * TARGET_BLOCK_TIME_MS))
DIFF_EXPECTED="$(ghost_scale retarget_expect "$DIFF_OLD" "$ELAPSED_MS" "$EXPECTED_MS" "$MIN_DIFFICULTY")"
DIFF_AFTER="$(ghost_next_difficulty "$ALICE_RPC" "$B2_HASH")"
DIFF_STORAGE="$(ghost_scale u256le "$(storage_at "$ALICE_RPC" "$DIFF_KEY" "$B2_HASH")")"

echo "retarget window at b$RETARGET_ADJUST: elapsed=${ELAPSED_MS}ms expected=${EXPECTED_MS}ms"
echo "difficulty: before=$DIFF_OLD expected=$DIFF_EXPECTED api=$DIFF_AFTER storage=$DIFF_STORAGE retargets_done=$DONE_N"

if (( DONE_N < 2 )); then
	echo "GhostConsensus::RetargetsDone=$DONE_N — retarget path did not run at b$RETARGET_ADJUST" >&2
	exit 1
fi
if [ "$DIFF_AFTER" != "$DIFF_EXPECTED" ]; then
	echo "GhostPowApi_next_difficulty ($DIFF_AFTER) != expected retarget result ($DIFF_EXPECTED)" >&2
	exit 1
fi
if [ "$DIFF_STORAGE" != "$DIFF_EXPECTED" ]; then
	echo "GhostConsensus::Difficulty storage ($DIFF_STORAGE) != expected retarget result ($DIFF_EXPECTED)" >&2
	exit 1
fi
if [ "$DIFF_EXPECTED" -gt "$DIFF_OLD" ]; then
	echo "retarget raised difficulty $DIFF_OLD -> $DIFF_AFTER (blocks faster than target)"
elif [ "$DIFF_EXPECTED" -lt "$DIFF_OLD" ]; then
	echo "retarget lowered difficulty $DIFF_OLD -> $DIFF_AFTER (blocks slower than target)"
else
	echo "retarget held difficulty at $DIFF_AFTER (computed below/inside MinDifficulty floor or at target)"
fi

echo "==> [6/7] Bob restarts on the same base path and resyncs"
ALICE_BEST_PRE_KILL="$(best_number "$ALICE_RPC")"
kill_node "$BOB_PID"
echo "bob killed at alice best=$ALICE_BEST_PRE_KILL"

# --reserved-nodes on the restart: a node killed moments ago can sit in
# redial backoff while the stale connection ages out on alice's side;
# reserved peers keep re-dialing through it.
start_node "$TMP_DIR/bob-restart.log" \
	--chain local --bob --validator \
	--mine --mining-threads "$MINING_THREADS" --miner-coinbase "$BOB_SS58" \
	--base-path "$TMP_DIR/bob" \
	--node-key "$BOB_NODE_KEY" \
	--rpc-port "$BOB_RPC" --port "$BOB_PORT" --prometheus-port "$BOB_PROM" \
	--bootnodes "/ip4/127.0.0.1/tcp/$ALICE_PORT/p2p/$BOOT_PEER" \
	--reserved-nodes "/ip4/127.0.0.1/tcp/$ALICE_PORT/p2p/$BOOT_PEER" \
	--rpc-methods Unsafe
BOB_PID="$LAST_PID"
wait_rpc "$BOB_RPC"

# Re-peer first — generous grace for the post-kill redial window.
if ! wait_peers_eq "$BOB_RPC" 1 240; then
	echo "restarted bob never re-peered" >&2
	dump_log_tail "$TMP_DIR/bob-restart.log"
	dump_log_tail "$TMP_DIR/alice.log" 30
	exit 1
fi

# Require blocks PAST the kill point so resync can't pass out of bob's
# pre-kill local DB alone.
if ! wait_block_at_least "$BOB_RPC" "$((ALICE_BEST_PRE_KILL + 2))" "bob resync" 240; then
	dump_log_tail "$TMP_DIR/bob-restart.log"
	exit 1
fi
if ! wait_finalized_at_least "$BOB_RPC" "$(finalized_number "$ALICE_RPC")" "bob resync" 240; then
	dump_log_tail "$TMP_DIR/bob-restart.log"
	exit 1
fi
BOB_RESUME_BEST="$(best_number "$BOB_RPC")"
wait_block_at_least "$BOB_RPC" "$((BOB_RESUME_BEST + 3))" "bob post-restart" 180

# Continued authoring on the restarted node: new sealed blocks arrive, and
# Bob's coinbase should reappear as an author (racing miners — bounded wait
# then warn, since equal-hashrate luck can briefly favour Alice).
RESTART_HEADERS="$TMP_DIR/bob-restart-headers"
: >"$RESTART_HEADERS"
RESTART_CURSOR=$((BOB_RESUME_BEST + 1))
RESTART_DEADLINE=$((SECONDS + 120))
while (( SECONDS < RESTART_DEADLINE )); do
	if BEST_SEEN="$(scan_new_headers "$BOB_RPC" "$RESTART_CURSOR" "$RESTART_HEADERS")"; then
		[ -n "$BEST_SEEN" ] && RESTART_CURSOR=$((BEST_SEEN + 1))
	fi
	awk -v b="$BOB_ACCT" '$2==b {found=1} $3!="True" || $4!="True" {bad=1} END {exit !(found && !bad)}' \
		"$RESTART_HEADERS" && break
	sleep 2
done
if awk -v b="$BOB_ACCT" '$2==b' "$RESTART_HEADERS" | grep -q .; then
	echo "bob authored new pow_-sealed blocks after restart"
else
	echo "WARN: no bob-authored block in the post-restart window (chain still advancing sealed blocks)" >&2
fi
if awk '$3!="True" || $4!="True"' "$RESTART_HEADERS" | grep -q .; then
	echo "post-restart block missing pow_ digests" >&2
	exit 1
fi

echo "==> [7/7] Miner/validator rewards minted on-chain"
# Free balance of each coinbase account at the b$RETARGET_ADJUST hash vs
# head after ~30+ more blocks. Both coinbases are also the staked session
# validators, so every block pays them the author (40%) and/or
# validator-share (60%) reward. Block $RETARGET_ADJUST stays inside the
# default pruning window, so the historical read always works.
wait_block_at_least "$ALICE_RPC" $((RETARGET_ADJUST + 30)) "reward window" "${REWARD_TIMEOUT:-1500}"
REWARD_BASE_HASH="$(block_hash_by_number "$ALICE_RPC" "$RETARGET_ADJUST")"
ALICE_BAL_BEFORE="$(account_free_balance "$ALICE_RPC" "$ALICE_ACCT" "$REWARD_BASE_HASH")"
BOB_BAL_BEFORE="$(account_free_balance "$ALICE_RPC" "$BOB_ACCT" "$REWARD_BASE_HASH")"
ALICE_BAL_AFTER="$(account_free_balance "$ALICE_RPC" "$ALICE_ACCT")"
BOB_BAL_AFTER="$(account_free_balance "$ALICE_RPC" "$BOB_ACCT")"
REWARD_WINDOW=$(( $(best_number "$ALICE_RPC") - RETARGET_ADJUST ))
echo "balances at b$RETARGET_ADJUST — alice=$ALICE_BAL_BEFORE bob=$BOB_BAL_BEFORE"
echo "balances at head (window=$REWARD_WINDOW) — alice=$ALICE_BAL_AFTER bob=$BOB_BAL_AFTER"
if (( REWARD_WINDOW < 30 )); then
	echo "reward accrual window too small ($REWARD_WINDOW blocks)" >&2
	exit 1
fi
if (( ALICE_BAL_BEFORE == 0 || BOB_BAL_BEFORE == 0 )); then
	echo "could not read b$RETARGET_ADJUST balances (state pruned or bad key)" >&2
	exit 1
fi
if (( ALICE_BAL_AFTER <= ALICE_BAL_BEFORE )); then
	echo "alice coinbase/validator balance did not increase ($ALICE_BAL_BEFORE -> $ALICE_BAL_AFTER)" >&2
	exit 1
fi
if (( BOB_BAL_AFTER <= BOB_BAL_BEFORE )); then
	echo "bob coinbase/validator balance did not increase ($BOB_BAL_BEFORE -> $BOB_BAL_AFTER)" >&2
	exit 1
fi
echo "rewards minted over $REWARD_WINDOW blocks: alice +$((ALICE_BAL_AFTER - ALICE_BAL_BEFORE)), bob +$((BOB_BAL_AFTER - BOB_BAL_BEFORE)) planck"

echo "==> Ghost-consensus E2E gate passed"
