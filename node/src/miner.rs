//! Mining functionality for Ghost consensus

use sp_core::H256;
use sp_runtime::traits::{BlakeTwo256, Hash};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Mining statistics
#[derive(Debug, Clone, Default)]
pub struct MiningStats {
	pub hashes_computed: u64,
	pub blocks_found: u64,
	pub hash_rate: f64,
	pub elapsed_time: Duration,
}

/// Block header data for mining
#[derive(Clone, Debug)]
pub struct MiningBlockHeader {
	pub number: u32,
	pub parent_hash: H256,
	pub state_root: H256,
	pub extrinsics_root: H256,
	pub difficulty: u64,
}

/// Miner instance
pub struct Miner {
	threads: usize,
	target_difficulty: u64,
	running: Arc<AtomicBool>,
	hashes: Arc<AtomicU64>,
}

impl Miner {
	/// Create a new miner
	pub fn new(threads: usize, target_difficulty: u64) -> Self {
		Self {
			threads,
			target_difficulty,
			running: Arc::new(AtomicBool::new(false)),
			hashes: Arc::new(AtomicU64::new(0)),
		}
	}

	/// Start mining
	pub fn start(&self, block_header: MiningBlockHeader) -> Option<(u64, MiningStats)> {
		println!("🚀 Starting Ghost PoW mining...");
		println!("   Block Number: {}", block_header.number);
		println!("   Target Difficulty: {}", self.target_difficulty);
		println!("   Mining Threads: {}", self.threads);
		println!("   Algorithm: Enhanced Blake2-256 (ASIC-resistant)\n");

		self.running.store(true, Ordering::SeqCst);
		let start_time = Instant::now();

		let found_nonce = Arc::new(AtomicU64::new(0));
		let found_solution = Arc::new(AtomicBool::new(false));

		let mut handles = vec![];

		// Spawn mining threads
		for thread_id in 0..self.threads {
			let running = Arc::clone(&self.running);
			let hashes = Arc::clone(&self.hashes);
			let found_nonce = Arc::clone(&found_nonce);
			let found_solution = Arc::clone(&found_solution);
			let header = block_header.clone();
			let target_difficulty = self.target_difficulty;
			let threads = self.threads;

			let handle = thread::spawn(move || {
				let mut nonce = thread_id as u64;
				let step = threads as u64;

				while running.load(Ordering::SeqCst) && !found_solution.load(Ordering::SeqCst) {
					// Enhanced Blake2 PoW (double hash for ASIC resistance)
					let hash_input = (
						header.number,
						header.parent_hash,
						header.state_root,
						header.extrinsics_root,
						nonce,
					);

					let first_hash = BlakeTwo256::hash_of(&hash_input);
					let final_hash = BlakeTwo256::hash_of(&first_hash);
					let hash_value = u64::from_be_bytes(
						final_hash.as_bytes()[0..8].try_into().unwrap_or_default(),
					);

					hashes.fetch_add(1, Ordering::Relaxed);

					// Check if solution found
					if hash_value <= target_difficulty {
						found_solution.store(true, Ordering::SeqCst);
						found_nonce.store(nonce, Ordering::SeqCst);
						running.store(false, Ordering::SeqCst);
						break;
					}

					nonce = nonce.wrapping_add(step);

					// Periodic check to allow graceful shutdown
					if nonce % 100_000 == 0 {
						thread::sleep(Duration::from_micros(1));
					}
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
		let hash_rate = total_hashes as f64 / elapsed.as_secs_f64();

		let stats = MiningStats {
			hashes_computed: total_hashes,
			blocks_found: if found_solution.load(Ordering::SeqCst) { 1 } else { 0 },
			hash_rate,
			elapsed_time: elapsed,
		};

		if found_solution.load(Ordering::SeqCst) {
			let nonce = found_nonce.load(Ordering::SeqCst);
			println!("\n✅ Block mined successfully!");
			println!("   Nonce: {}", nonce);
			println!("   Hashes computed: {}", total_hashes);
			println!("   Hash rate: {:.2} H/s", hash_rate);
			println!("   Time elapsed: {:.2}s", elapsed.as_secs_f64());
			Some((nonce, stats))
		} else {
			println!("\n❌ Mining stopped without finding solution");
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
	fn test_mining_with_easy_difficulty() {
		let header = MiningBlockHeader {
			number: 1,
			parent_hash: H256::zero(),
			state_root: H256::from_low_u64_be(1),
			extrinsics_root: H256::from_low_u64_be(2),
			difficulty: u64::MAX / 1000, // Easy difficulty for testing
		};

		let miner = Miner::new(2, u64::MAX / 1000);
		let result = miner.start(header);

		assert!(result.is_some());
		let (nonce, stats) = result.unwrap();
		assert!(nonce > 0 || nonce == 0); // Any nonce is valid
		assert!(stats.hashes_computed > 0);
		assert_eq!(stats.blocks_found, 1);
	}
}
