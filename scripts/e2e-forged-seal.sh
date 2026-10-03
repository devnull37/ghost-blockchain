#!/usr/bin/env bash
# e2e-forged-seal.sh — invalid-PoW rejection gate.
#
# An "evil" binary (scripts/forged-seal.patch applied at build time) authors
# seals whose embedded `pre_hash` is corrupted to 0xEE…; its own verify()
# accepts them so it mines and gossips continuously. Every honest node runs
# the real `verify_seal` and must reject each forged block at the embedded
# pre_hash equality check — proven by scanning every header on an honest
# node: none may be authored by the evil coinbase.
#
# Env:
#   GHOST_NODE_BIN       honest binary (default: target/debug/ghost-node)
#   GHOST_EVIL_NODE_BIN  prebuilt evil binary; when unset the script applies
#                        forged-seal.patch, builds, then restores the tree
#   MINING_THREADS       per-miner grind threads (default 2)
#   WINDOW_BLOCKS        honest blocks to observe (default 8)
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
BIN="${GHOST_NODE_BIN:-$TARGET_DIR/debug/ghost-node}"

export GHOST_E2E_HELPER_DIR="${GHOST_E2E_HELPER_DIR:-}"
# shellcheck source=scripts/lib-ghost-rpc.sh
. "$(dirname "${BASH_SOURCE[0]}")/lib-ghost-rpc.sh"

MINING_THREADS="${MINING_THREADS:-2}"
WINDOW_BLOCKS="${WINDOW_BLOCKS:-8}"

EVE_ACCT=e659a7a1628cdd93febc04a4e0646ea20e9f5f0ce097d9a05290d4a9e054df4e
EVE_SS58=5HGjWAeFDfFCWPsjFQdVV2Msvz2XtMktvgocEZcCj68kUMaw
ALICE_SS58=5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY
CHARLIE_SS58=5FLSigC9HGRKVhB9FiEo4Y3koPsNmBmLJbpXg2mp1hXcS59Y

# Ports disjoint from e2e-ghost.sh (2994x/3143x) and e2e-faults.sh (2995x/3144x).
A_RPC=29960; M_RPC=29961; E_RPC=29962
A_PORT=31450; M_PORT=31451; E_PORT=31452
A_KEY=0000000000000000000000000000000000000000000000000000000000000001
M_KEY=0000000000000000000000000000000000000000000000000000000000000002
E_KEY=0000000000000000000000000000000000000000000000000000000000000005
A_PEER=12D3KooWEyoppNCUx8Yx66oV9fJnriXwCcXwDDUA2kj6vnc6iDEp

declare -A LIVE_PIDS=()
LAST_PID=""
TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ghost-forged.XXXXXX")"
GHOST_E2E_HELPER_DIR="$TMP_DIR"
trap 'kill $(printf "%s " "${!LIVE_PIDS[@]}") 2>/dev/null; sleep 1; kill -9 $(printf "%s " "${!LIVE_PIDS[@]}") 2>/dev/null; rm -rf "$TMP_DIR"' EXIT

start() {
	local log="$1"; shift
	"$@" --no-telemetry -l warn >"$log" 2>&1 &
	LAST_PID=$!
	LIVE_PIDS[$LAST_PID]=1
}

echo "==> evil binary"
if [ -n "${GHOST_EVIL_NODE_BIN:-}" ]; then
	EVIL_BIN="$GHOST_EVIL_NODE_BIN"
else
	EVIL_BIN="$TMP_DIR/ghost-node-evil"
	# Stash the honest binary FIRST: the evil build overwrites
	# $TARGET_DIR/debug/ghost-node on disk, and the "honest" nodes start
	# from that path — without this stash they'd run the evil binary too.
	HONEST_BIN="$TMP_DIR/ghost-node-honest"
	cp "$BIN" "$HONEST_BIN"
	BIN="$HONEST_BIN"
	# Guard against a dirty/partially-patched tree: `patch -f` on an
	# already-applied patch silently REVERSES (GNU patch behaviour), which
	# previously left the tree evil-side-out. Refuse ambiguous state.
	if grep -q "0xEE; 32" "$ROOT_DIR/consensus/ghost-consensus/src/mining.rs"; then
		echo "forged-seal.patch already applied — restore the tree first (git checkout consensus/ghost-consensus/src/{mining,algorithm}.rs)" >&2
		exit 1
	fi
	restore_tree() {
		# Idempotent: only acts while the marker is present.
		if grep -q "0xEE; 32" "$ROOT_DIR/consensus/ghost-consensus/src/mining.rs"; then
			( cd "$ROOT_DIR" && patch -f -R -p1 < scripts/forged-seal.patch ) \
				|| ( cd "$ROOT_DIR" && git checkout -- \
					consensus/ghost-consensus/src/mining.rs \
					consensus/ghost-consensus/src/algorithm.rs )
		fi
		# Hard verify: the marker must be gone after restore.
		if grep -q "0xEE; 32" "$ROOT_DIR/consensus/ghost-consensus/src/mining.rs"; then
			( cd "$ROOT_DIR" && git checkout -- \
				consensus/ghost-consensus/src/mining.rs \
				consensus/ghost-consensus/src/algorithm.rs )
		fi
	}
	echo "applying scripts/forged-seal.patch"
	( cd "$ROOT_DIR" && patch -f -p1 < scripts/forged-seal.patch )
	grep -q "0xEE; 32" "$ROOT_DIR/consensus/ghost-consensus/src/mining.rs" || {
		echo "patch did not take effect — aborting before an honest-only build" >&2
		exit 1
	}
	trap 'kill $(printf "%s " "${!LIVE_PIDS[@]}") 2>/dev/null; sleep 1; kill -9 $(printf "%s " "${!LIVE_PIDS[@]}") 2>/dev/null; restore_tree; rm -rf "$TMP_DIR"' EXIT
	( cd "$ROOT_DIR" && cargo build --bin ghost-node )
	cp "$TARGET_DIR/debug/ghost-node" "$EVIL_BIN"
	restore_tree
	# Rebuild the honest binary so the tree's binary matches the tree again
	# (the file on disk is currently the evil build).
	( cd "$ROOT_DIR" && cargo build --bin ghost-node )
	echo "evil binary built at $EVIL_BIN (tree + honest binary restored)"
fi
"$EVIL_BIN" --version

echo "==> honest network up (alice committee+miner, miner1 keyless)"
start "$TMP_DIR/a.log" "$BIN" \
	--chain local --alice --validator \
	--mine --mining-threads "$MINING_THREADS" --miner-coinbase "$ALICE_SS58" \
	--base-path "$TMP_DIR/a" --node-key "$A_KEY" \
	--port "$A_PORT" --rpc-port "$A_RPC"
A_PID=$LAST_PID
wait_rpc "$A_RPC"

start "$TMP_DIR/m.log" "$BIN" \
	--chain local --mine --mining-threads "$MINING_THREADS" \
	--miner-coinbase "$CHARLIE_SS58" \
	--base-path "$TMP_DIR/m" --node-key "$M_KEY" \
	--port "$M_PORT" --rpc-port "$M_RPC" \
	--bootnodes "/ip4/127.0.0.1/tcp/$A_PORT/p2p/$A_PEER"
M_PID=$LAST_PID
wait_rpc "$M_RPC"
wait_peers_eq "$M_RPC" 1

base="$(best_number "$A_RPC")"
wait_block_at_least "$A_RPC" $((base + 3)) "honest alice" 300

echo "==> evil node joins and forges"
start "$TMP_DIR/e.log" "$EVIL_BIN" \
	--chain local --mine --mining-threads "$MINING_THREADS" \
	--miner-coinbase "$EVE_SS58" \
	--base-path "$TMP_DIR/e" --node-key "$E_KEY" \
	--port "$E_PORT" --rpc-port "$E_RPC" \
	--bootnodes "/ip4/127.0.0.1/tcp/$A_PORT/p2p/$A_PEER"
E_PID=$LAST_PID
wait_rpc "$E_RPC"
wait_peers_eq "$E_RPC" 2 120

# Prove the run is not vacuous: the evil node must itself be producing
# (its own best advances and its own headers carry EVE as author).
evil_base="$(best_number "$E_RPC")"
wait_block_at_least "$E_RPC" $((evil_base + 2)) "evil" 300
scan_pow_headers "$E_RPC" 1 "$(best_number "$E_RPC")" >"$TMP_DIR/evil-headers.txt"
if ! grep -q " $EVE_ACCT " "$TMP_DIR/evil-headers.txt"; then
	echo "evil node authored nothing — test is vacuous" >&2
	tail -30 "$TMP_DIR/e.log" >&2
	exit 1
fi
echo "evil node authored blocks on its own view (vacuousness check ok)"

# The gate: over a window of honest progress, no EVE-authored header may
# appear on the honest node's chain — every forged seal is rejected.
target=$(( $(best_number "$A_RPC") + WINDOW_BLOCKS ))
wait_block_at_least "$A_RPC" "$target" "honest alice" 600
scan_pow_headers "$A_RPC" 1 "$(best_number "$A_RPC")" >"$TMP_DIR/honest-headers.txt"
if grep -q " $EVE_ACCT " "$TMP_DIR/honest-headers.txt"; then
	echo "FORGED SEAL IMPORTED on honest node — consensus breach" >&2
	grep " $EVE_ACCT " "$TMP_DIR/honest-headers.txt" >&2
	exit 1
fi
honest_total=$(wc -l < "$TMP_DIR/honest-headers.txt")
echo "scanned $honest_total honest headers: zero forged blocks imported"

# Sanity: honest chain is still healthy alongside the attack.
fin="$(finalized_number "$A_RPC")"
[ "$fin" -ge 1 ] || { echo "finality never advanced on honest node" >&2; exit 1; }
echo "finalized=$fin — honest chain healthy under forged-seal spam"
echo "PASS: honest nodes rejected every forged seal (verified end-to-end)"
