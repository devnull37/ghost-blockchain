#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/ghost-target}"
BIN="$TARGET_DIR/debug/ghost-node"

export CARGO_TARGET_DIR="$TARGET_DIR"
export WASM_BUILD_WORKSPACE_HINT="${WASM_BUILD_WORKSPACE_HINT:-$ROOT_DIR}"
export LIBCLANG_PATH="${LIBCLANG_PATH:-/lib/llvm-18/lib}"
export BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:--I/usr/lib/gcc/x86_64-linux-gnu/13/include -I/usr/include/x86_64-linux-gnu -I/usr/include}"

DEV_RPC=19944
DEV_PORT=31333
ALICE_RPC=19945
ALICE_PORT=31334
BOB_RPC=19946
BOB_PORT=31335
ALICE_PROM=19615
BOB_PROM=19616
ALICE_NODE_KEY=0000000000000000000000000000000000000000000000000000000000000001
BOB_NODE_KEY=0000000000000000000000000000000000000000000000000000000000000002
ALICE_PEER=12D3KooWEyoppNCUx8Yx66oV9fJnriXwCcXwDDUA2kj6vnc6iDEp

PIDS=()
TMP_DIR="$(mktemp -d /tmp/ghost-e2e.XXXXXX)"

cleanup() {
	for pid in "${PIDS[@]:-}"; do
		kill "$pid" >/dev/null 2>&1 || true
	done
	wait >/dev/null 2>&1 || true
	rm -rf "$TMP_DIR"
}
trap cleanup EXIT

rpc() {
	local port="$1"
	local method="$2"
	local params="${3:-[]}"
	curl -fsS \
		-H 'Content-Type: application/json' \
		-d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":$params}" \
		"http://127.0.0.1:$port"
}

json_field() {
	local expr="$1"
	python3 -c "import json,sys; data=json.load(sys.stdin); print($expr)"
}

wait_rpc() {
	local port="$1"
	for _ in $(seq 1 60); do
		if rpc "$port" system_health >/dev/null 2>&1; then
			return 0
		fi
		sleep 1
	done
	echo "RPC port $port did not become ready" >&2
	return 1
}

wait_block_at_least() {
	local port="$1"
	local min_block="$2"
	local label="$3"
	for _ in $(seq 1 90); do
		local number
		number="$(rpc "$port" chain_getHeader | json_field "int(data['result']['number'], 16)")"
		if (( number >= min_block )); then
			echo "$label best block: $number"
			return 0
		fi
		sleep 1
	done
	echo "$label did not reach block $min_block" >&2
	return 1
}

wait_finalized_at_least() {
	local port="$1"
	local min_block="$2"
	local label="$3"
	for _ in $(seq 1 120); do
		local hash number
		hash="$(rpc "$port" chain_getFinalizedHead | json_field "data['result']")"
		number="$(rpc "$port" chain_getHeader "[\"$hash\"]" | json_field "int(data['result']['number'], 16)")"
		if (( number >= min_block )); then
			echo "$label finalized block: $number"
			return 0
		fi
		sleep 1
	done
	echo "$label did not finalize block $min_block" >&2
	return 1
}

echo "==> Building ghost-node with embedded Wasm"
cargo build --bin ghost-node

echo "==> Running Ghost CLI smoke checks"
"$BIN" ghost status --detailed >/dev/null
"$BIN" ghost mine --threads 1 --difficulty 18446744073709551615 >/dev/null

echo "==> Running single-node dev authoring smoke test"
"$BIN" \
	--dev \
	--tmp \
	--rpc-port "$DEV_RPC" \
	--port "$DEV_PORT" \
	--no-telemetry \
	-l warn \
	>"$TMP_DIR/dev.log" 2>&1 &
PIDS+=("$!")
wait_rpc "$DEV_RPC"
wait_block_at_least "$DEV_RPC" 2 "dev"
rpc "$DEV_RPC" ghost_getConsensusMode >/dev/null
kill "${PIDS[-1]}" >/dev/null 2>&1 || true
wait "${PIDS[-1]}" >/dev/null 2>&1 || true
unset 'PIDS[-1]'

echo "==> Running two-node local chain authoring/finality smoke test"
rm -rf /tmp/ghost-e2e-alice /tmp/ghost-e2e-bob
"$BIN" \
	--chain local \
	--alice \
	--validator \
	--base-path /tmp/ghost-e2e-alice \
	--node-key "$ALICE_NODE_KEY" \
	--port "$ALICE_PORT" \
	--rpc-port "$ALICE_RPC" \
	--prometheus-port "$ALICE_PROM" \
	--no-telemetry \
	-l warn \
	>"$TMP_DIR/alice.log" 2>&1 &
PIDS+=("$!")
wait_rpc "$ALICE_RPC"

"$BIN" \
	--chain local \
	--bob \
	--validator \
	--base-path /tmp/ghost-e2e-bob \
	--node-key "$BOB_NODE_KEY" \
	--port "$BOB_PORT" \
	--rpc-port "$BOB_RPC" \
	--prometheus-port "$BOB_PROM" \
	--bootnodes "/ip4/127.0.0.1/tcp/$ALICE_PORT/p2p/$ALICE_PEER" \
	--no-telemetry \
	-l warn \
	>"$TMP_DIR/bob.log" 2>&1 &
PIDS+=("$!")
wait_rpc "$BOB_RPC"

wait_block_at_least "$ALICE_RPC" 8 "alice"
wait_block_at_least "$BOB_RPC" 8 "bob"
wait_finalized_at_least "$ALICE_RPC" 6 "alice"
wait_finalized_at_least "$BOB_RPC" 6 "bob"

ALICE_HEALTH="$(rpc "$ALICE_RPC" system_health)"
BOB_HEALTH="$(rpc "$BOB_RPC" system_health)"
ALICE_PEERS="$(printf '%s' "$ALICE_HEALTH" | json_field "data['result']['peers']")"
BOB_PEERS="$(printf '%s' "$BOB_HEALTH" | json_field "data['result']['peers']")"

if (( ALICE_PEERS < 1 || BOB_PEERS < 1 )); then
	echo "Expected both nodes to have at least one peer; got Alice=$ALICE_PEERS Bob=$BOB_PEERS" >&2
	exit 1
fi

echo "alice peers: $ALICE_PEERS"
echo "bob peers: $BOB_PEERS"

echo "==> Restarting Bob and checking resync/finality"
BOB_PID="${PIDS[-1]}"
kill "$BOB_PID" >/dev/null 2>&1 || true
wait "$BOB_PID" >/dev/null 2>&1 || true
unset 'PIDS[-1]'

"$BIN" \
	--chain local \
	--bob \
	--validator \
	--base-path /tmp/ghost-e2e-bob \
	--node-key "$BOB_NODE_KEY" \
	--port "$BOB_PORT" \
	--rpc-port "$BOB_RPC" \
	--prometheus-port "$BOB_PROM" \
	--bootnodes "/ip4/127.0.0.1/tcp/$ALICE_PORT/p2p/$ALICE_PEER" \
	--no-telemetry \
	-l warn \
	>"$TMP_DIR/bob-restart.log" 2>&1 &
PIDS+=("$!")

wait_rpc "$BOB_RPC"
wait_block_at_least "$BOB_RPC" 10 "bob after restart"
wait_finalized_at_least "$BOB_RPC" 8 "bob after restart"

BOB_RESTART_PEERS="$(rpc "$BOB_RPC" system_health | json_field "data['result']['peers']")"
if (( BOB_RESTART_PEERS < 1 )); then
	echo "Expected restarted Bob to reconnect to at least one peer; got $BOB_RESTART_PEERS" >&2
	exit 1
fi
echo "bob restart peers: $BOB_RESTART_PEERS"
echo "==> Local E2E smoke test passed"
