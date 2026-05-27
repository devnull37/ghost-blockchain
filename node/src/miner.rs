//! Local CPU mining demo for the Ghost Proof-of-Work algorithm.
//!
//! This runs the EXACT work function the live node authors blocks with:
//! `crate::pow::pow_hash` (double Blake2-256 over `pre_hash || nonce`) checked against
//! the conventional `U256` difficulty by `crate::pow::meets_difficulty`. It is a
//! standalone benchmark/illustration that searches for a valid nonce over a synthetic
//! pre-hash and reports the hash rate.
//!
//! It does NOT submit blocks to a running node and does not author live chain blocks —
//! real authoring is driven by `sc-consensus-pow` in `service.rs`. Keeping the demo on
//! `crate::pow` means there is a single source of truth for the PoW algorithm.

use sp_core::{H256, U256};
use sp_runtime::traits::{BlakeTwo256, Hash};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::pow::{meets_difficulty, pow_hash};

/// Mining statistics
#[derive(Debug, Clone, Default)]
pub struct MiningStats {
    pub hashes_computed: u64,
    pub blocks_found: u64,
    pub hash_rate: f64,
    pub elapsed_time: Duration,
}

/// Block header data for the mining demo. These fields are folded into a synthetic
/// pre-hash, mirroring how the node mines over a block's pre-seal hash.
#[derive(Clone, Debug)]
pub struct MiningBlockHeader {
    pub number: u32,
    pub parent_hash: H256,
    pub state_root: H256,
    pub extrinsics_root: H256,
    /// Conventional difficulty: numerically larger = harder (matches the runtime and
    /// `crate::pow`). A nonce is valid iff `pow_hash <= U256::MAX / difficulty`.
    pub difficulty: u64,
}

/// Miner instance
pub struct Miner {
    threads: usize,
    difficulty: U256,
    running: Arc<AtomicBool>,
    hashes: Arc<AtomicU64>,
}

impl Miner {
    /// Create a new miner. `difficulty` uses the conventional convention (larger = harder).
    pub fn new(threads: usize, difficulty: u64) -> Result<Self, &'static str> {
        if threads == 0 {
            return Err("mining threads must be at least 1");
        }

        Ok(Self {
            threads,
            difficulty: U256::from(difficulty.max(1)),
            running: Arc::new(AtomicBool::new(false)),
            hashes: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Search for a nonce whose Ghost PoW hash satisfies the difficulty.
    pub fn start(&self, block_header: MiningBlockHeader) -> Option<(u64, MiningStats)> {
        println!("Starting local Ghost PoW demo mining...");
        println!("   Block Number: {}", block_header.number);
        println!(
            "   Difficulty (conventional, larger = harder): {}",
            block_header.difficulty
        );
        println!("   Mining Threads: {}", self.threads);
        println!("   Algorithm: double Blake2-256 over pre_hash || nonce (same as live authoring)");
        println!("   Note: this demo does not submit blocks to a running node.\n");

        // Synthetic pre-hash from the header fields. The live node instead mines over the
        // block's real pre-seal hash, but the per-nonce work function is identical.
        let pre_hash = BlakeTwo256::hash_of(&(
            block_header.number,
            block_header.parent_hash,
            block_header.state_root,
            block_header.extrinsics_root,
        ));

        self.hashes.store(0, Ordering::SeqCst);
        self.running.store(true, Ordering::SeqCst);
        let start_time = Instant::now();

        let found_nonce = Arc::new(AtomicU64::new(0));
        let found_solution = Arc::new(AtomicBool::new(false));

        let mut handles = vec![];

        // Spawn mining threads; each searches a disjoint arithmetic subsequence of nonces.
        for thread_id in 0..self.threads {
            let running = Arc::clone(&self.running);
            let hashes = Arc::clone(&self.hashes);
            let found_nonce = Arc::clone(&found_nonce);
            let found_solution = Arc::clone(&found_solution);
            let difficulty = self.difficulty;
            let threads = self.threads;

            let handle = thread::spawn(move || {
                let mut nonce = thread_id as u64;
                let step = threads as u64;

                while running.load(Ordering::SeqCst) && !found_solution.load(Ordering::SeqCst) {
                    hashes.fetch_add(1, Ordering::Relaxed);

                    if meets_difficulty(&pow_hash(&pre_hash, nonce), difficulty) {
                        found_solution.store(true, Ordering::SeqCst);
                        found_nonce.store(nonce, Ordering::SeqCst);
                        running.store(false, Ordering::SeqCst);
                        break;
                    }

                    nonce = nonce.wrapping_add(step);
                }
            });

            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            let _ = handle.join();
        }

        let elapsed = start_time.elapsed();
        let total_hashes = self.hashes.load(Ordering::SeqCst);
        let hash_rate = total_hashes as f64 / elapsed.as_secs_f64().max(f64::MIN_POSITIVE);

        let stats = MiningStats {
            hashes_computed: total_hashes,
            blocks_found: if found_solution.load(Ordering::SeqCst) {
                1
            } else {
                0
            },
            hash_rate,
            elapsed_time: elapsed,
        };

        if found_solution.load(Ordering::SeqCst) {
            let nonce = found_nonce.load(Ordering::SeqCst);
            println!("\nLocal PoW demo found a valid nonce.");
            println!("   Nonce: {}", nonce);
            println!("   Hashes computed: {}", total_hashes);
            println!("   Hash rate: {:.2} H/s", hash_rate);
            println!("   Time elapsed: {:.2}s", elapsed.as_secs_f64());
            Some((nonce, stats))
        } else {
            println!("\nMining stopped without finding a solution.");
            println!("   Hashes computed: {}", total_hashes);
            println!("   Time elapsed: {:.2}s", elapsed.as_secs_f64());
            None
        }
    }

    /// Stop mining
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_mines_a_nonce_that_verifies_under_the_node_pow() {
        let header = MiningBlockHeader {
            number: 1,
            parent_hash: H256::zero(),
            state_root: H256::from_low_u64_be(1),
            extrinsics_root: H256::from_low_u64_be(2),
            difficulty: 8, // conventional: ~1 in 8 hashes pass -> found quickly
        };
        let pre_hash = BlakeTwo256::hash_of(&(
            header.number,
            header.parent_hash,
            header.state_root,
            header.extrinsics_root,
        ));

        let (nonce, stats) = Miner::new(2, 8)
            .expect("valid non-zero thread count")
            .start(header)
            .expect("a valid nonce exists at low difficulty");

        assert_eq!(stats.blocks_found, 1);
        assert!(stats.hashes_computed > 0);
        // The demo's result verifies under the SAME function the node checks seals with.
        assert!(meets_difficulty(&pow_hash(&pre_hash, nonce), U256::from(8u64)));
    }
}
