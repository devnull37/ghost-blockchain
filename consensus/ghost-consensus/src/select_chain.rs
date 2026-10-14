//! `HeaviestChain`: `SelectChain` that follows cumulative PoW work.
//!
//! `docs/ghost-consensus-design.md` §9: among the chain's leaves pick the one
//! with the greatest `PowAux.total_difficulty`; on a tie, the leaf whose seal
//! embeds the smaller `pre_hash` wins — the exact ordering
//! `GhostPowAlgorithm::break_tie` applies inside `PowBlockImport`, so the
//! import-time and selection-time fork choices cannot diverge. Block number
//! is NOT part of the ordering: `break_tie` sees only seal bytes, so any
//! rule it cannot reproduce would split the network on ties.
//! `LongestChain` is wrong here — it selects by block number while
//! `PowBlockImport` decides the client's best by
//! `ForkChoiceStrategy::Custom(total_difficulty)`. The same instance feeds
//! the GRANDPA voter so finality tracks the heaviest chain.
//!
//! The ancestor-walk logic mirrors `sc_consensus::LongestChain`, with leaf
//! selection by total difficulty instead of block number.

use std::{marker::PhantomData, sync::Arc};

use sc_client_api::backend;
use sc_consensus_pow::PowAux;
use sp_blockchain::{Backend as _, Error as ClientError, HeaderBackend};
use sp_consensus::{Error as ConsensusError, SelectChain};
use sp_core::U256;
use sp_runtime::{
    traits::{Block as BlockT, Header as HeaderT, NumberFor},
    DigestItem,
};

use ghost_pow_primitives::POW_ENGINE_ID;

use crate::algorithm::seal_pre_hash;

/// The running best leaf while scanning `leaves()`.
type BestLeaf<Block> = (U256, [u8; 32], <Block as BlockT>::Header);

/// A `SelectChain` that picks the leaf with the most accumulated work.
pub struct HeaviestChain<B, Block> {
    backend: Arc<B>,
    _phantom: PhantomData<fn() -> Block>,
}

impl<B, Block> Clone for HeaviestChain<B, Block> {
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            _phantom: PhantomData,
        }
    }
}

/// `candidate` strictly outranks `incumbent` under the Ghost fork-choice
/// order: more total work, else the smaller seal `pre_hash` — identical to
/// `GhostPowAlgorithm::break_tie`.
///
/// Extracted as a pure function so the ordering is directly testable.
fn leaf_beats(candidate: (U256, &[u8; 32]), incumbent: (U256, &[u8; 32])) -> bool {
    let (cand_total, cand_pre_hash) = candidate;
    let (best_total, best_pre_hash) = incumbent;
    cand_total > best_total || (cand_total == best_total && cand_pre_hash < best_pre_hash)
}

/// Tie-break key for a leaf header: the `pre_hash` embedded in its PoW seal,
/// or `[0xff; 32]` (loses every tie) when the header carries no decodable
/// Ghost seal — e.g. genesis.
fn seal_tie_hash<Header: HeaderT>(header: &Header) -> [u8; 32] {
    header
        .digest()
        .logs()
        .iter()
        .find_map(|log| match log {
            DigestItem::Seal(id, bytes) if id == &POW_ENGINE_ID => seal_pre_hash(bytes),
            _ => None,
        })
        .unwrap_or([0xff; 32])
}

impl<B, Block> HeaviestChain<B, Block>
where
    B: backend::Backend<Block>,
    Block: BlockT,
{
    /// Create a `HeaviestChain` over the given backend.
    pub fn new(backend: Arc<B>) -> Self {
        Self {
            backend,
            _phantom: PhantomData,
        }
    }

    fn leaves(&self) -> Result<Vec<Block::Hash>, ClientError> {
        self.backend.blockchain().leaves()
    }

    /// Total work recorded for `hash`; zero when no aux exists (e.g. genesis),
    /// which degrades selection to the seal `pre_hash` tie-break.
    fn total_difficulty(&self, hash: &Block::Hash) -> Result<U256, ClientError> {
        PowAux::<U256>::read::<_, Block>(self.backend.as_ref(), hash)
            .map(|aux| aux.total_difficulty)
            .map_err(|e| ClientError::Application(e.to_string().into()))
    }

    fn best_header(&self) -> Result<Block::Header, ClientError> {
        let blockchain = self.backend.blockchain();
        let mut best: Option<BestLeaf<Block>> = None;
        for hash in self.leaves()? {
            let header = blockchain
                .header(hash)?
                .ok_or_else(|| ClientError::MissingHeader(hash.to_string()))?;
            let total = self.total_difficulty(&hash)?;
            let tie_hash = seal_tie_hash(&header);
            let wins = match &best {
                None => true,
                Some((best_total, best_pre_hash, _)) => {
                    leaf_beats((total, &tie_hash), (*best_total, best_pre_hash))
                }
            };
            if wins {
                best = Some((total, tie_hash, header));
            }
        }
        best.map(|(_, _, header)| header)
            .ok_or_else(|| ClientError::Backend("no leaves in the chain".into()))
    }

    /// Highest descendant of `base_hash` on the heaviest chain that is a valid
    /// finality candidate — identical semantics to `LongestChain`'s
    /// implementation, but anchored at the heaviest head instead of the
    /// longest one.
    fn finality_target(
        &self,
        base_hash: Block::Hash,
        maybe_max_number: Option<NumberFor<Block>>,
    ) -> Result<Block::Hash, ClientError> {
        use sp_blockchain::Error::{Application, MissingHeader};
        let blockchain = self.backend.blockchain();

        let mut current_head = self.best_header()?;
        let mut best_hash = current_head.hash();

        let base_header = blockchain
            .header(base_hash)?
            .ok_or_else(|| MissingHeader(base_hash.to_string()))?;
        let base_number = *base_header.number();

        if let Some(max_number) = maybe_max_number {
            if max_number < base_number {
                let msg = format!(
                    "Requested a finality target using max number {max_number} below the base number {base_number}"
                );
                return Err(Application(msg.into()));
            }

            while current_head.number() > &max_number {
                best_hash = *current_head.parent_hash();
                current_head = blockchain
                    .header(best_hash)?
                    .ok_or_else(|| MissingHeader(format!("{best_hash:?}")))?;
            }
        }

        while current_head.hash() != base_hash {
            if *current_head.number() < base_number {
                let msg = format!(
                    "Requested a finality target using a base {base_hash:?} not in the best chain {best_hash:?}"
                );
                return Err(Application(msg.into()));
            }
            let current_hash = *current_head.parent_hash();
            current_head = blockchain
                .header(current_hash)?
                .ok_or_else(|| MissingHeader(format!("{best_hash:?}")))?;
        }

        Ok(best_hash)
    }
}

#[async_trait::async_trait]
impl<B, Block> SelectChain<Block> for HeaviestChain<B, Block>
where
    B: backend::Backend<Block>,
    Block: BlockT,
{
    async fn leaves(&self) -> Result<Vec<Block::Hash>, ConsensusError> {
        HeaviestChain::leaves(self).map_err(|e| ConsensusError::ChainLookup(e.to_string()))
    }

    async fn best_chain(&self) -> Result<Block::Header, ConsensusError> {
        self.best_header()
            .map_err(|e| ConsensusError::ChainLookup(e.to_string()))
    }

    async fn finality_target(
        &self,
        base_hash: Block::Hash,
        maybe_max_number: Option<NumberFor<Block>>,
    ) -> Result<Block::Hash, ConsensusError> {
        HeaviestChain::finality_target(self, base_hash, maybe_max_number)
            .map_err(|e| ConsensusError::ChainLookup(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn leaf_ordering() {
        let h1 = [1u8; 32];
        let h2 = [2u8; 32];

        // More total work always wins, regardless of the tie-break key.
        assert!(leaf_beats(
            (U256::from(100u64), &h2),
            (U256::from(99u64), &h1),
        ));
        assert!(!leaf_beats(
            (U256::from(99u64), &h1),
            (U256::from(100u64), &h2),
        ));

        // Equal work: the smaller seal `pre_hash` wins — same rule as
        // `GhostPowAlgorithm::break_tie`.
        assert!(leaf_beats(
            (U256::from(50u64), &h1),
            (U256::from(50u64), &h2)
        ));
        assert!(!leaf_beats(
            (U256::from(50u64), &h2),
            (U256::from(50u64), &h1)
        ));

        // Identical leaf does not beat itself.
        assert!(!leaf_beats(
            (U256::from(50u64), &h1),
            (U256::from(50u64), &h1)
        ));
    }

    proptest::proptest! {
        /// `leaf_beats` must be a strict total order on (total, tie_hash)
        /// pairs — the same ordering `break_tie` applies — or the import-time
        /// and selection-time fork choices can diverge.
        #[test]
        fn leaf_beats_is_a_total_order(
            a in (any::<u64>(), any::<[u8; 32]>()),
            b in (any::<u64>(), any::<[u8; 32]>()),
            c in (any::<u64>(), any::<[u8; 32]>()),
        ) {
            let (at, ah) = (U256::from(a.0), &a.1);
            let (bt, bh) = (U256::from(b.0), &b.1);
            let (ct, ch) = (U256::from(c.0), &c.1);

            // Irreflexive.
            prop_assert!(!leaf_beats((at, ah), (at, ah)));

            // Asymmetric: at most one direction can win.
            prop_assert!(!(leaf_beats((at, ah), (bt, bh)) && leaf_beats((bt, bh), (at, ah))));

            // Total on distinct keys: distinct (total, pre_hash) pairs order.
            if (at, ah) != (bt, bh) {
                prop_assert!(
                    leaf_beats((at, ah), (bt, bh)) || leaf_beats((bt, bh), (at, ah)),
                    "distinct leaves must be comparable"
                );
            }

            // Transitive: a beats b and b beats c implies a beats c.
            if leaf_beats((at, ah), (bt, bh)) && leaf_beats((bt, bh), (ct, ch)) {
                prop_assert!(leaf_beats((at, ah), (ct, ch)));
            }
        }
    }
}
