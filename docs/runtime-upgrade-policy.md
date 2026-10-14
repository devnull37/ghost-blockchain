# Runtime Versioning and Upgrade Policy

## Version constants (`runtime/src/lib.rs` `VERSION`)

| Field | Rule | Current |
|---|---|---|
| `spec_version` | **Bump on every change to the runtime's logic or metadata** — pallet set, call order, storage layout, Config types, consensus constants. This is what `CheckSpecVersion` and native-vs-Wasm matching key on. | 103 |
| `impl_version` | Bump for implementation-only changes that leave the spec identical — bug fixes in behavior-preserving code, perf changes with no semantics delta. When in doubt, bump `spec_version` instead. | 1 |
| `transaction_version` | Bump when an extrinsic's *encoding* changes in a way that breaks previously-signed payloads (signature payload layout). Rare. | 1 |
| `authoring_version` | Bump when the block-authoring interface changes (seal format, digest layout) such that old authors cannot produce valid blocks. | 1 |

**Policy:** any PR touching `runtime/`, `pallets/`, `primitives/`, or the
consensus seal/verifier must bump `spec_version` unless it can show the
compiled Wasm is bit-identical. CI does not yet enforce this — reviewers
check the diff manually until an automated metadata-diff gate exists.

## Storage migration convention

- Every pallet that ships `StorageVersion`-gated state declares its version
  via `#[pallet::storage_version]` and migrates with an
  `OnRuntimeUpgrade` hook guarded by `on_chain_storage_version()`.
- Migrations live in the pallet's `migrations` module, not in the runtime.
- The v1→v2 Ghost pallet migration (already landed) is the reference: it
  wiped all 12 legacy storages inside `#[pallet::storage_version]` gating
  and is covered by a `try-runtime`/unit test.
- Every `spec_version` bump that mutates existing storage **must** ship the
  migration in the same commit as the layout change — no "we'll migrate
  later".

## Runtime-upgrade procedure (testnet)

1. Bump `spec_version` (+ `impl_version` reset semantics as above) and land
   the code + any migrations.
2. Build the new `runtime.compact.compressed.wasm` from the release binary
   (`ghost-node` embeds it; extract via `build-spec` or the build output).
3. Submit `system.set_code` — currently via sudo (testnet-only; sudo is not
   a production-preset feature).

   **polkadot-js encoding hazard (observed in the upgrade drill):** pass the
   wasm as a hex string — `api.tx.system.setCode(u8aToHex(code))` — never a
   raw `Buffer`/`Uint8Array`. The installed `@polkadot/types` treats a u8a
   `Bytes` arg as already-SCALE-encoded, so a wasm blob (magic starts
   `0x00`) encodes as an EMPTY vec: the chain stores a zero-length `:code`
   and wedges permanently (`UnexpectedEof` on every executor call; a node
   cannot even boot on that db — rollback is impossible because the
   runtime needed to submit the fix is the corrupt one). Verify the
   submitted extrinsic length matches the wasm size before broadcasting.
4. Verify post-upgrade: `state_getRuntimeVersion` shows the new
   `spec_version`; blocks keep being authored/finalized; run
   `e2e-ghost.sh` against a node that *upgraded* rather than restarted
   (upgrade drill — required once before any public launch claim).
5. If the chain halts: the PoW path has no slot to lose, so nodes simply
   refuse bad imports — rollback is re-running `set_code` with the prior
   Wasm, not restarting history.
