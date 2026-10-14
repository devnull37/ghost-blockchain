#!/usr/bin/env bash
# srtool-build.sh — build the Ghost runtime wasm inside the deterministic
# paritytech/srtool environment and print its BLAKE2-256 proposal hash.
#
# This is the cross-machine reproducibility gate: anyone running this on
# the same git commit MUST get the same BLAKE2_256 — srtool pins the
# toolchain, linker flags and build env so the output depends only on
# source. Compare it against the hash published in the release notes
# (docs/release-verification.md) or against a second machine's run.
#
# Usage:
#   scripts/srtool-build.sh [srtool-image-tag]
#
# Requires: docker. Default image tag: 1.88.0 (must match rust-toolchain.toml).
set -euo pipefail

TAG="${1:-1.88.0}"
IMAGE="paritytech/srtool:${TAG}"

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# The container's builder user writes into runtime/target — make it
# writable before the build (bind mounts keep host ownership).
mkdir -p runtime/target
chmod -R a+w runtime/target

docker run --rm \
	-e PACKAGE=solochain-template-runtime \
	-e RUNTIME_DIR=runtime \
	-v "$PWD":/build \
	"$IMAGE" build
