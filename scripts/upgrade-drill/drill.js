// Runtime-upgrade drill driver.
// Usage: node drill.js <ws-url> <new-wasm-path> <expected-spec-version>
//
// Submits sudo(system.set_code(wasm)) as //Alice, waits for inclusion,
// then asserts: spec_version bumped, blocks keep producing + finalizing.
const { ApiPromise, WsProvider, Keyring } = require("@polkadot/api");
const fs = require("fs");

const WS = process.argv[2];
const WASM_PATH = process.argv[3];
const EXPECTED = parseInt(process.argv[4], 10);

function log(msg) {
  console.log(`[drill] ${msg}`);
}

async function main() {
  const api = await ApiPromise.create({ provider: new WsProvider(WS) });
  await api.isReady;

  const before = api.runtimeVersion.specVersion.toNumber();
  log(`runtimeVersion.specVersion before: ${before}`);
  if (before === EXPECTED) {
    log("spec already at target — nothing to do (stale wasm?)");
    process.exit(2);
  }

  const code = fs.readFileSync(WASM_PATH);
  log(`wasm: ${WASM_PATH} (${code.length} bytes)`);

  const keyring = new Keyring({ type: "sr25519" });
  const alice = keyring.addFromUri("//Alice");

  const inner = api.tx.system.setCode(code);
  const call = api.tx.sudo.sudo(inner);

  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error("set_code inclusion timeout")), 180_000);
    let done = false;
    call.signAndSend(alice, ({ status, dispatchError }) => {
      if (done) return;
      if (dispatchError) {
        done = true;
        clearTimeout(timeout);
        reject(new Error(`dispatch: ${dispatchError.toString()}`));
        return;
      }
      if (status.isInBlock) {
        done = true;
        clearTimeout(timeout);
        log(`set_code included in ${status.asInBlock.toHex()}`);
        resolve();
      }
    }).catch(reject);
  });

  // New wasm executes on the NEXT block's runtime update. Poll the LIVE
  // version via RPC — api.runtimeVersion is captured once at init and
  // never refreshes.
  const deadline = Date.now() + 120_000;
  let after = before;
  while (Date.now() < deadline) {
    const rv = await api.rpc.state.getRuntimeVersion();
    after = rv.specVersion.toNumber();
    if (after === EXPECTED) break;
    await new Promise((r) => setTimeout(r, 3000));
  }
  if (after !== EXPECTED) {
    throw new Error(`spec_version still ${after}, expected ${EXPECTED}`);
  }
  log(`runtimeVersion.specVersion after: ${after} (upgrade applied)`);

  // Blocks must keep producing AND finalizing on the new runtime.
  const headBefore = (await api.rpc.chain.getHeader()).number.toNumber();
  const finBefore = (await api.rpc.chain.getFinalizedHead());
  const finNumBefore = (
    await api.rpc.chain.getHeader(finBefore)
  ).number.toNumber();
  await new Promise((r) => setTimeout(r, 20_000));
  const headAfter = (await api.rpc.chain.getHeader()).number.toNumber();
  const finNumAfter = (
    await api.rpc.chain.getHeader(await api.rpc.chain.getFinalizedHead())
  ).number.toNumber();
  if (headAfter <= headBefore) throw new Error("best head stalled post-upgrade");
  if (finNumAfter <= finNumBefore)
    throw new Error("finality stalled post-upgrade");
  log(`post-upgrade liveness: best ${headBefore}->${headAfter}, finalized ${finNumBefore}->${finNumAfter}`);

  // Pallet storage version sanity: ghostConsensus must read v2 (proves the
  // wired on_runtime_upgrade hook ran without corrupting the stamp).
  const svKey =
    api.query.ghostConsensus && api.query.ghostConsensus
      ? "0x" +
        Buffer.from(
          require("@polkadot/util-crypto").xxhashAsU8a("GhostConsensus", 128)
        ).toString("hex") +
        Buffer.from(
          require("@polkadot/util-crypto").xxhashAsU8a(":__STORAGE_VERSION__", 128)
        ).toString("hex")
      : null;
  if (svKey) {
    const raw = await api.rpc.state.getStorage(svKey);
    if (raw.isSome) {
      const v = raw.unwrap()[0];
      log(`ghostConsensus on-chain storage version: ${v}`);
      if (v !== 2) throw new Error(`unexpected storage version ${v}`);
    }
  }

  await api.disconnect();
  log("PASS");
}

main().catch((e) => {
  console.error(`[drill] FAIL: ${e.message}`);
  process.exit(1);
});
