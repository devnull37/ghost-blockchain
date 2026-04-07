//! Pure helper functions used by the Ghost consensus pallet.

use super::*;
use crate::types::{BlockReward, GhostBlockHeader, PosSelection, ValidatorStake};
use frame_support::pallet_prelude::*;
use sp_core::H256;
use sp_runtime::traits::{BlakeTwo256, Hash, SaturatedConversion, Zero};

/// Calculate a simple difficulty adjustment towards the target block time.
pub fn calculate_difficulty_adjustment<T: Config>(
	current_difficulty: u64,
	actual_block_time: u64,
	target_block_time: u64,
) -> u64 {
	if actual_block_time == 0 {
		return current_difficulty;
	}

	current_difficulty
		.saturating_mul(target_block_time)
		.checked_div(actual_block_time)
		.unwrap_or(current_difficulty)
}

/// Verify Proof-of-Work using a single Blake2-256 hash.
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

/// Verify Proof-of-Work using a double Blake2-256 hash.
pub fn verify_pow_enhanced(block_header: &GhostBlockHeader, target_difficulty: u64) -> bool {
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

/// Verify Proof-of-Work using double SHA-256.
pub fn verify_pow_sha256(block_header: &GhostBlockHeader, target_difficulty: u64) -> bool {
	use sp_core::sha2_256;

	let hash_input = (
		block_header.number,
		block_header.parent_hash,
		block_header.state_root,
		block_header.extrinsics_root,
		block_header.nonce,
	);

	let first_hash = sha2_256(&hash_input.encode());
	let final_hash = sha2_256(&first_hash);
	let hash_value = u64::from_be_bytes(final_hash[0..8].try_into().unwrap_or_default());

	hash_value <= target_difficulty
}

/// Verify Proof-of-Work using Keccak-256.
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

/// Select a validator using stake-weighted sampling.
pub fn select_pos_validator<T: Config>(
	stakers: Vec<ValidatorStake<T::AccountId, BalanceOf<T>>>,
	seed: H256,
) -> Option<PosSelection<T::AccountId>> {
	if stakers.is_empty() {
		return None;
	}

	let total_weight: u64 = stakers.iter().map(|stake| stake.weight).sum();
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

/// Split a block reward 40/60 between miner and stakers.
pub fn calculate_block_reward<T: Config>(total_reward: BalanceOf<T>) -> BlockReward<BalanceOf<T>> {
	let miner_reward = (total_reward * 40u32.into()) / 100u32.into();
	let stakers_reward = total_reward - miner_reward;

	BlockReward { total: total_reward, miner_reward, stakers_reward }
}

/// Validate a submitted Ghost block header against its parent.
pub fn validate_block_header<T: Config>(
	header: &GhostBlockHeader,
	parent_header: &GhostBlockHeader,
) -> DispatchResult {
	ensure!(header.number == parent_header.number + 1, Error::<T>::InvalidBlockNumber);
	ensure!(header.parent_hash == BlakeTwo256::hash_of(parent_header), Error::<T>::InvalidParentHash);
	ensure!(verify_pow_enhanced(header, header.difficulty), Error::<T>::InvalidPow);

	let expected_difficulty = Difficulty::<T>::get();
	ensure!(header.difficulty >= expected_difficulty / 2, Error::<T>::DifficultyTooLow);
	ensure!(header.difficulty <= expected_difficulty.saturating_mul(2), Error::<T>::DifficultyTooHigh);

	Ok(())
}

/// Distribute rewards to the miner and all current stakers.
pub fn distribute_rewards<T: Config>(
	miner: T::AccountId,
	stakers: Vec<ValidatorStake<T::AccountId, BalanceOf<T>>>,
	reward: BlockReward<BalanceOf<T>>,
) -> DispatchResult {
	let _ = pallet_balances::Pallet::<T>::deposit_creating(&miner, reward.miner_reward);

	let total_stake: BalanceOf<T> =
		stakers.iter().fold(Zero::zero(), |acc, stake| acc + stake.stake);
	if !total_stake.is_zero() {
		for staker in stakers {
			let staker_reward = (reward.stakers_reward * staker.stake) / total_stake;
			let _ = pallet_balances::Pallet::<T>::deposit_creating(&staker.account, staker_reward);
		}
	}

	Ok(())
}
