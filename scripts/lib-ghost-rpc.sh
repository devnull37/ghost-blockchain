# Shared helpers for Ghost-consensus e2e scripts.
#
# Sourced by scripts/e2e-ghost.sh. Provides:
#   * rpc / json_field / wait_*            — JSON-RPC plumbing (curl + python3)
#   * best_number / finalized_number       — chain head helpers
#   * scan_pow_headers                     — `pow_` digest decoding via python3
#   * wait_peers_eq                        — peers==N with grace for drops
#   * storage_at                           — state_getStorage (opt. block hash)
#   * account_free_balance                 — System::Account free balance
#   * ghost_next_difficulty                — state_call GhostPowApi_next_difficulty
#   * ghost_difficulty_storage             — GhostConsensus::Difficulty storage read
#
# Digest wire format (sp_runtime::generic::DigestItem):
#   variant byte ++ engine id (4B) ++ compact-len ++ payload
#   Seal        = 0x05 ++ "pow_" (0x706f775f) ++ len ++ SCALE(GhostSeal{nonce:u64})
#   PreRuntime  = 0x06 ++ "pow_" (0x706f775f) ++ len ++ SCALE(AccountId32)
# so "05706f775f" in a log hex proves a Ghost PoW seal and "06706f775f" proves
# the seal-bound miner pre-runtime.

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

# All SCALE / hashing decoding lives in one python helper written out lazily
# (so `... | ghost_scale` still sees the piped stdin). Set
# GHOST_E2E_HELPER_DIR before the first call to control where it lands.
GHOST_SCALE_PY_FILE=""
ghost_scale() {
	if [ -z "$GHOST_SCALE_PY_FILE" ]; then
		local dir="${GHOST_E2E_HELPER_DIR:-}"
		[ -n "$dir" ] || dir="$(mktemp -d "${TMPDIR:-/tmp}/ghost-scale.XXXXXX")"
		GHOST_SCALE_PY_FILE="$dir/ghost_scale.py"
		cat >"$GHOST_SCALE_PY_FILE" <<'PYEOF'
import hashlib
import json
import sys
import urllib.request

MASK64 = (1 << 64) - 1


def rpc_call(port: str, method: str, params=None):
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}",
        data=json.dumps({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params or [],
        }).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        body = json.loads(resp.read())
    if "error" in body:
        raise RuntimeError(f"{method}: {body['error']}")
    return body["result"]


def xxh64(data: bytes, seed: int = 0) -> int:
    """Pure-python XXH64 — needed for twox_128 storage keys (no xxhash dep)."""
    p1 = 11400714785074694791
    p2 = 14029467366897019727
    p3 = 1609587929392839161
    p4 = 9650029242287828579
    p5 = 2870177450012600261

    def rotl(x: int, r: int) -> int:
        return ((x << r) | (x >> (64 - r))) & MASK64

    def rnd(acc: int, lane: int) -> int:
        acc = (acc + lane * p2) & MASK64
        acc = rotl(acc, 31)
        return (acc * p1) & MASK64

    n = len(data)
    i = 0
    if n >= 32:
        v1 = (seed + p1 + p2) & MASK64
        v2 = (seed + p2) & MASK64
        v3 = seed & MASK64
        v4 = (seed - p1) & MASK64
        while i + 32 <= n:
            v1 = rnd(v1, int.from_bytes(data[i:i + 8], "little"))
            v2 = rnd(v2, int.from_bytes(data[i + 8:i + 16], "little"))
            v3 = rnd(v3, int.from_bytes(data[i + 16:i + 24], "little"))
            v4 = rnd(v4, int.from_bytes(data[i + 24:i + 32], "little"))
            i += 32
        h = (rotl(v1, 1) + rotl(v2, 7) + rotl(v3, 12) + rotl(v4, 18)) & MASK64
        for v in (v1, v2, v3, v4):
            h ^= rnd(0, v)
            h = (h * p1 + p4) & MASK64
    else:
        h = (seed + p5) & MASK64
    h = (h + n) & MASK64
    while i + 8 <= n:
        h ^= rnd(0, int.from_bytes(data[i:i + 8], "little"))
        h = (rotl(h, 27) * p1 + p4) & MASK64
        i += 8
    if i + 4 <= n:
        h ^= (int.from_bytes(data[i:i + 4], "little") * p1) & MASK64
        h = (rotl(h, 23) * p2 + p3) & MASK64
        i += 4
    while i < n:
        h ^= (data[i] * p5) & MASK64
        h = (rotl(h, 11) * p1) & MASK64
        i += 1
    h ^= h >> 33
    h = (h * p2) & MASK64
    h ^= h >> 29
    h = (h * p3) & MASK64
    h ^= h >> 32
    return h


def twox128(name: str) -> bytes:
    b = name.encode()
    return (
        xxh64(b, 0).to_bytes(8, "little") + xxh64(b, 1).to_bytes(8, "little")
    )


def unhex(s: str) -> bytes:
    s = s.strip()
    if s.startswith("0x"):
        s = s[2:]
    return bytes.fromhex(s)


def compact_len(buf: bytes, off: int):
    """Decode a SCALE compact length at off -> (length, bytes_consumed)."""
    b0 = buf[off]
    mode = b0 & 0b11
    if mode == 0:
        return b0 >> 2, 1
    if mode == 1:
        return int.from_bytes(buf[off:off + 2], "little") >> 2, 2
    if mode == 2:
        return int.from_bytes(buf[off:off + 4], "little") >> 2, 4
    nbytes = (b0 >> 2) + 4
    return int.from_bytes(buf[off + 1:off + 1 + nbytes], "little"), 1 + nbytes


def parse_digest_item(hexstr: str):
    """-> (variant, engine, payload_bytes). engine is '' for Other."""
    b = unhex(hexstr)
    variant = b[0]
    if variant in (4, 5, 6):  # Consensus / Seal / PreRuntime
        engine = b[1:5].decode("ascii", "replace")
        ln, used = compact_len(b, 5)
        return variant, engine, b[5 + used:5 + used + ln]
    if variant == 0:  # Other
        ln, used = compact_len(b, 1)
        return variant, "", b[1 + used:1 + used + ln]
    return variant, "", b""


def header_pow_info(header: dict):
    """-> (number, [authors], has_pow_seal, has_pow_preruntime)."""
    logs = header["digest"]["logs"]
    authors = []
    has_seal = False
    has_pre = False
    for log in logs:
        variant, engine, payload = parse_digest_item(log)
        if engine != "pow_":
            continue
        if variant == 5:
            has_seal = True
        elif variant == 6:
            has_pre = True
            if len(payload) >= 32:
                authors.append(payload[:32].hex())
    return int(header["number"], 16), authors, has_seal, has_pre


def storage_key(pallet: str, item: str) -> str:
    return "0x" + (twox128(pallet) + twox128(item)).hex()


def system_account_key(account_hex: str) -> str:
    account = unhex(account_hex)
    hash16 = hashlib.blake2b(account, digest_size=16).digest()
    return "0x" + (twox128("System") + twox128("Account") + hash16 + account).hex()


def account_free(value_hex: str) -> int:
    """System::Account value -> free balance (AccountInfo then AccountData)."""
    b = unhex(value_hex)
    # AccountInfo: nonce/consumers/providers/sufficients u32 x4, then
    # AccountData{ free:u128, .. } — `free` is always the first u128.
    return int.from_bytes(b[16:32], "little")


def u256_le(value_hex: str) -> int:
    b = unhex(value_hex)
    return int.from_bytes(b[:32], "little")


cmd = sys.argv[1]
if cmd == "twox128":
    print("0x" + twox128(sys.argv[2]).hex())
elif cmd == "storage_key":
    print(storage_key(sys.argv[2], sys.argv[3]))
elif cmd == "account_key":
    print(system_account_key(sys.argv[2]))
elif cmd == "map_key":
    # map_key <pallet> <item> <key-hex-no-0x> [hasher] — StorageMap entry key.
    # hasher defaults to Blake2_128Concat (what FRAME uses for AccountId
    # maps in this workspace, e.g. GhostConsensus::Bonded):
    #   twox128(pallet) ++ twox128(item) ++ blake2_128(key) ++ key
    # Pass "twox64concat" explicitly for Twox64Concat maps instead:
    #   twox128(pallet) ++ twox128(item) ++ xxh64(key)LE ++ key
    pallet, item = sys.argv[2], sys.argv[3]
    key = unhex(sys.argv[4])
    hasher = sys.argv[5] if len(sys.argv) > 5 else "blake2_128concat"
    if hasher == "twox64concat":
        h = xxh64(key, 0).to_bytes(8, "little")
    else:
        h = hashlib.blake2b(key, digest_size=16).digest()
    print("0x" + (twox128(pallet) + twox128(item) + h + key).hex())
elif cmd == "account_free":
    print(account_free(sys.argv[2]))
elif cmd == "u256le":
    print(u256_le(sys.argv[2]))
elif cmd == "uintle":
    print(int.from_bytes(unhex(sys.argv[2]), "little"))
elif cmd == "retarget_expect":
    # Mirror of pallet_ghost_consensus::Pallet::retarget — given the pre-retarget
    # difficulty, measured elapsed ms across one RetargetInterval window, the
    # window's expected ms (interval * TargetBlockTimeMs), and MinDifficulty:
    # >4x fast -> *4, >4x slow -> /4, else *expected/elapsed, floored at min.
    old, elapsed, expected, mind = (int(x) for x in sys.argv[2:6])
    if elapsed == 0 or elapsed * 4 < expected:
        num, den = 4, 1
    elif elapsed > expected * 4:
        num, den = 1, 4
    else:
        num, den = expected, elapsed
    print(max(old * num // den, mind))
elif cmd == "header_info":
    number, authors, has_seal, has_pre = header_pow_info(
        json.loads(sys.stdin.read())["result"]
    )
    print(json.dumps({
        "number": number,
        "authors": authors,
        "has_pow_seal": has_seal,
        "has_pow_preruntime": has_pre,
    }))
elif cmd == "scan":
    # scan <port> <from> <to> — one line per block:
    #   <number> <author_hex|-> <has_pow_seal> <has_pow_preruntime>
    port, frm, to = sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
    for n in range(frm, to + 1):
        block_hash = rpc_call(port, "chain_getBlockHash", [n])
        header = rpc_call(port, "chain_getHeader", [block_hash])
        number, authors, seal, pre = header_pow_info(header)
        print(number, authors[0] if authors else "-", seal, pre)
elif cmd == "selftest":
    # Known-good constants: twox128("System") and twox128("Account") are the
    # prefixes of the well-known System::Account storage key.
    assert twox128("System").hex() == "26aa394eea5630e07c48ae0c9558cef7"
    assert twox128("Account").hex() == "b99d880ec681799c0cf30e8886371da9"
    # Digest parse sanity: Seal("pow_", [8B nonce]) and
    # PreRuntime("pow_", [32B account]).
    v, e, p = parse_digest_item("0x05706f775f20" + "01" * 8)
    assert (v, e, len(p)) == (5, "pow_", 8)
    v, e, p = parse_digest_item("0x06706f775f80" + "ab" * 32)
    assert (v, e, len(p)) == (6, "pow_", 32)
    print("selftest ok")
else:
    sys.exit(f"unknown ghost_scale subcommand: {cmd}")
PYEOF
	fi
	python3 "$GHOST_SCALE_PY_FILE" "$@"
}

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

best_number() {
	local port="$1"
	rpc "$port" chain_getHeader | json_field "int(data['result']['number'], 16)"
}

finalized_number() {
	local port="$1"
	local hash
	hash="$(rpc "$port" chain_getFinalizedHead | json_field "data['result']")"
	rpc "$port" chain_getHeader "[\"$hash\"]" | json_field "int(data['result']['number'], 16)"
}

peer_count() {
	local port="$1"
	rpc "$port" system_health | json_field "data['result']['peers']"
}

wait_block_at_least() {
	local port="$1"
	local min_block="$2"
	local label="$3"
	local tries="${4:-90}"
	for i in $(seq 1 "$tries"); do
		local number
		number="$(best_number "$port" 2>/dev/null)" || number=0
		if (( number >= min_block )); then
			echo "$label best block: $number"
			return 0
		fi
		if (( i % 30 == 0 )); then
			echo "  $label at block $number (waiting for $min_block)"
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
	local tries="${4:-120}"
	for _ in $(seq 1 "$tries"); do
		local number
		number="$(finalized_number "$port" 2>/dev/null)" || number=0
		if (( number >= min_block )); then
			echo "$label finalized block: $number"
			return 0
		fi
		sleep 1
	done
	echo "$label did not finalize block $min_block" >&2
	return 1
}

block_hash_by_number() {
	local port="$1"
	local number="$2"
	rpc "$port" chain_getBlockHash "[$number]" | json_field "data['result']"
}

# Scan heights $2..$3 on node $1; echoes one line per header:
#   <number> <author_hex|-> <has_pow_seal> <has_pow_preruntime>
# Single python process over HTTP — no per-block curl/python spawning (that
# churn starves mining threads and can drop peers on small boxes).
scan_pow_headers() {
	ghost_scale scan "$1" "$2" "$3"
}

# Wait until `system_health.peers` equals $2 on port $1 (grace for transient
# disconnects under mining load).
wait_peers_eq() {
	local port="$1"
	local want="$2"
	local tries="${3:-90}"
	for _ in $(seq 1 "$tries"); do
		local peers
		peers="$(peer_count "$port" 2>/dev/null)" || peers=-1
		if (( peers == want )); then
			return 0
		fi
		sleep 1
	done
	echo "node on :$port has peers=$(peer_count "$port"), wanted $want" >&2
	return 1
}

# Free balance of an account (raw 32-byte AccountId32 hex, no 0x) via
# System::Account storage; optional block hash $3 (empty = best block).
# Prints an integer (planck).
account_free_balance() {
	local port="$1"
	local account_hex="$2"
	local at="${3:-}"
	local key value
	key="$(ghost_scale account_key "$account_hex")"
	value="$(storage_at "$port" "$key" "$at")"
	if [ "$value" = "None" ] || [ -z "$value" ]; then
		echo 0
		return 0
	fi
	ghost_scale account_free "$value"
}

# Runtime-api difficulty via state_call GhostPowApi_next_difficulty at the
# node's best block (or the block hash given as $2).
ghost_next_difficulty() {
	local port="$1"
	local at="${2:-}"
	local result
	if [ -n "$at" ]; then
		result="$(rpc "$port" state_call '["GhostPowApi_next_difficulty", "0x", "'"$at"'"]' | json_field "data['result']")"
	else
		result="$(rpc "$port" state_call '["GhostPowApi_next_difficulty", "0x"]' | json_field "data['result']")"
	fi
	ghost_scale u256le "$result"
}

# Storage value hex for key $2 on node $1; optional block hash $3 (empty =
# best block). Prints the hex value or "None".
storage_at() {
	local port="$1" key="$2" at="${3:-}"
	if [ -n "$at" ]; then
		rpc "$port" state_getStorage "[\"$key\", \"$at\"]" | json_field "data['result']"
	else
		rpc "$port" state_getStorage "[\"$key\"]" | json_field "data['result']"
	fi
}

# Pallet storage read of GhostConsensus::Difficulty at best block.
ghost_difficulty_storage() {
	local port="$1"
	local key
	key="$(ghost_scale storage_key GhostConsensus Difficulty)"
	ghost_scale u256le "$(storage_at "$port" "$key")"
}
