// Craft a cryptographically valid GRANDPA prevote-equivocation proof with
// Alice's dev `gran` key and submit it via the signed `report_equivocation`
// call. (The unsigned variant only accepts TransactionSource::Local — i.e.
// offchain-worker submissions — so RPC-submitted reports must be signed.)
//
// Two identical-view validator processes on localhost sign identical votes,
// so a live double-sign cannot be forced deterministically. This produces
// the exact same on-chain evidence a real double-signer leaks: two
// Alice-signed prevotes for the same (round, setId) on different targets.
// The pallet cannot distinguish it — signature verification is real.
//
// Signed payload = SCALE(Message::Prevote, round, setId)
//                = 0x00 || target_hash(32B) || target_number(u32le)
//                    || round(u64le) || set_id(u64le)
//
// Usage: node craft-equivocation.js ws://127.0.0.1:<rpc-port>
// Prints "SUBMITTED <hash>" on inclusion or exits non-zero with the
// dispatch error. Slash landing is asserted by the caller.

const { ApiPromise, WsProvider, Keyring } = require("@polkadot/api");

const RPC = process.argv[2] || "ws://127.0.0.1:9944";
const ROUND = BigInt(0xdeadbee5); // unique slot, off the real round sequence

function prevotePayload(targetHash, targetNumber, round, setId) {
	const buf = Buffer.alloc(1 + 32 + 4 + 8 + 8);
	buf.writeUInt8(0, 0); // Message::Prevote variant index
	const h = Buffer.from(targetHash.replace("0x", ""), "hex");
	h.copy(buf, 1);
	buf.writeUInt32LE(targetNumber >>> 0, 33);
	buf.writeBigUInt64LE(BigInt(round), 37);
	buf.writeBigUInt64LE(BigInt(setId), 45);
	return buf;
}

async function main() {
	const api = await ApiPromise.create({ provider: new WsProvider(RPC) });
	const keyring = new Keyring({ type: "ed25519" });
	const aliceGran = keyring.addFromUri("//Alice");
	const reporter = new Keyring({ type: "sr25519" }).addFromUri("//Alice");

	const setId = (await api.query.grandpa.currentSetId()).toBigInt();
	console.log(`current set id: ${setId}`);

	// Two different prevote targets — a real fork view. Any divergence counts.
	const n1 = 2, n2 = 3;
	const h1 = await api.rpc.chain.getBlockHash(n1);
	const h2 = await api.rpc.chain.getBlockHash(n2);

	const sig1 = aliceGran.sign(prevotePayload(h1.toHex(), n1, ROUND, setId));
	const sig2 = aliceGran.sign(prevotePayload(h2.toHex(), n2, ROUND, setId));

	const keyOwnerProof = await api.call.grandpaApi.generateKeyOwnershipProof(
		setId,
		aliceGran.publicKey,
	);
	if (keyOwnerProof.isNone) {
		console.error("generate_key_ownership_proof returned None");
		process.exit(2);
	}

	const equivocationProof = {
		setId: setId.toString(),
		equivocation: {
			Prevote: {
				roundNumber: ROUND.toString(),
				identity: aliceGran.publicKey,
				first: [{ targetHash: h1, targetNumber: n1 }, sig1],
				second: [{ targetHash: h2, targetNumber: n2 }, sig2],
			},
		},
	};

	const tx = api.tx.grandpa.reportEquivocation(
		equivocationProof,
		keyOwnerProof.unwrap(),
	);

	const hash = await new Promise((resolve, reject) => {
		let done = false;
		tx.signAndSend(reporter, ({ status, dispatchError }) => {
			if (status.isInBlock && !done) {
				done = true;
				if (dispatchError) {
					let msg = dispatchError.toString();
					if (dispatchError.isModule) {
						const e = api.registry.findMetaError(dispatchError.asModule);
						msg = `${e.section}.${e.name}: ${e.docs.join(" ")}`;
					}
					reject(new Error(`extrinsic failed: ${msg}`));
				} else {
					resolve(status.asInBlock);
				}
			}
		}).catch(reject);
		setTimeout(() => !done && reject(new Error("submission timeout")), 120000);
	});

	console.log(`SUBMITTED ${hash.toHex()}`);
	await api.disconnect();
}

main().catch((e) => {
	console.error(e.message || e);
	process.exit(1);
});
