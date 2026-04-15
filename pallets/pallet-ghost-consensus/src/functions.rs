//! Core functions for Ghost Consensus

use super::*;
use crate::types::*;
use frame_support::{pallet_prelude::*, traits::Currency};
use sp_core::H256;
use sp_runtime::sp_std::{collections::btree_map::BTreeMap, vec::Vec};
use sp_runtime::traits::{BlakeTwo256, Hash, SaturatedConversion, Zero};

/// Calculate mining difficulty adjustment
pub fn calculate_difficulty_adjustment<T: Config>(
    current_difficulty: u64,
    actual_block_time: u64,
    target_block_time: u64,
    entropy: u64,
) -> u64 {
    if actual_block_time == 0 || target_block_time == 0 {
        return current_difficulty.saturating_div(2).max(1);
    }

    // `difficulty` is the maximum accepted hash prefix. Lower values are harder.
    let mut new_target = if actual_block_time < target_block_time {
        // Blocks are too fast; lower the target to make PoW harder.
        current_difficulty.saturating_mul(actual_block_time) / target_block_time
    } else {
        // Blocks are too slow; raise the target to make PoW easier.
        current_difficulty.saturating_mul(actual_block_time) / target_block_time
    };
    new_target = new_target.max(1);

    // Entropy Steering:
    // Maximum entropy for N items is log2(N).
    // For N=100, max entropy is ~6.64, scaled: 6,640,000.
    // If entropy is low (centralization), lower the target further.
    // We'll use 4,000,000 (log2(16)) as a "healthy decentralization" threshold.
    let entropy_threshold = 4_000_000;
    if entropy < entropy_threshold {
        let entropy_factor = entropy_threshold.saturating_add(entropy);
        new_target =
            new_target.saturating_mul(entropy_factor) / entropy_threshold.saturating_mul(2);
        new_target = new_target.max(1);
    }

    new_target
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
    ensure!(
        header.number == parent_header.number + 1,
        Error::<T>::InvalidBlockNumber
    );

    // Check parent hash
    ensure!(
        header.parent_hash == BlakeTwo256::hash_of(parent_header),
        Error::<T>::InvalidParentHash
    );

    // Verify PoW (using enhanced Blake2 for better ASIC resistance)
    ensure!(
        verify_pow_enhanced(header, header.difficulty),
        Error::<T>::InvalidPow
    );

    // Check difficulty is reasonable
    let expected_difficulty = Difficulty::<T>::get();
    ensure!(
        header.difficulty >= expected_difficulty.saturating_div(2),
        Error::<T>::DifficultyTooLow
    );
    ensure!(
        header.difficulty <= expected_difficulty.saturating_mul(2),
        Error::<T>::DifficultyTooHigh
    );

    Ok(())
}

/// Distribute rewards to miner and stakers
pub fn distribute_rewards<T: Config>(
    miner: T::AccountId,
    stakers: Vec<ValidatorStake<T::AccountId, BalanceOf<T>>>,
    reward: BlockReward<BalanceOf<T>>,
) -> DispatchResult {
    // Reward the miner
    let _ = pallet_balances::Pallet::<T>::deposit_creating(&miner, reward.miner_reward);

    // Distribute to stakers proportionally
    let total_stake: BalanceOf<T> = stakers.iter().fold(Zero::zero(), |acc, s| acc + s.stake);
    if !total_stake.is_zero() {
        for staker in stakers {
            let staker_reward = (reward.stakers_reward * staker.stake) / total_stake;
            let _ = pallet_balances::Pallet::<T>::deposit_creating(&staker.account, staker_reward);
        }
    }

    Ok(())
}

/// Calculate Shannon entropy for a list of accounts.
/// Used to measure centralization in the block production process.
/// Returns entropy scaled by 1,000,000.
///
/// H = -sum(pi * log2(pi))
///
/// We use scaled integers to avoid floats.
/// p_i = count_i / total_count
/// log2(p_i) = log2(count_i) - log2(total_count)
pub fn calculate_entropy<T: Config>(producers: Vec<T::AccountId>) -> u64 {
    let total_count = producers.len() as u64;
    if total_count == 0 {
        return 0;
    }

    // Count occurrences of each producer
    let mut counts = BTreeMap::new();
    for producer in producers {
        *counts.entry(producer).or_insert(0u64) += 1;
    }

    let mut total_entropy: u64 = 0;
    let log2_total = integer_log2_scaled(total_count);

    for count in counts.values() {
        // p_i = count / total_count
        // Entropy contribution = -(p_i * log2(p_i))
        // In scaled terms: - (count / total_count) * (log2(count) - log2(total_count))
        // => (count * (log2(total_count) - log2(count))) / total_count

        let log2_count = integer_log2_scaled(*count);
        let term = (*count * (log2_total.saturating_sub(log2_count))) / total_count;
        total_entropy = total_entropy.saturating_add(term);
    }

    total_entropy
}

/// Deterministic integer-based log2 scaled by 1,000,000.
/// Uses the property: log2(x) = log2(x * 2^k) - k
/// For higher precision, we scale the input.
fn integer_log2_scaled(x: u64) -> u64 {
    if x == 0 {
        return 0;
    }

    let scaling_factor = 1_000_000;

    // log2(x) * 1,000,000
    // We use the leading zeros to find the integer part.
    let leading_zeros = x.leading_zeros();
    let integer_part = (63 - leading_zeros) as u64;

    // Fractional part approximation: log2(x) approx integer_part + (x / 2^integer_part) - 1
    // More accurately using bit shifts for (x / 2^integer_part):
    let rem = x ^ (1 << integer_part);
    let fractional_part = (rem * scaling_factor) / (1 << integer_part);

    (integer_part * scaling_factor) + fractional_part
}

/// Check for slashing conditions
pub fn check_slashing_conditions<T: Config>(_validator: T::AccountId) -> Option<SlashingReason> {
    // This function would need access to storage, so it's better implemented in the pallet
    // For now, return None
    None
}

/// Apply slashing
pub fn apply_slashing<T: Config>(
    _validator: T::AccountId,
    _reason: SlashingReason,
) -> DispatchResult {
    // This function should be implemented in the pallet where storage access is available
    // For now, just return Ok
    Ok(())
}

/// Verify PQC Dilithium signature
pub fn verify_pqc_signature(
    message: &[u8],
    signature_bytes: &[u8],
    public_key_bytes: &[u8],
) -> bool {
    #[cfg(feature = "std")]
    {
        use pqcrypto_dilithium::dilithium5::{
            verify_detached_signature, DetachedSignature, PublicKey,
        };
        use pqcrypto_traits::sign::{DetachedSignature as _, PublicKey as _};

        if let (Ok(sig), Ok(pk)) = (
            DetachedSignature::from_bytes(signature_bytes),
            PublicKey::from_bytes(public_key_bytes),
        ) {
            return verify_detached_signature(&sig, message, &pk).is_ok();
        }
    }

    // For no_std or if parsing fails, we use a placeholder that would fail validation
    // In a real production deployment, a no_std compatible PQC library would be used.
    false
}
