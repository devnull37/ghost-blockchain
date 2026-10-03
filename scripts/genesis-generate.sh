#!/usr/bin/env bash
# genesis-generate.sh — produce a Ghost testnet chainspec from a ceremony
# parameter file. Usage:
#
#   GENESIS_PARAMS=<params.json> BIN=<ghost-node> \
#     rtk scripts/genesis-generate.sh <output-dir>
#
# <params.json> is ceremony input (see
# chainspecs/ghost-testnet-params.example.json):
#   {
#     "name": "Ghost Testnet 1",
#     "id": "ghost-testnet-1",
#     "chainType": "Live",
#     "bootNodes": ["/dns4/seed1.example.com/tcp/30333/p2p/<peer>"],
#     "root": "<sudo AccountId32 hex or SS58 — set null for no sudo>",
#     "authorities": [
#       {
#         "account": "<AccountId32 hex/SS58>",
#         "grandpa": "<ed25519 pub hex>",
#         "imOnline": "<sr25519 pub hex>",
#         "stake":  <planck integer>
#       }
#     ],
#     "endowed": [ {"account": "<acct>", "balance": <planck>} ... ],
#     "difficulty": <integer, default 1000000>
#   }
#
# The script writes:
#   <out>/<id>.json          plain spec (keys embedded — ceremony output)
#   <out>/<id>-raw.json      raw spec (what nodes boot with)
#   <out>/SHA256SUMS.txt     checksums of both
#
# Committing the params file is a policy decision: it contains only
# *public* keys, but the ceremony doc says the params file is generated
# offline and its hash attested by participants. See
# docs/genesis-ceremony.md.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
OUT_DIR="${1:?usage: genesis-generate.sh <output-dir>}"
PARAMS="${GENESIS_PARAMS:?set GENESIS_PARAMS to the ceremony params json}"
BIN="${BIN:-./target/debug/ghost-node}"
# Raw runtime wasm embedded as genesis code — the wbuild artifact. Must be
# built from the same tree/spec_version the network will run.
GHOST_WASM="${GHOST_WASM:-./target/debug/wbuild/solochain-template-runtime/solochain_template_runtime.wasm}"
[ -f "$GHOST_WASM" ] || { echo "runtime wasm not found at $GHOST_WASM (build first)"; exit 1; }

mkdir -p "$OUT_DIR"

python3 - "$PARAMS" "$OUT_DIR" "$GHOST_WASM" <<'PYEOF'
import hashlib, json, sys

B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
SS58_FORMAT = 42  # generic Substrate; the spec's `properties.ss58Format` matches


def b58encode(b: bytes) -> str:
    n = int.from_bytes(b, "big")
    out = ""
    while n:
        n, r = divmod(n, 58)
        out = B58[r] + out
    # leading zero bytes -> '1'
    for byte in b:
        if byte == 0:
            out = "1" + out
        else:
            break
    return out or "1"


def ss58(pubkey_hex: str) -> str:
    """SS58-encode a 32-byte key with the canonical address type."""
    pub = bytes.fromhex(pubkey_hex.removeprefix("0x"))
    assert len(pub) == 32
    body = bytes([SS58_FORMAT]) + pub
    chk = hashlib.blake2b(b"SS58PRE" + body, digest_size=64).digest()[:2]
    return b58encode(body + chk)


params = json.load(open(sys.argv[1]))
out_dir = sys.argv[2]
wasm_code = "0x" + open(sys.argv[3], "rb").read().hex()

name = params.get("name", "Ghost Testnet")
cid = params["id"]
chain_type = params.get("chainType", "Live")
boot_nodes = params.get("bootNodes", [])
difficulty = params.get("difficulty", 1_000_000)

# SS58 input support: if an account string isn't 0x-hex, leave it to the
# operator to convert (SS58 decode needs the address type); ceremony docs
# produce hex. Validate early — a bad key in genesis is a dead chain.
def acct(a, field):
    """Validate + render as SS58 — the genesis patch codec is base58."""
    if not (isinstance(a, str) and len(a) == 64 and all(c in "0123456789abcdefABCDEF" for c in a)):
        raise SystemExit(
            f"{field}: expected 64-hex public key (SS58 conversion happens "
            f"here — see docs/genesis-ceremony.md), got: {a!r}"
        )
    return ss58(a.lower())

authorities = []
stakers = []
initial_validators = []
for i, a in enumerate(params["authorities"]):
    account = acct(a["account"], f"authorities[{i}].account")
    grandpa = acct(a["grandpa"], f"authorities[{i}].grandpa")
    im_online = acct(a["imOnline"], f"authorities[{i}].imOnline")
    stake = int(a["stake"])
    if stake <= 0:
        raise SystemExit(f"authorities[{i}].stake must be > 0")
    authorities.append((account, account, {"grandpa": grandpa, "im_online": im_online}))
    stakers.append([account, stake])
    initial_validators.append(account)

balances = [[acct(e["account"], "endowed.account"), int(e["balance"])]
            for e in params.get("endowed", [])]

genesis = {
    "balances": {"balances": balances},
    # Session keys seed grandpa/im-online genesis authorities — the two
    # pallets' own genesis sections panic if also present.
    "session": {"keys": authorities},
    "ghostConsensus": {
        # U256 genesis serde is a hex string, not an integer.
        "difficulty": hex(difficulty),
        "stakers": stakers,
        "initialValidators": initial_validators,
    },
}
root = params.get("root")
if root is not None:
    genesis["sudo"] = {"key": acct(root, "root")}

spec = {
    "name": name,
    "id": cid,
    "chainType": chain_type,
    "bootNodes": boot_nodes,
    "telemetryEndpoints": None,
    "protocolId": "ghost",
    "properties": {
        "tokenDecimals": 12,
        "tokenSymbol": "GHOST",
        "ss58Format": 42,
    },
    "genesis": {"runtimeGenesis": {"code": wasm_code, "patch": genesis}},
}

path = f"{out_dir}/{cid}.json"
with open(path, "w") as f:
    json.dump(spec, f, indent=2)
print(path)
PYEOF

SPEC="${OUT_DIR}/$(python3 -c "import json;print(json.load(open('$PARAMS'))['id'])").json"
RAW="${SPEC%.json}-raw.json"

"$BIN" build-spec --chain "$SPEC" --raw > "$RAW"

(cd "$OUT_DIR" && sha256sum "$(basename "$SPEC")" "$(basename "$RAW")" > SHA256SUMS.txt)
echo "== genesis spec generated =="
cat "${OUT_DIR}/SHA256SUMS.txt"
