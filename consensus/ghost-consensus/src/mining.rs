//! Mining helpers.
//!
//! [`pow_value`] is the single definition of the Ghost PoW hash: both
//! `verify` (import path) and [`hash_meets`] / [`grind`] (mining path) use it,
//! so a seal produced by the grind loop is guaranteed to pass verification.
//!
//! The grind loop is pure and thread-friendly: `start_mining_worker` fills a
//! [`MiningMetadata`] for each new best block; N mining threads call [`grind`]
//! over disjoint nonce strides and submit the winning `GhostSeal` via
//! `MiningHandle::submit(seal.encode())`.

use codec::{Decode, Encode};
use sc_consensus_pow::MiningMetadata;
use sp_core::{crypto::AccountId32, U256};
use sp_crypto_hashing::blake2_256;
use sp_runtime::DigestItem;

use ghost_pow_primitives::{GhostSeal, POW_ENGINE_ID};

/// The Ghost PoW hash output: `blake2_256(blake2_256(input))` as a
/// **big-endian** `U256`, where `input = pre_hash ++ pre_digest ++
/// seal.encode()` (`docs/ghost-consensus-design.md` §3).
///
/// `pre_digest` is the raw payload of `DigestItem::PreRuntime(POW_ENGINE_ID, _)`
/// (the SCALE-encoded miner account), not the `DigestItem` itself.
pub fn pow_value(pre_hash: &[u8], pre_digest: &[u8], seal: &GhostSeal) -> U256 {
    let mut input = Vec::with_capacity(pre_hash.len() + pre_digest.len() + 40);
    input.extend_from_slice(pre_hash);
    input.extend_from_slice(pre_digest);
    input.extend_from_slice(&seal.encode());
    let inner = blake2_256(&input);
    let outer = blake2_256(&inner);
    U256::from_big_endian(&outer)
}

/// Whether a PoW hash output meets a work-factor difficulty.
///
/// `difficulty` is a **work factor** — larger means harder. The seal is valid
/// iff `value * difficulty <= U256::MAX`, equivalently `value <= MAX /
/// difficulty` (`docs/ghost-consensus-design.md` §3). `difficulty == 0` never
/// meets — a zero work factor would make every seal valid and break
/// heaviest-chain accounting.
pub fn pow_meets(value: U256, difficulty: U256) -> bool {
    match U256::MAX.checked_div(difficulty) {
        Some(limit) => value <= limit,
        None => false,
    }
}

/// Try a single nonce. Returns the constructed [`GhostSeal`] iff the PoW hash
/// meets `difficulty` under [`pow_meets`]. `pre_hash` must be exactly 32 bytes
/// — it is embedded into the seal so fork choice can tie-break on it.
pub fn hash_meets(
    pre_hash: &[u8],
    pre_digest: &[u8],
    nonce: u64,
    difficulty: U256,
) -> Option<GhostSeal> {
    let seal = GhostSeal {
        nonce,
        pre_hash: pre_hash.try_into().ok()?,
    };
    pow_meets(pow_value(pre_hash, pre_digest, &seal), difficulty).then_some(seal)
}

/// Grind `rounds` nonces starting at `nonce_start`, stepping by `nonce_step`
/// (use `step = num_threads` with per-thread `start` to partition the nonce
/// space across `--mining-threads`). Returns `None` if no nonce in the range
/// meets `difficulty`, or if `metadata` carries no `pre_runtime` — the seal
/// would fail `verify` anyway since the miner account is hash-bound.
pub fn grind<H: AsRef<[u8]>>(
    metadata: &MiningMetadata<H, U256>,
    nonce_start: u64,
    nonce_step: u64,
    rounds: u64,
) -> Option<GhostSeal> {
    let pre_digest = metadata.pre_runtime.as_deref()?;
    let step = nonce_step.max(1);
    let mut nonce = nonce_start;
    for _ in 0..rounds {
        if let Some(seal) = hash_meets(
            metadata.pre_hash.as_ref(),
            pre_digest,
            nonce,
            metadata.difficulty,
        ) {
            return Some(seal);
        }
        nonce = nonce.wrapping_add(step);
    }
    None
}

/// Bytes to pass as `start_mining_worker`'s `pre_runtime` argument: it stamps
/// `DigestItem::PreRuntime(POW_ENGINE_ID, these_bytes)` into every proposal,
/// binding `author` into the seal hash input.
pub fn miner_pre_runtime(author: &AccountId32) -> Vec<u8> {
    author.encode()
}

/// The full pre-runtime digest item a miner header carries (used by tests and
/// by callers constructing digests directly).
pub fn miner_digest_item(author: &AccountId32) -> DigestItem {
    DigestItem::PreRuntime(POW_ENGINE_ID, miner_pre_runtime(author))
}

/// Decode the miner `AccountId32` from a `PreRuntime(POW_ENGINE_ID, _)` payload.
/// Same decode the pallet applies for attribution; trailing bytes are ignored
/// (they remain covered by the seal hash regardless).
pub fn author_from_pre_digest(bytes: &[u8]) -> Result<AccountId32, codec::Error> {
    AccountId32::decode(&mut &bytes[..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify_seal;
    use proptest::prelude::*;

    #[test]
    fn seal_roundtrip() {
        let seal = GhostSeal {
            nonce: 0xdead_beef_cafe_f00d,
            pre_hash: [0xabu8; 32],
        };
        assert_eq!(GhostSeal::decode(&mut &seal.encode()[..]).unwrap(), seal);
        // Wire format: u64 nonce (8 bytes LE) ++ 32-byte pre_hash.
        let mut expected = 0xdead_beef_cafe_f00du64.to_le_bytes().to_vec();
        expected.extend_from_slice(&[0xabu8; 32]);
        assert_eq!(seal.encode(), expected);
    }

    #[test]
    fn miner_pre_runtime_roundtrip() {
        let author = AccountId32::new([5u8; 32]);
        let bytes = miner_pre_runtime(&author);
        assert_eq!(bytes, author.encode());
        assert_eq!(author_from_pre_digest(&bytes).unwrap(), author);
        assert!(author_from_pre_digest(&[0u8; 10]).is_err());
        match miner_digest_item(&author) {
            DigestItem::PreRuntime(id, payload) => {
                assert_eq!(id, POW_ENGINE_ID);
                assert_eq!(author_from_pre_digest(&payload).unwrap(), author);
            }
            _ => panic!("expected PreRuntime digest item"),
        }
    }

    #[test]
    fn pow_meets_work_factor_semantics() {
        // Work factor 1: every hash meets it (limit = MAX).
        assert!(pow_meets(U256::MAX, U256::one()));
        // Work factor 0: degenerate — must never accept (would trivialise PoW).
        assert!(!pow_meets(U256::zero(), U256::zero()));
        assert!(!pow_meets(U256::MAX, U256::zero()));
        // Boundary: value <= MAX / difficulty.
        let value = U256::from(1_000_000u64);
        let exact = U256::MAX / value;
        assert!(pow_meets(value, exact));
        assert!(!pow_meets(value, exact + 1));
        // value * difficulty must not overflow-accept: MAX work factor only
        // admits values <= 1.
        assert!(pow_meets(U256::one(), U256::MAX));
        assert!(!pow_meets(U256::from(2u64), U256::MAX));
    }

    #[test]
    fn hash_meets_known_directions() {
        let author = AccountId32::new([7u8; 32]);
        let pre_digest = miner_pre_runtime(&author);
        let pre_hash = [1u8; 32];

        let seal0 = GhostSeal { nonce: 0, pre_hash };
        // Work factor 1: every nonce meets.
        assert_eq!(
            hash_meets(&pre_hash, &pre_digest, 0, U256::one()),
            Some(seal0)
        );
        // Work factor 0: nothing meets.
        assert_eq!(hash_meets(&pre_hash, &pre_digest, 0, U256::zero()), None);
        // Non-32-byte pre_hash: no seal can be constructed.
        assert_eq!(hash_meets(&[1u8; 8], &pre_digest, 0, U256::one()), None);
        // Exact boundary, both directions.
        let seal7 = GhostSeal { nonce: 7, pre_hash };
        let value = pow_value(&pre_hash, &pre_digest, &seal7);
        if !value.is_zero() {
            let exact = U256::MAX / value;
            assert_eq!(hash_meets(&pre_hash, &pre_digest, 7, exact), Some(seal7));
            assert_eq!(hash_meets(&pre_hash, &pre_digest, 7, exact + 1), None);
        }
    }

    #[test]
    fn grind_finds_and_bounds() {
        let metadata = MiningMetadata {
            best_hash: [0u8; 32],
            pre_hash: [2u8; 32],
            pre_runtime: Some(miner_pre_runtime(&AccountId32::new([1u8; 32]))),
            difficulty: U256::one(),
        };
        // Work factor 1: first nonce wins.
        let seal0 = GhostSeal {
            nonce: 0,
            pre_hash: [2u8; 32],
        };
        let seal1 = GhostSeal {
            nonce: 1,
            pre_hash: [2u8; 32],
        };
        assert_eq!(grind(&metadata, 0, 1, 8), Some(seal0));
        // Work factor 0: rounds are bounded and nothing is found.
        let mut impossible = metadata.clone();
        impossible.difficulty = U256::zero();
        assert_eq!(grind(&impossible, 0, 1, 1024), None);
        // Missing pre_runtime: cannot produce a verifiable seal.
        let mut no_miner = metadata.clone();
        no_miner.pre_runtime = None;
        assert_eq!(grind(&no_miner, 0, 1, 1024), None);
        // Strides partition the nonce space deterministically.
        assert_eq!(grind(&metadata, 1, 4, 4), Some(seal1));
    }

    proptest! {
        #[test]
        fn mining_and_verify_agree_on_accept_and_reject(
            pre_hash in any::<[u8; 32]>(),
            author in any::<[u8; 32]>(),
            nonce in any::<u64>(),
            diff_be in any::<[u8; 32]>(),
        ) {
            // `verify` (import) and `hash_meets` (mining) must reach the same
            // verdict for every seal/difficulty pair — this pins the shared
            // contract both paths are supposed to implement.
            let pre_digest = miner_pre_runtime(&AccountId32::new(author));
            let difficulty = U256::from_big_endian(&diff_be);
            let seal = GhostSeal {
                nonce,
                pre_hash,
            };
            let expected = pow_meets(pow_value(&pre_hash, &pre_digest, &seal), difficulty);
            prop_assert_eq!(
                verify_seal(&pre_hash, Some(&pre_digest), &seal.encode(), difficulty),
                expected
            );
        }

        #[test]
        fn pow_value_deterministic_and_meets_is_monotone(
            pre_hash in any::<[u8; 32]>(),
            pre_digest in proptest::collection::vec(any::<u8>(), 0..64),
            nonce in any::<u64>(),
            d1 in any::<[u8; 32]>(),
            d2 in any::<[u8; 32]>(),
        ) {
            let seal = GhostSeal {
                nonce,
                pre_hash: [0u8; 32],
            };
            let v1 = pow_value(&pre_hash, &pre_digest, &seal);
            let v2 = pow_value(&pre_hash, &pre_digest, &seal);
            prop_assert_eq!(v1, v2, "hash must be deterministic");

            let d1 = U256::from_big_endian(&d1);
            let d2 = U256::from_big_endian(&d2);
            let (lo, hi) = if d1 <= d2 { (d1, d2) } else { (d2, d1) };
            // Work factor: meeting the *harder* difficulty implies meeting
            // the easier one (limit = MAX/d shrinks as d grows).
            if pow_meets(v1, hi) {
                prop_assert!(pow_meets(v1, lo), "meets(harder) must imply meets(easier)");
            }
        }
    }
}
