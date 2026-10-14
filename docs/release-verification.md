## Verifying this release

Every artifact ships with a SHA256 entry in `SHA256SUMS.txt` and an
independent Sigstore (cosign keyless) signature `*.sigstore.json`.

### 1. Checksums

```bash
sha256sum -c SHA256SUMS.txt
```

### 2. Signatures

Signatures were produced by the release workflow's GitHub OIDC identity.
Verify with:

```bash
cosign verify-blob \
  --bundle ghost-node.sigstore.json \
  --certificate-identity-regexp \
    'https://github.com/devnull37/ghost-blockchain/.github/workflows/release.yml@refs/tags/.*' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  ghost-node
```

Repeat for `ghost-runtime.wasm` and `SHA256SUMS.txt`. A valid result proves
the artifact was produced by the tagged `release.yml` run on this repo — it
cannot have been tampered with after the workflow ran.

### 3. Reproducing `ghost-runtime.wasm`

The runtime wasm is the consensus-critical artifact. Rebuild it from the tag
and compare hashes:

```bash
git checkout <tag>
rtk cargo build --release -p solochain-template-runtime
sha256sum target/release/wbuild/solochain-template-runtime/solochain_template_runtime.wasm
```

The toolchain is pinned (`rust-toolchain.toml`: rustc 1.88.0 +
wasm32-unknown-unknown), so a clean-tree build should produce the same hash
as `ghost-runtime.wasm`. `scripts/check-wasm-reproducible.sh` automates the
two-clean-dir comparison if you want byte-exact evidence on your own
hardware — path-dependent metadata can still differ on some toolchains, so a
mismatch should be investigated rather than assumed malicious (see
`docs/production-plan.md` — full srtool determinism is tracked as a P2 item).
