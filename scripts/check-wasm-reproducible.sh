#!/usr/bin/env bash
# check-wasm-reproducible.sh — evidence on whether the runtime wasm is
# deterministic: builds solochain-template-runtime twice from a fresh
# git-worktree copy of HEAD **at the same absolute path** and compares
# sha256.
#
# Why same-path: rustc embeds the workspace path in the build inputs, so
# comparing builds from two different directories measures path-leak, not
# nondeterminism. Same-path isolates real nondeterminism (timestamps,
# hash-map ordering, parallelism artifacts): a MISMATCH here means the
# build is not deterministic even in place.
#
# A MATCH does NOT prove users can reproduce release hashes: bit-exact
# cross-machine reproduction needs the pinned deterministic build
# environment (the srtool P2 item in docs/production-plan.md). Until that
# lands, treat docs/release-verification.md's rebuild-compare step as
# "deterministic on a fixed machine/toolchain", not portable proof.
#
# Usage:  bash scripts/check-wasm-reproducible.sh
# Env:    WORKDIR_BASE  parent dir for the worktree (default: mktemp)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
BASE="${WORKDIR_BASE:-$(mktemp -d "${TMPDIR:-/tmp}/ghost-repro-XXXXXX")}"
KEEP="${KEEP:-}"

cleanup() {
    [ -n "$KEEP" ] && { echo "kept $BASE"; return; }
    git -C "$ROOT" worktree remove --force "$BASE/tree" 2>/dev/null || true
    rm -rf "$BASE"
}
trap cleanup EXIT

mkdir -p "$BASE"
git -C "$ROOT" worktree add --force "$BASE/tree" HEAD >/dev/null

build() {
    (cd "$BASE/tree" && rtk cargo build --release -p solochain-template-runtime >/dev/null 2>&1)
    sha256sum "$BASE/tree/target/release/wbuild/solochain-template-runtime/solochain_template_runtime.wasm" | awk '{print $1}'
}

H1=$(build)
rm -rf "$BASE/tree/target/release/wbuild"
H2=$(build)

echo "build 1: $H1"
echo "build 2: $H2"

if [ "$H1" = "$H2" ]; then
    echo "== IN-PATH REPRODUCIBLE: identical runtime wasm, same tree/path =="
else
    echo "== MISMATCH — wasm build is nondeterministic in place ==" >&2
    exit 1
fi
