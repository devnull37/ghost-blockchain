#!/usr/bin/env bash
# e2e-governance.sh — governance lifecycle drill on a live dev chain.
#
# Boots a --dev node and drives, via Polkadot-JS:
#   preimage note -> council propose/vote/close (external proposal)
#   -> council fast_track -> democracy vote -> root enactment of
#   balances.force_set_balance -> finality-still-advancing assert.
#
# Env:
#   GHOST_NODE_BIN / BIN   node binary path (default ./target/debug/ghost-node)
#   KEEP_BASE              keep chain dir
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib-ghost-rpc.sh"

BASE="$(mktemp -d "${TMPDIR:-/tmp}/ghost-gov-XXXXXX")"
PORT=29980
RPCPORT=29981
PROMPORT=31471
NODE_PID=""
BIN="${GHOST_NODE_BIN:-${BIN:-$ROOT/target/debug/ghost-node}}"

cleanup() {
    [ -n "$NODE_PID" ] && kill "$NODE_PID" 2>/dev/null || true
    [ -z "${KEEP_BASE:-}" ] && rm -rf "$BASE"
}
trap cleanup EXIT

[ -x "$BIN" ] || {
    echo "== building node =="
    (cd "$ROOT" && rtk env WASM_BUILD_WORKSPACE_HINT="$ROOT" cargo build --bin ghost-node)
}

echo "== booting dev node (rpc $RPCPORT) =="
"$BIN" --dev -d "$BASE" \
    --mine --miner-coinbase 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY \
    --rpc-port "$RPCPORT" --port "$PORT" --prometheus-port "$PROMPORT" \
    --rpc-methods Unsafe \
    > "$BASE/node.log" 2>&1 &
NODE_PID=$!
wait_rpc "$RPCPORT" 120

cd "$HERE/upgrade-drill"
[ -d node_modules/@polkadot/api ] || npm install --no-audit --no-fund @polkadot/api >/dev/null
node governance-drill.js "ws://127.0.0.1:$RPCPORT"

echo "== e2e-governance PASSED =="
