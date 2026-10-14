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
wasm32-unknown-unknown). Note the honest limit: rustc embeds the build
directory path in wasm inputs, so the hash only reproduces when you build
at the *same absolute path* the release CI used — treat this as a
tamper-evident integrity check on a known environment, not portable
reproducible-build proof. `scripts/check-wasm-reproducible.sh` automates an
in-path double build for byte-exact evidence; full cross-machine
determinism requires srtool (tracked as a P2 item in
`docs/production-plan.md`).
