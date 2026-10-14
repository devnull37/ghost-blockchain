#![no_main]

//! Fuzz `verify_seal` (the stateless check every import applies) and
//! `seal_beats` (the equal-total-difficulty fork tie-break). Both must be
//! total, deterministic, and free of panics on adversarial inputs — a node
//! that rejects a valid seal, accepts an invalid one, or orders ties
//! inconsistently splits the network.

use arbitrary::Arbitrary;
use ghost_consensus::{pow_meets, pow_value, seal_beats, seal_pre_hash, verify_seal, GhostSeal};
use libfuzzer_sys::fuzz_target;
use parity_scale_codec::DecodeAll;
use sp_core::U256;

#[derive(Arbitrary, Debug)]
struct Input {
    pre_hash: [u8; 32],
    has_pre_digest: bool,
    pre_digest: Vec<u8>,
    seal_bytes: Vec<u8>,
    difficulty: [u8; 32],
    own_seal: Vec<u8>,
    new_seal: Vec<u8>,
}

fuzz_target!(|input: Input| {
    let pre_digest = input.has_pre_digest.then_some(input.pre_digest.as_slice());
    let difficulty = U256::from_big_endian(&input.difficulty);

    // verify_seal: deterministic, panic-free, and — when it accepts — every
    // constituent check must independently hold.
    let a = verify_seal(&input.pre_hash, pre_digest, &input.seal_bytes, difficulty);
    let b = verify_seal(&input.pre_hash, pre_digest, &input.seal_bytes, difficulty);
    assert_eq!(a, b);
    if a {
        let pre_digest = pre_digest.expect("accepted seal implies pre_digest");
        assert!(ghost_consensus::author_from_pre_digest(pre_digest).is_ok());
        let seal = GhostSeal::decode_all(&mut &input.seal_bytes[..]).unwrap();
        assert_eq!(seal.pre_hash, input.pre_hash);
        assert!(pow_meets(
            pow_value(&input.pre_hash, pre_digest, &seal),
            difficulty
        ));
    }

    // seal_beats: the documented ordering — `new` wins iff both seals decode
    // and it carries the strictly smaller embedded pre_hash. Recomputed from
    // the public decode surface, not from the implementation.
    let own_ph = seal_pre_hash(&input.own_seal);
    let new_ph = seal_pre_hash(&input.new_seal);
    let expected = match (own_ph, new_ph) {
        (Some(own), Some(new)) => new < own,
        _ => false,
    };
    assert_eq!(seal_beats(&input.own_seal, &input.new_seal), expected);
    // Irreflexive and deterministic.
    assert!(!seal_beats(&input.own_seal, &input.own_seal));
    assert!(!seal_beats(&input.new_seal, &input.new_seal));
    assert_eq!(
        seal_beats(&input.own_seal, &input.new_seal),
        seal_beats(&input.own_seal, &input.new_seal)
    );
    // Total on distinct valid seals: exactly one direction wins.
    if let (Some(own), Some(new)) = (own_ph, new_ph) {
        if own != new {
            assert_ne!(
                seal_beats(&input.own_seal, &input.new_seal),
                seal_beats(&input.new_seal, &input.own_seal)
            );
        }
    }
});
