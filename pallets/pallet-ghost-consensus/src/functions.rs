//! Core functions for Ghost Consensus

use super::*;
use crate::types::*;
use frame_support::pallet_prelude::*;
use sp_core::H256;
use sp_runtime::traits::{BlakeTwo256, Hash, SaturatedConversion};

/// Calculate mining difficulty adjustment
pub fn calculate_difficulty_adjustment<T: Config>(
	current_difficulty: u64,
	actual_block_time: u64,
	target_block_time: u64,
) -> u64 {
	let adjustment_factor = if actual_block_time < target_block_time {
		// Increase difficulty if blocks are too fast
		(target_block_time * 100) / actual_block_time
	} else {
		// Decrease difficulty if blocks are too slow
		(actual_block_time * 100) / target_block_time
	};

	(current_difficulty * adjustment_factor) / 100
}

/// Verify Proof-of-Work using Blake2-256 (ASIC-resistant, fast for 5-second blocks)
pub fn verify_pow(block_header: &GhostBlockHeader, target_difficulty: u64) -> bool {
	let hash_input = (
		block_header.number,
		block_header.parent_hash,
		block_header.state_root,
		block_header.extrinsics_root,
		block_header.nonce,
	);

	let hash = BlakeTwo256::hash_of(&hash_input);
	let hash_value = u64::from_be_bytes(hash.as_bytes()[0..8].try_into().unwrap_or_default());

	hash_value <= target_difficulty
}

/// Enhanced Blake2 PoW with additional ASIC resistance
pub fn verify_pow_enhanced(block_header: &GhostBlockHeader, target_difficulty: u64) -> bool {
	// Double hash for additional ASIC resistance
	let hash_input = (
		block_header.number,
		block_header.parent_hash,
		block_header.state_root,
		block_header.extrinsics_root,
		block_header.nonce,
	);

	let first_hash = BlakeTwo256::hash_of(&hash_input);
	let final_hash = BlakeTwo256::hash_of(&first_hash);
	let hash_value = u64::from_be_bytes(final_hash.as_bytes()[0..8].try_into().unwrap_or_default());

	hash_value <= target_difficulty
}

/// Alternative: Verify Proof-of-Work using double SHA-256 (Bitcoin-style)
pub fn verify_pow_sha256(block_header: &GhostBlockHeader, target_difficulty: u64) -> bool {
	use sp_core::sha2_256;

	let hash_input = (
		block_header.number,
		block_header.parent_hash,
		block_header.state_root,
		block_header.extrinsics_root,
		block_header.nonce,
	);

	// First SHA-256
	let first_hash = sha2_256(&hash_input.encode());

	// Second SHA-256 (Bitcoin standard)
	let final_hash = sha2_256(&first_hash);

	// Convert first 8 bytes to u64 for difficulty comparison
	let hash_value = u64::from_be_bytes(final_hash[0..8].try_into().unwrap_or_default());

	hash_value <= target_difficulty
}

/// Alternative: Verify Proof-of-Work using Keccak-256 (Ethereum-style)
pub fn verify_pow_keccak(block_header: &GhostBlockHeader, target_difficulty: u64) -> bool {
	use sp_core::keccak_256;

	let hash_input = (
		block_header.number,
		block_header.parent_hash,
		block_header.state_root,
		block_header.extrinsics_root,
		block_header.nonce,
	);

	let hash = keccak_256(&hash_input.encode());
	let hash_value = u64::from_be_bytes(hash[0..8].try_into().unwrap_or_default());

	hash_value <= target_difficulty
}

/// Select PoS validator based on stake weight
pub fn select_pos_validator<T: Config>(
	stakers: Vec<ValidatorStake<T::AccountId, BalanceOf<T>>>,
	seed: H256,
) -> Option<PosSelection<T::AccountId>> {
	if stakers.is_empty() {
		return None;
	}

	let total_weight: u64 = stakers.iter().map(|s| s.weight).sum();
	if total_weight == 0 {
		return None;
	}

	let mut random_value = u64::from_be_bytes(seed.as_bytes()[0..8].try_into().unwrap_or_default());
	random_value %= total_weight;

	let mut cumulative_weight = 0u64;
	for staker in stakers {
		cumulative_weight += staker.weight;
		if random_value < cumulative_weight {
			return Some(PosSelection {
				validator: staker.account,
				weight: staker.weight,
				round: frame_system::Pallet::<T>::block_number().saturated_into(),
			});
		}
	}

	None
}

/// Calculate block rewards
pub fn calculate_block_reward<T: Config>(total_reward: BalanceOf<T>) -> BlockReward<BalanceOf<T>> {
	let miner_reward = (total_reward * 40u32.into()) / 100u32.into(); // 40%
	let stakers_reward = total_reward - miner_reward; // 60%

	BlockReward {
		total: total_reward,
		miner_reward,
		stakers_reward,
	}
}

/// Validate block header
pub fn validate_block_header<T: Config>(
	header: &GhostBlockHeader,
	parent_header: &GhostBlockHeader,
) -> DispatchResult {
	// Check block number sequence
	ensure!(header.number == parent_header.number + 1, Error::<T>::InvalidBlockNumber);

	// Check parent hash
	ensure!(header.parent_hash == BlakeTwo256::hash_of(parent_header), Error::<T>::InvalidParentHash);

	// Verify PoW (using enhanced Blake2 for better ASIC resistance)
	ensure!(verify_pow_enhanced(header, header.difficulty), Error::<T>::InvalidPow);

	// Check difficulty is reasonable
	let expected_difficulty = Difficulty::<T>::get();
	ensure!(header.difficulty >= expected_difficulty / 2, Error::<T>::DifficultyTooLow);
	ensure!(header.difficulty <= expected_difficulty * 2, Error::<T>::DifficultyTooHigh);

	Ok(())
}

/// Distribute rewards to miner and stakers
pub fn distribute_rewards<T: Config>(
	miner: T::AccountId,
	stakers: Vec<ValidatorStake<T::AccountId, BalanceOf<T>>>,
	reward: BlockReward<BalanceOf<T>>,
) -> DispatchResult {
	// Reward the miner
	pallet_balances::Pallet::<T>::mutate(&miner, |balance| {
		*balance += reward.miner_reward;
	});

	// Distribute to stakers proportionally
	let total_stake: BalanceOf<T> = stakers.iter().map(|s| s.stake).sum();
	if !total_stake.is_zero() {
		for staker in stakers {
			let staker_reward = (reward.stakers_reward * staker.stake) / total_stake;
			pallet_balances::Pallet::<T>::mutate(&staker.account, |balance| {
				*balance += staker_reward;
			});
		}
	}

	Ok(())
}

/// Check for slashing conditions
pub fn check_slashing_conditions<T: Config>(
	validator: T::AccountId,
) -> Option<SlashingReason> {
	// This function would need access to storage, so it's better implemented in the pallet
	// For now, return None
	None
}

/// Apply slashing
pub fn apply_slashing<T: Config>(
	validator: T::AccountId,
	reason: SlashingReason,
) -> DispatchResult {
	// This function should be implemented in the pallet where storage access is available
	// For now, just return Ok
	Ok(())
}
