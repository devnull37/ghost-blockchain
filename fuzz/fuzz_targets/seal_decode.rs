#![no_main]

//! Fuzz `GhostSeal` SCALE decoding — the parser every node runs on the
//! `DigestItem::Seal(POW_ENGINE_ID, ..)` payload of every imported header.
//! Any panic, hang, or acceptance of a non-canonical encoding here is a
//! remote-reachable consensus bug.

use ghost_pow_primitives::GhostSeal;
use libfuzzer_sys::fuzz_target;
use parity_scale_codec::{Decode, DecodeAll, Encode};

const SEAL_LEN: usize = 40; // u64 LE nonce + 32-byte pre_hash

fuzz_target!(|data: &[u8]| {
    // Strict decode: accepts iff `data` is exactly the canonical encoding,
    // and whatever it accepts must re-encode byte-identically.
    match GhostSeal::decode_all(&mut &data[..]) {
        Ok(seal) => {
            assert_eq!(data.len(), SEAL_LEN);
            assert_eq!(seal.encode().as_slice(), data);
        }
        Err(_) => assert_ne!(
            data.len(),
            SEAL_LEN,
            "every {SEAL_LEN}-byte input is a valid (nonce, pre_hash) pair"
        ),
    }

    // Streaming decode: must never panic, must succeed exactly when a full
    // seal prefix exists, and must agree with the strict decoder on it.
    match GhostSeal::decode(&mut &data[..]) {
        Ok(seal) => {
            assert!(data.len() >= SEAL_LEN);
            let strict = GhostSeal::decode_all(&mut &data[..SEAL_LEN]).unwrap();
            assert_eq!(seal, strict);
            // Decoding must be prefix-stable: feeding a longer buffer can
            // never change the seal that was read.
            let mut extended = data.to_vec();
            extended.extend_from_slice(&[0xaa; 8]);
            assert_eq!(GhostSeal::decode(&mut &extended[..]).unwrap(), seal);
        }
        Err(_) => assert!(data.len() < SEAL_LEN),
    }
});
