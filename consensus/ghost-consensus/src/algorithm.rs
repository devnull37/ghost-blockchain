//! `GhostPowAlgorithm`: the `PowAlgorithm` implementation backing Ghost's
//! `PowBlockImport` / `PowVerifier`.

use std::{
    marker::PhantomData,
    sync::{Arc, Mutex},
};

use codec::DecodeAll;
use sc_client_api::backend::AuxStore;
use sc_consensus_pow::{Error, PowAlgorithm};
use sp_api::ProvideRuntimeApi;
use sp_consensus_pow::Seal;
use sp_core::{crypto::AccountId32, U256};
use sp_runtime::{
    generic::BlockId,
    traits::{Block as BlockT, Header as HeaderT},
    DigestItem,
};

use ghost_pow_primitives::{GhostPowApi, GhostSeal, POW_ENGINE_ID};

use crate::mining::{author_from_pre_digest, pow_meets, pow_value};

/// Shared one-entry memo: `difficulty()` is called twice per import with the
/// same parent hash.
type DifficultyMemo<B> = Arc<Mutex<Option<(<B as BlockT>::Hash, U256)>>>;

/// PoW algorithm for Ghost.
///
/// Per `docs/ghost-consensus-design.md` §3:
/// * `Difficulty = U256` — a **work factor** (larger = harder), so
///   `PowBlockImport`'s `total_difficulty` ordering is monotone in real work.
/// * Seal valid iff `blake2_256(blake2_256(pre_hash ++ pre_digest ++ seal))`
///   as a big-endian `U256`, multiplied by `difficulty`, does not exceed
///   `U256::MAX` — equivalently `value <= MAX / difficulty`. `difficulty == 0`
///   always fails.
/// * `pre_digest` is the payload of `DigestItem::PreRuntime(POW_ENGINE_ID, _)`
///   and must SCALE-decode to the miner's `AccountId32` — authorship is bound
///   into the hash input, so a block that cannot name a decodable miner is
///   rejected (§5: attribution must be unspoofable).
/// * `preliminary_verify` stays `Ok(None)` — full verification needs the
///   difficulty derived from parent state, which a stateless pre-check lacks.
/// * `break_tie` resolves equal-total-difficulty races on the seals' embedded
///   `pre_hash` (lowest wins) — identical to `HeaviestChain`'s leaf ordering,
///   so the import-time and selection-time fork choices cannot diverge on
///   ties (earliest-seen would let arrival order split the network).
pub struct GhostPowAlgorithm<B: BlockT, C> {
    client: Arc<C>,
    /// Genesis-configured difficulty, kept for call-site documentation and
    /// future cold-start paths — `difficulty()` itself always defers to the
    /// runtime API and rejects when it is unreachable.
    initial_difficulty: U256,
    /// One-entry memo for `difficulty()` — the import path calls it twice per
    /// block against the same parent (design doc §3).
    memo: DifficultyMemo<B>,
    _block: PhantomData<fn() -> B>,
}

impl<B: BlockT, C> GhostPowAlgorithm<B, C> {
    /// `initial_difficulty` must equal the genesis `Difficulty` configured in
    /// `pallet-ghost-consensus` (chain spec `ghostConsensus.difficulty`).
    pub fn new(client: Arc<C>, initial_difficulty: U256) -> Self {
        Self {
            client,
            initial_difficulty,
            memo: Arc::new(Mutex::new(None)),
            _block: PhantomData,
        }
    }
}

impl<B: BlockT, C> Clone for GhostPowAlgorithm<B, C> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            initial_difficulty: self.initial_difficulty,
            memo: self.memo.clone(),
            _block: PhantomData,
        }
    }
}

/// Stateless seal check shared by [`GhostPowAlgorithm::verify`] and tests.
///
/// `pre_digest` must be `Some` bytes that SCALE-decode to an `AccountId32`;
/// `seal_bytes` must decode (all bytes consumed) to a `GhostSeal`.
pub fn verify_seal(
    pre_hash: &[u8],
    pre_digest: Option<&[u8]>,
    seal_bytes: &[u8],
    difficulty: U256,
) -> bool {
    let pre_digest = match pre_digest {
        Some(d) => d,
        None => return false,
    };
    if author_from_pre_digest(pre_digest).is_err() {
        return false;
    }
    let seal = match GhostSeal::decode_all(&mut &seal_bytes[..]) {
        Ok(seal) => seal,
        Err(_) => return false,
    };
    // The embedded pre-hash must match the real one — it exists so fork
    // choice can trust it for tie-breaking (see `GhostSeal::pre_hash`).
    if seal.pre_hash[..] != *pre_hash {
        return false;
    }
    pow_meets(pow_value(pre_hash, pre_digest, &seal), difficulty)
}

/// Extract the tie-break `pre_hash` from raw seal bytes, if they decode.
/// `HeaviestChain` uses this so its ordering matches `break_tie` exactly.
pub fn seal_pre_hash(seal_bytes: &[u8]) -> Option<[u8; 32]> {
    GhostSeal::decode_all(&mut &seal_bytes[..])
        .ok()
        .map(|seal| seal.pre_hash)
}

/// Equal-total-difficulty tie-break: does `new_seal` beat `own_seal`? The
/// seal carrying the smaller embedded `pre_hash` wins; a seal that does not
/// decode never wins. `HeaviestChain` applies the identical ordering to its
/// leaves, so the import-time and selection-time fork choices cannot
/// diverge on ties (earliest-seen would let arrival order split the
/// network).
pub fn seal_beats(own_seal: &[u8], new_seal: &[u8]) -> bool {
    match (seal_pre_hash(own_seal), seal_pre_hash(new_seal)) {
        (Some(own), Some(new)) => new < own,
        _ => false,
    }
}

/// Decode the miner account from the `PreRuntime(POW_ENGINE_ID, _)` digest in
/// a header. Returns `None` when the item is absent or undecodable — callers
/// must not treat such a block as having an author.
pub fn author_from_header<H: HeaderT>(header: &H) -> Option<AccountId32> {
    header.digest().logs().iter().find_map(|log| match log {
        DigestItem::PreRuntime(id, bytes) if id == &POW_ENGINE_ID => {
            author_from_pre_digest(bytes).ok()
        }
        _ => None,
    })
}

impl<B, C> PowAlgorithm<B> for GhostPowAlgorithm<B, C>
where
    B: BlockT,
    C: ProvideRuntimeApi<B> + AuxStore + Send + Sync,
    C::Api: GhostPowApi<B>,
{
    type Difficulty = U256;

    fn difficulty(&self, parent: B::Hash) -> Result<Self::Difficulty, Error<B>> {
        {
            let memo = self.memo.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((hash, difficulty)) = *memo {
                if hash == parent {
                    return Ok(difficulty);
                }
            }
        }

        // The runtime is the ONLY source of truth: its `on_finalize` retarget
        // adjusts `Difficulty` every RETARGET_INTERVAL blocks. On API failure
        // we REJECT rather than fall back to the parent's aux value — after a
        // retarget boundary the parent value may be up to 4x off, so silently
        // accepting it would let seals pass at a stale difficulty (and split
        // verifiers on transient executor errors).
        let difficulty = self
            .client
            .runtime_api()
            .next_difficulty(parent)
            .map_err(|e| {
                Error::<B>::Environment(format!(
                    "GhostPowApi::next_difficulty({parent:?}) failed: {e}"
                ))
            })?;

        let mut memo = self.memo.lock().unwrap_or_else(|e| e.into_inner());
        *memo = Some((parent, difficulty));
        Ok(difficulty)
    }

    fn preliminary_verify(
        &self,
        _pre_hash: &B::Hash,
        _seal: &Seal,
    ) -> Result<Option<bool>, Error<B>> {
        // Design doc §3: full verification needs parent context; no stateless
        // pre-check is offered.
        Ok(None)
    }

    fn break_tie(&self, own_seal: &Seal, new_seal: &Seal) -> bool {
        seal_beats(own_seal, new_seal)
    }

    fn verify(
        &self,
        _parent: &BlockId<B>,
        pre_hash: &B::Hash,
        pre_digest: Option<&[u8]>,
        seal: &Seal,
        difficulty: Self::Difficulty,
    ) -> Result<bool, Error<B>> {
        Ok(verify_seal(pre_hash.as_ref(), pre_digest, seal, difficulty))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mining::{miner_pre_runtime, pow_meets};
    use codec::Encode;
    use sp_runtime::{generic::Digest, testing::Header as TestHeader};

    #[test]
    fn verify_seal_accepts_and_rejects() {
        let author = AccountId32::new([7u8; 32]);
        let pre_digest = miner_pre_runtime(&author);
        let pre_hash = [3u8; 32];
        let seal = GhostSeal {
            nonce: 42,
            pre_hash,
        };
        let seal_bytes = seal.encode();

        // Work factor 1: always meets. Work factor 0: never meets.
        assert!(verify_seal(
            &pre_hash,
            Some(&pre_digest),
            &seal_bytes,
            U256::one()
        ));
        assert!(!verify_seal(
            &pre_hash,
            Some(&pre_digest),
            &seal_bytes,
            U256::zero()
        ));

        // Exact boundary: meets iff value <= MAX / difficulty.
        let value = pow_value(&pre_hash, &pre_digest, &seal);
        if !value.is_zero() {
            let exact = U256::MAX / value;
            assert!(verify_seal(
                &pre_hash,
                Some(&pre_digest),
                &seal_bytes,
                exact
            ));
            assert!(!verify_seal(
                &pre_hash,
                Some(&pre_digest),
                &seal_bytes,
                exact + 1
            ));
        }
    }

    #[test]
    fn verify_seal_rejects_malformed_inputs() {
        let pre_digest = miner_pre_runtime(&AccountId32::new([7u8; 32]));
        let pre_hash = [3u8; 32];
        let seal_bytes = GhostSeal { nonce: 1, pre_hash }.encode();

        // Missing pre-runtime digest: no miner attribution possible.
        assert!(!verify_seal(&pre_hash, None, &seal_bytes, U256::one()));
        // Truncated account bytes cannot decode to AccountId32.
        assert!(!verify_seal(
            &pre_hash,
            Some(&[0u8; 10]),
            &seal_bytes,
            U256::one()
        ));
        // Undecodable / padded seal bytes are not a valid GhostSeal.
        assert!(!verify_seal(
            &pre_hash,
            Some(&pre_digest),
            &[0u8; 3],
            U256::one()
        ));
        assert!(!verify_seal(
            &pre_hash,
            Some(&pre_digest),
            &[&seal_bytes[..], &[0u8]].concat(),
            U256::one()
        ));
        // A seal embedded with a foreign pre_hash is rejected even though the
        // hash input would otherwise commit it — fork choice must be able to
        // trust the embedded value.
        let foreign = GhostSeal {
            nonce: 1,
            pre_hash: [9u8; 32],
        }
        .encode();
        assert!(!verify_seal(
            &pre_hash,
            Some(&pre_digest),
            &foreign,
            U256::one()
        ));
    }

    #[test]
    fn pow_meets_boundary() {
        assert!(pow_meets(U256::one(), U256::MAX));
        assert!(!pow_meets(U256::from(2u64), U256::MAX));
    }

    #[test]
    fn seal_beats_orders_by_embedded_pre_hash() {
        let seal_a = GhostSeal {
            nonce: 1,
            pre_hash: [0x0au8; 32],
        }
        .encode();
        let seal_b = GhostSeal {
            nonce: 2,
            pre_hash: [0x0bu8; 32],
        }
        .encode();
        // Smaller embedded pre_hash wins; nonce is irrelevant.
        assert!(seal_beats(&seal_b, &seal_a));
        assert!(!seal_beats(&seal_a, &seal_b));
        // Identical seal does not beat itself; undecodable never wins and
        // never loses to a valid seal.
        assert!(!seal_beats(&seal_a, &seal_a));
        assert!(!seal_beats(&seal_a, &[0u8; 3]));
        assert!(!seal_beats(&[0u8; 3], &seal_a));
    }

    #[test]
    fn author_from_header_decodes_pre_runtime() {
        let author = AccountId32::new([9u8; 32]);
        let mut digest = Digest::default();
        digest.push(DigestItem::PreRuntime(POW_ENGINE_ID, author.encode()));
        let header = TestHeader::new(
            1,
            Default::default(),
            Default::default(),
            Default::default(),
            digest,
        );
        assert_eq!(author_from_header(&header), Some(author));

        // Undecodable payload -> no author.
        let mut bad = Digest::default();
        bad.push(DigestItem::PreRuntime(POW_ENGINE_ID, vec![0u8; 10]));
        let header = TestHeader::new(
            1,
            Default::default(),
            Default::default(),
            Default::default(),
            bad,
        );
        assert_eq!(author_from_header(&header), None);
    }
}
