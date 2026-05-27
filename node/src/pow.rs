//! Real Proof-of-Work algorithm for the Ghost node.
//!
//! Block authoring is driven by `sc-consensus-pow`. A miner searches for a `nonce`
//! such that the double-Blake2-256 hash of `(pre_hash || nonce)` satisfies the
//! current difficulty. Difficulty is read from the runtime via the standard
//! `sp_consensus_pow::DifficultyApi`, which exposes the value retargeted on-chain by
//! `pallet-ghost-consensus`.
//!
//! Difficulty uses the conventional convention: numerically larger = harder. A hash
//! (as a big-endian 256-bit integer) is valid iff it is `<= U256::MAX / difficulty`.
//! This makes difficulty a direct measure of work, which is exactly what
//! `sc-consensus-pow` sums for fork-choice (`TotalDifficulty`).

use std::sync::Arc;

use codec::{Decode, Encode};
use sc_consensus_pow::{Error as PowError, PowAlgorithm};
use sp_api::ProvideRuntimeApi;
use sp_consensus_pow::{DifficultyApi, Seal};
use sp_core::{H256, U256};
use sp_runtime::{
    generic::BlockId,
    traits::{BlakeTwo256, Block as BlockT, Hash},
};

use solochain_template_runtime::opaque::Block;

/// The PoW seal: a single nonce. SCALE-encoded into the opaque `Seal` (`Vec<u8>`).
#[derive(Clone, Copy, PartialEq, Eq, Encode, Decode, Debug)]
pub struct GhostSeal {
    pub nonce: u64,
}

/// Double Blake2-256 work hash over `(pre_hash || nonce)`.
pub fn pow_hash(pre_hash: &H256, nonce: u64) -> H256 {
    let mut data = [0u8; 40];
    data[..32].copy_from_slice(pre_hash.as_bytes());
    data[32..].copy_from_slice(&nonce.to_le_bytes());
    let first = BlakeTwo256::hash(&data);
    BlakeTwo256::hash(first.as_bytes())
}

/// Whether `hash` satisfies `difficulty` (larger difficulty = harder).
pub fn meets_difficulty(hash: &H256, difficulty: U256) -> bool {
    let work = U256::from_big_endian(hash.as_bytes());
    let target = U256::MAX / difficulty.max(U256::one());
    work <= target
}

/// The Ghost PoW algorithm. Reads difficulty from the runtime each block.
pub struct GhostPow<C> {
    client: Arc<C>,
}

impl<C> GhostPow<C> {
    pub fn new(client: Arc<C>) -> Self {
        Self { client }
    }
}

impl<C> Clone for GhostPow<C> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
        }
    }
}

impl<C> PowAlgorithm<Block> for GhostPow<C>
where
    C: ProvideRuntimeApi<Block> + Send + Sync,
    C::Api: DifficultyApi<Block, U256>,
{
    type Difficulty = U256;

    fn difficulty(&self, parent: <Block as BlockT>::Hash) -> Result<Self::Difficulty, PowError<Block>> {
        self.client
            .runtime_api()
            .difficulty(parent)
            .map_err(|err| PowError::Other(format!("difficulty runtime API failed: {err}")))
    }

    fn verify(
        &self,
        _parent: &BlockId<Block>,
        pre_hash: &<Block as BlockT>::Hash,
        _pre_digest: Option<&[u8]>,
        seal: &Seal,
        difficulty: Self::Difficulty,
    ) -> Result<bool, PowError<Block>> {
        let seal = match GhostSeal::decode(&mut &seal[..]) {
            Ok(seal) => seal,
            Err(_) => return Ok(false),
        };
        Ok(meets_difficulty(&pow_hash(pre_hash, seal.nonce), difficulty))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difficulty_one_accepts_any_hash() {
        // difficulty = 1 => target = U256::MAX => every hash is valid.
        assert!(meets_difficulty(&H256::repeat_byte(0xFF), U256::one()));
    }

    #[test]
    fn high_difficulty_rejects_most_hashes() {
        // An all-0xFF hash is the maximum value, so any difficulty > 1 rejects it.
        assert!(!meets_difficulty(&H256::repeat_byte(0xFF), U256::from(2u64)));
    }

    #[test]
    fn mining_finds_a_nonce_at_easy_difficulty() {
        let pre_hash = H256::repeat_byte(0x42);
        let difficulty = U256::from(8u64); // ~1 in 8 hashes pass
        let mut nonce = 0u64;
        let mut found = None;
        for _ in 0..100_000 {
            if meets_difficulty(&pow_hash(&pre_hash, nonce), difficulty) {
                found = Some(nonce);
                break;
            }
            nonce += 1;
        }
        let nonce = found.expect("a valid nonce exists at low difficulty");
        // The seal round-trips and verifies.
        let seal = GhostSeal { nonce };
        assert!(meets_difficulty(&pow_hash(&pre_hash, seal.nonce), difficulty));
    }
}
