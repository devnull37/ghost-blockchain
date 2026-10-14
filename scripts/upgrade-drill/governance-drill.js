// Governance drill driver.
// Usage: node governance-drill.js <ws-url>
//
// Exercises the full governance path on a live dev chain end-to-end:
//   preimage.note_preimage -> council.propose/vote/close (external proposal)
//   -> council fast_track -> token-holder referendum vote -> enactment
//   (democracy dispatches as ROOT via scheduler).
//
// Target call: balances.force_set_balance — a root-only call whose effect is
// trivially observable, proving council+democracy+scheduler+preimage all
// dispatched correctly. Asserts finality keeps advancing throughout.
const { ApiPromise, WsProvider, Keyring } = require("@polkadot/api");
const { blake2AsHex, cryptoWaitReady } = require("@polkadot/util-crypto");

const WS = process.argv[2];
const TARGET = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"; // //Bob ss58
const SET_BALANCE = "12345000000000000"; // ~12.345 GHOST
const VOTING_PERIOD = 15; // blocks (~75s at 5s)
const ENACT_DELAY = 5;

function log(msg) {
  console.log(`[gov-drill] ${msg}`);
}
function die(msg) {
  console.error(`[gov-drill] FAIL: ${msg}`);
  process.exit(1);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function signAndSend(api, signer, call, what) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error(`${what} timeout`)), 180_000);
    call
      .signAndSend(signer, ({ status, dispatchError, events }) => {
        if (!status.isInBlock) return;
        clearTimeout(timeout);
        if (dispatchError) {
          if (dispatchError.isModule) {
            const dec = api.registry.findMetaError(dispatchError.asModule);
            return reject(new Error(`${what}: ${dec.section}.${dec.name} ${dec.docs.join(" ")}`));
          }
          return reject(new Error(`${what}: ${dispatchError.toString()}`));
        }
        resolve(events.map((e) => {
          let data = "";
          try { data = ` ${JSON.stringify(e.event.data.toHuman())}`; } catch (_) {}
          return `${e.event.section}.${e.event.method}${data}`;
        }));
      })
      .catch(reject);
  });
}

async function finalizedHead(api) {
  const h = await api.rpc.chain.getFinalizedHead();
  const b = await api.rpc.chain.getBlock(h);
  return b.block.header.number.toNumber();
}

async function main() {
  await cryptoWaitReady();
  const api = await ApiPromise.create({ provider: new WsProvider(WS) });
  await api.isReady;
  const keyring = new Keyring({ type: "sr25519" });
  const alice = keyring.addFromUri("//Alice");
  const bob = keyring.addFromUri("//Bob");

  // -- 1. Genesis council seeded ------------------------------------------
  const members = (await api.query.council.members()).map((a) => a.toString());
  log(`council members: ${JSON.stringify(members)}`);
  if (!members.includes(alice.address)) die("genesis council missing Alice");
  const threshold = Math.max(1, Math.ceil(members.length / 2));
  log(`council threshold: ${threshold}`);

  const fin0 = await finalizedHead(api);

  // -- 2. Note preimage for the root call ----------------------------------
  const inner = api.tx.balances.forceSetBalance(TARGET, SET_BALANCE);
  const encoded = inner.method.toHex();
  const propHash = blake2AsHex(inner.method.toU8a());
  log(`noting preimage for force_set_balance (${encoded.length / 2 - 1} bytes, hash ${propHash})`);
  await signAndSend(api, alice, api.tx.preimage.notePreimage(encoded), "note_preimage");
  const stored = await api.query.preimage.preimageFor([propHash, encoded.length / 2 - 1]);
  if (stored.isNone) die("preimage not stored");

  // -- 3. Council tables the external proposal -----------------------------
  // A motion whose threshold is met by the proposer's own vote dispatches
  // immediately (one-member councils always auto-execute); otherwise it sits
  // in proposals() for vote+close. Handle both.
  const extProposal = api.tx.democracy.externalProposeMajority({ Lookup: { hash: propHash, len: encoded.length / 2 - 1 } });
  let events = await signAndSend(api, alice,
    api.tx.council.propose(threshold, extProposal, extProposal.encodedLength),
    "council.propose(external)");
  log(`propose events: ${events.join(",")}`);
  let pending = await api.query.council.proposals();
  if (pending.length > 0) {
    const hash = pending[pending.length - 1].toHex();
    log(`council motion pending: ${hash} — voting`);
    await signAndSend(api, alice, api.tx.council.vote(hash, 0, true), "council.vote(external)");
    events = await signAndSend(
      api, alice,
      api.tx.council.close(hash, 0, { refTime: "10000000000", proofSize: "65536" }, extProposal.encodedLength),
      "council.close(external)"
    );
    log(`close events: ${events.join(",")}`);
  } else {
    log("motion auto-executed on propose (threshold met by proposer)");
  }
  const nextExt = await api.query.democracy.nextExternal();
  if (nextExt.isNone) die("democracy.nextExternal empty — council motion did not table");
  log(`external proposal tabled: ${JSON.stringify(nextExt.toHuman())}`);

  // -- 4. Council fast-tracks it to a referendum ---------------------------
  const fastTrack = api.tx.democracy.fastTrack(propHash, VOTING_PERIOD, ENACT_DELAY);
  events = await signAndSend(api, alice,
    api.tx.council.propose(threshold, fastTrack, fastTrack.encodedLength),
    "council.propose(fastTrack)");
  log(`propose events: ${events.join(",")}`);
  pending = await api.query.council.proposals();
  if (pending.length > 0) {
    const hash2 = pending[pending.length - 1].toHex();
    await signAndSend(api, alice, api.tx.council.vote(hash2, 1, true), "council.vote(fastTrack)");
    events = await signAndSend(
      api, alice,
      api.tx.council.close(hash2, 1, { refTime: "10000000000", proofSize: "65536" }, fastTrack.encodedLength),
      "council.close(fastTrack)"
    );
    log(`close events: ${events.join(",")}`);
  } else {
    log("fast-track motion auto-executed on propose");
  }
  const refCount = await api.query.democracy.referendumCount();
  if (refCount.toNumber() !== 1) die(`expected 1 referendum, got ${refCount}`);
  log(`referendum 0 live (voting_period=${VOTING_PERIOD}, enact_delay=${ENACT_DELAY})`);

  // -- 5. Token-holder vote -------------------------------------------------
  const aye = {
    Standard: {
      vote: { aye: true, conviction: { Locked1x: null } },
      balance: api.consts.democracy.minimumDeposit.toBigInt() * 2n,
    },
  };
  await signAndSend(api, alice, api.tx.democracy.vote(0, aye), "democracy.vote(Alice AYE)");
  await signAndSend(api, bob, api.tx.democracy.vote(0, aye), "democracy.vote(Bob AYE)");
  const votes = await api.query.democracy.votingOf(alice.address);
  if (votes.isNone) die("alice vote not recorded");
  log("AYE votes cast by Alice + Bob");

  // -- 6. Wait for enactment; assert world-state ---------------------------
  const deadline = Date.now() + (VOTING_PERIOD + ENACT_DELAY + 60) * 5_000;
  let passed = false;
  while (Date.now() < deadline) {
    const info = await api.query.democracy.referendumInfoOf(0);
    if (info.isSome && info.unwrap().isFinished) {
      const fin = info.unwrap().asFinished;
      if (!fin.approved.isTrue) die("referendum finished but NOT approved");
      passed = true;
      break;
    }
    await sleep(5_000);
  }
  if (!passed) die(`referendum still active after deadline`);

  // Enactment is a scheduled dispatch — poll the target balance.
  let enacted = false;
  const deadline2 = Date.now() + (ENACT_DELAY + 30) * 5_000;
  while (Date.now() < deadline2) {
    const acct = await api.query.system.account(TARGET);
    if (acct.data.free.toString() === SET_BALANCE) {
      enacted = true;
      break;
    }
    await sleep(5_000);
  }
  if (!enacted) die("referendum passed but force_set_balance never enacted");
  log(`enacted: ${TARGET} free balance == ${SET_BALANCE}`);

  // -- 7. Chain still healthy ------------------------------------------------
  const fin1 = await finalizedHead(api);
  if (fin1 <= fin0) die("finality stalled during drill");
  log(`finality advanced ${fin0} -> ${fin1}`);

  log("GOVERNANCE DRILL PASSED");
  await api.disconnect();
}

main().catch((e) => {
  console.error(`[gov-drill] FAIL: ${e.message}`);
  process.exit(1);
});
