#!/usr/bin/env bash
# check-wasm-reproducible.sh — byte-exact evidence that the runtime wasm is
# deterministic: builds solochain-template-runtime twice from two fresh
# git-worktree copies of HEAD and compares sha256.
#
# A match means the toolchain pin (rust-toolchain.toml) plus this tree's
# build inputs fully determine the artifact. A mismatch means something
# path/time/environment-dependent leaks into the build — investigate before
# trusting release-hashes as proof (see docs/release-verification.md and the
# srtool P2 item in docs/production-plan.md).
#
# Usage:  bash scripts/check-wasm-reproducible.sh
# Env:    WORKDIR_BASE  parent dir for the two worktrees (default: mktemp)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
BASE="${WORKDIR_BASE:-$(mktemp -d "${TMPDIR:-/tmp}/ghost-repro-XXXXXX")}"
KEEP="${KEEP:-}"

cleanup() {
    [ -n "$KEEP" ] && { echo "kept $BASE"; return; }
    git -C "$ROOT" worktree remove --force "$BASE/a" 2>/dev/null || true
    git -C "$ROOT" worktree remove --force "$BASE/b" 2>/dev/null || true
    rm -rf "$BASE"
}
trap cleanup EXIT

mkdir -p "$BASE"
git -C "$ROOT" worktree add --force "$BASE/a" HEAD >/dev/null
git -C "$ROOT" worktree add --force "$BASE/b" HEAD >/dev/null

build() {
    local dir="$1"
    echo "== building runtime wasm in $dir ==" >&2
    (cd "$dir" && rtk cargo build --release -p solochain-template-runtime >/dev/null 2>&1)
    sha256sum "$dir/target/release/wbuild/solochain-template-runtime/solochain_template_runtime.wasm" | awk '{print $1}'
}

H1=$(build "$BASE/a")
H2=$(build "$BASE/b")
echo "a: $H1"
echo "b: $H2"

if [ "$H1" = "$H2" ]; then
    echo "== REPRODUCIBLE: identical runtime wasm across independent trees =="
else
    echo "== MISMATCH — investigate before relying on release hashes ==" >&2
    exit 1
fi
