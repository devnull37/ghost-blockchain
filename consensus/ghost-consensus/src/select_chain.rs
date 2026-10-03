//! `HeaviestChain`: `SelectChain` that follows cumulative PoW work.
//!
//! `docs/ghost-consensus-design.md` §9: among the chain's leaves pick the one
//! with the greatest `PowAux.total_difficulty`; tie-break on the higher block
//! number, then the lower header hash. `LongestChain` is wrong here — it
//! selects by block number while `PowBlockImport` decides the client's best
//! by `ForkChoiceStrategy::Custom(total_difficulty)`. The same instance feeds
//! the GRANDPA voter so finality tracks the heaviest chain.
//!
//! The ordering and ancestor-walk logic mirror `sc_consensus::LongestChain`,
//! with leaf selection by total difficulty instead of block number.

use std::{marker::PhantomData, sync::Arc};

use sc_client_api::backend;
use sc_consensus_pow::PowAux;
use sp_blockchain::{Backend as _, Error as ClientError, HeaderBackend};
use sp_consensus::{Error as ConsensusError, SelectChain};
use sp_core::U256;
use sp_runtime::traits::{Block as BlockT, Header as HeaderT, NumberFor};

/// The running best leaf while scanning `leaves()`.
type BestLeaf<Block> = (
    U256,
    NumberFor<Block>,
    <Block as BlockT>::Hash,
    <Block as BlockT>::Header,
);

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
/// order: more total work, else higher number, else lower hash.
///
/// Extracted as a pure function so the ordering is directly testable.
fn leaf_beats<Number, Hash>(
    candidate: (U256, &Number, &Hash),
    incumbent: (U256, &Number, &Hash),
) -> bool
where
    Number: Ord,
    Hash: AsRef<[u8]>,
{
    let (cand_total, cand_number, cand_hash) = candidate;
    let (best_total, best_number, best_hash) = incumbent;
    cand_total > best_total
        || (cand_total == best_total && cand_number > best_number)
        || (cand_total == best_total
            && cand_number == best_number
            && cand_hash.as_ref() < best_hash.as_ref())
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
    /// which degrades selection to the number/hash tie-breaks.
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
            let number = *header.number();
            let wins = match &best {
                None => true,
                Some((best_total, best_number, best_hash, _)) => leaf_beats(
                    (total, &number, &hash),
                    (*best_total, best_number, best_hash),
                ),
            };
            if wins {
                best = Some((total, number, hash, header));
            }
        }
        best.map(|(_, _, _, header)| header)
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

    #[test]
    fn leaf_ordering() {
        let h1 = [1u8; 32];
        let h2 = [2u8; 32];
        let n1 = 10u32;
        let n2 = 11u32;

        // More total work always wins, regardless of number/hash.
        assert!(leaf_beats(
            (U256::from(100u64), &n1, &h2),
            (U256::from(99u64), &n2, &h1),
        ));
        assert!(!leaf_beats(
            (U256::from(99u64), &n2, &h1),
            (U256::from(100u64), &n1, &h2),
        ));

        // Equal work: higher number wins.
        assert!(leaf_beats(
            (U256::from(50u64), &n2, &h2),
            (U256::from(50u64), &n1, &h1),
        ));

        // Equal work and number: lower hash wins.
        assert!(leaf_beats(
            (U256::from(50u64), &n1, &h1),
            (U256::from(50u64), &n1, &h2),
        ));
        assert!(!leaf_beats(
            (U256::from(50u64), &n1, &h2),
            (U256::from(50u64), &n1, &h1),
        ));

        // Identical leaf does not beat itself.
        assert!(!leaf_beats(
            (U256::from(50u64), &n1, &h1),
            (U256::from(50u64), &n1, &h1),
        ));
    }
}
