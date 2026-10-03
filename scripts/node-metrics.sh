#!/usr/bin/env bash
# node-metrics.sh — point-in-time health/chain/sync snapshot of a running
# ghost-node over its JSON-RPC HTTP endpoint.
#
# Usage:
#   scripts/node-metrics.sh [rpc-url]
#   GHOST_RPC_URL=http://127.0.0.1:19933 scripts/node-metrics.sh
#
# Prints one `key=value` per line so callers can scrape it; for a live view:
#   watch -n5 scripts/node-metrics.sh
#
# Requires: curl, python3. Default RPC URL: http://127.0.0.1:9933.
set -euo pipefail

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
	sed -n '2,12p' "${BASH_SOURCE[0]}"
	exit 0
fi

RPC_URL="${1:-${GHOST_RPC_URL:-http://127.0.0.1:9933}}"

rpc() {
	local method="$1"
	local params="${2:-[]}"
	curl -fsS --max-time 10 \
		-H 'Content-Type: application/json' \
		-d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":$params}" \
		"$RPC_URL"
}

json_field() {
	local expr="$1"
	python3 -c "import json,sys; data=json.load(sys.stdin); print($expr)"
}

NODE_NAME="$(rpc system_name | json_field "data['result']")"
NODE_VERSION="$(rpc system_version | json_field "data['result']")"
CHAIN="$(rpc system_chain | json_field "data['result']")"

HEALTH="$(rpc system_health)"
PEERS="$(printf '%s' "$HEALTH" | json_field "data['result']['peers']")"
IS_SYNCING="$(printf '%s' "$HEALTH" | json_field "data['result']['isSyncing']")"
SHOULD_HAVE_PEERS="$(printf '%s' "$HEALTH" | json_field "data['result']['shouldHavePeers']")"

BEST_NUMBER="$(rpc chain_getHeader | json_field "int(data['result']['number'], 16)")"
FINALIZED_HASH="$(rpc chain_getFinalizedHead | json_field "data['result']")"
FINALIZED_NUMBER="$(rpc chain_getHeader "[\"$FINALIZED_HASH\"]" | json_field "int(data['result']['number'], 16)")"
FINALITY_LAG=$((BEST_NUMBER - FINALIZED_NUMBER))

SYNC_STATE="$(rpc system_syncState)"
CURRENT_BLOCK="$(printf '%s' "$SYNC_STATE" | json_field "data['result']['currentBlock']")"
HIGHEST_BLOCK="$(printf '%s' "$SYNC_STATE" | json_field "data['result'].get('highestBlock', data['result']['currentBlock'])")"

cat <<EOF
node_name=$NODE_NAME
node_version=$NODE_VERSION
rpc_url=$RPC_URL
chain=$CHAIN
best_block=$BEST_NUMBER
finalized_block=$FINALIZED_NUMBER
finality_lag_blocks=$FINALITY_LAG
peers=$PEERS
is_syncing=$IS_SYNCING
should_have_peers=$SHOULD_HAVE_PEERS
sync_current_block=$CURRENT_BLOCK
sync_highest_block=$HIGHEST_BLOCK
EOF
