#!/usr/bin/env bash
# e2e-upgrade.sh — runtime-upgrade drill on a live dev chain.
#
# Builds a spec-bumped runtime wasm (spec_version +1), boots a --dev node
# running the CURRENT spec, submits sudo(system.set_code(wasm)), and
# asserts the new spec_version goes live and blocks keep finalizing.
#
# Env:
#   NEW_SPEC_VERSION   spec to bump to (default: current+1)
#   GHOST_NODE_BIN     prebuilt node binary (skips cargo build)
#   KEEP_BASE          keep chain dir
#   BIN                node binary path override (default ./target/debug/ghost-node)
#
# Requires: node/npm for the drill driver (scripts/upgrade-drill/).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
source "$HERE/lib-ghost-rpc.sh"

BASE="$(mktemp -d "${TMPDIR:-/tmp}/ghost-upgrade-XXXX")"
PORT=29970
RPCPORT=29971
PROMPORT=31461
NODE_PID=""
BIN="${BIN:-$ROOT/target/debug/ghost-node}"

cleanup() {
    [ -n "$NODE_PID" ] && kill "$NODE_PID" 2>/dev/null || true
    [ -z "${KEEP_BASE:-}" ] && rm -rf "$BASE"
}
trap cleanup EXIT

cd "$ROOT"

# ── 1. Build a spec-bumped runtime wasm ────────────────────────────────
CUR_SPEC=$(grep -oP 'spec_version:\s*\K[0-9]+' runtime/src/lib.rs | head -1)
NEW_SPEC="${NEW_SPEC_VERSION:-$((CUR_SPEC + 1))}"
echo "== bumping spec_version $CUR_SPEC -> $NEW_SPEC for drill wasm =="

sed -i "s/spec_version: ${CUR_SPEC},/spec_version: ${NEW_SPEC},/" runtime/src/lib.rs
restore_spec() {
    sed -i "s/spec_version: ${NEW_SPEC},/spec_version: ${CUR_SPEC},/" runtime/src/lib.rs
}
trap 'restore_spec; cleanup' EXIT

echo "== building drill runtime wasm (cargo build -p solochain-template-runtime) =="
rtk cargo build -p solochain-template-runtime 2>&1 | tail -2

WASM_FILE="$ROOT/target/debug/wbuild/solochain-template-runtime/solochain_template_runtime.wasm"
[ -f "$WASM_FILE" ] || { echo "drill wasm not found at $WASM_FILE"; exit 1; }
restore_spec
trap cleanup EXIT

# ── 2. Boot dev node on the OLD spec ──────────────────────────────────
[ -x "$BIN" ] || {
    echo "== building node (embedded wasm at spec $CUR_SPEC) =="
    rtk env WASM_BUILD_WORKSPACE_HINT="$ROOT" cargo build --bin ghost-node
}

echo "== booting dev node (port $PORT) =="
"$BIN" --dev --tmp -d "$BASE" \
    --mine --miner-coinbase d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d \
    --rpc-port "$RPCPORT" --port "$PORT" --prometheus-port "$PROMPORT" \
    --rpc-methods Unsafe \
    > "$BASE/node.log" 2>&1 &
NODE_PID=$!
wait_rpc "$RPCPORT" 120

# ── 3. Drill ───────────────────────────────────────────────────────────
cd "$HERE/upgrade-drill"
[ -d node_modules/@polkadot/api ] || npm install --no-audit --no-fund @polkadot/api >/dev/null
node drill.js "ws://127.0.0.1:$RPCPORT" "$WASM_FILE" "$NEW_SPEC"

echo "== e2e-upgrade PASSED =="
