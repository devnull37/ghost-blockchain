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
wasm32-unknown-unknown). Note the honest limit for the *plain cargo* path:
rustc embeds the build directory path in wasm inputs, so that hash only
reproduces when you build at the *same absolute path* the release CI used
— treat it as a tamper-evident integrity check on a known environment.
`scripts/check-wasm-reproducible.sh` automates an in-path double build for
byte-exact evidence.

## Portable reproducible proof — srtool

For verification that does NOT depend on the build path or host, the
release pipeline also builds the runtime inside `paritytech/srtool`
(pinned toolchain/env container — the standard Substrate mechanism):

```bash
scripts/srtool-build.sh        # docker required; ~10 min cold, ~40 s warm
```

Two independent srtool runs of the same commit produce byte-identical
wasm (proven on spec-104: compact BLAKE2-256
`0xbeb659cce1c05d9281e8d9269f559fe291dca894863ad10f277a1e7788509f57`,
compressed `0x04ec4c1843f3d5927e88ccb5efd55150d038827098cc7256f5d913103605654c`).
srtool additionally prints the on-chain proposal hashes (`setCode` /
`authorizeUpgrade`) — those are the values a governance runtime-upgrade
proposal must commit to, so anyone can re-run `srtool-build.sh` at the
tag and verify the proposed hash before voting. The `srtool-wasm` job in
`.github/workflows/release.yml` double-builds and diffs the hash as part
of every release.
