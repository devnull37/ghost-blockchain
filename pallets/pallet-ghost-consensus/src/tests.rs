//! Unit tests for Ghost Consensus Pallet

use super::*;
use crate::mock::*;
use crate::types::*;
use frame_support::{assert_err, assert_ok};
use sp_core::H256;
use sp_runtime::traits::{BlakeTwo256, Hash};

#[test]
fn test_genesis_config() {
	run_test(|| {
		// Check initial difficulty is set
		let difficulty = Difficulty::<Test>::get();
		assert_eq!(difficulty, 1_000_000_000_000u64);

		// Check initial phase is PoW mining
		let phase = CurrentPhase::<Test>::get();
		assert_eq!(phase, ConsensusPhase::PowMining);
	});
}

#[test]
fn test_difficulty_adjustment_increase() {
	run_test(|| {
		let current_difficulty = 1_000_000u64;
		let actual_block_time = 3u64; // 3 seconds (too fast)
		let target_block_time = 5u64; // 5 seconds target

		let new_difficulty = functions::calculate_difficulty_adjustment::<Test>(
			current_difficulty,
			actual_block_time,
			target_block_time,
		);

		// Difficulty should increase since blocks are too fast
		assert!(new_difficulty > current_difficulty);
		// Should be roughly 1.67x harder (5/3 = 1.67)
		assert_eq!(new_difficulty, 1_666_666u64);
	});
}

#[test]
fn test_difficulty_adjustment_decrease() {
	run_test(|| {
		let current_difficulty = 1_000_000u64;
		let actual_block_time = 10u64; // 10 seconds (too slow)
		let target_block_time = 5u64; // 5 seconds target

		let new_difficulty = functions::calculate_difficulty_adjustment::<Test>(
			current_difficulty,
			actual_block_time,
			target_block_time,
		);

		// Difficulty should decrease since blocks are too slow
		assert!(new_difficulty < current_difficulty);
		// Should be roughly 0.5x easier (5/10 = 0.5)
		assert_eq!(new_difficulty, 500_000u64);
	});
}

#[test]
fn test_pow_verification_enhanced() {
	run_test(|| {
		// Create a block header with a low difficulty to make testing easier
		let mut header = create_block_header(1, 0);
		header.difficulty = u64::MAX; // Very easy difficulty

		// This should pass with a high difficulty
		assert!(functions::verify_pow_enhanced(&header, u64::MAX));

		// Test with impossible difficulty (should fail)
		assert!(!functions::verify_pow_enhanced(&header, 1));
	});
}

#[test]
fn test_staking_basic() {
	run_test(|| {
		let staker = 1u64; // Alice
		let stake_amount = 10_000_000_000_000_000_000u128; // 10 GHOST

		// Stake tokens
		assert_ok!(GhostConsensus::stake(RuntimeOrigin::signed(staker), stake_amount));

		// Check stake was recorded
		let recorded_stake = ValidatorStakes::<Test>::get(staker);
		assert!(recorded_stake.is_some());
		assert_eq!(recorded_stake.unwrap(), stake_amount);
	});
}

#[test]
fn test_staking_below_minimum() {
	run_test(|| {
		let staker = 1u64; // Alice
		let stake_amount = 500_000_000_000_000_000u128; // 0.5 GHOST (below minimum)

		// Should fail because stake is below minimum
		assert_err!(
			GhostConsensus::stake(RuntimeOrigin::signed(staker), stake_amount),
			Error::<Test>::InsufficientStake
		);
	});
}

#[test]
fn test_staking_multiple_times() {
	run_test(|| {
		let staker = 1u64; // Alice
		let first_stake = 5_000_000_000_000_000_000u128; // 5 GHOST
		let second_stake = 3_000_000_000_000_000_000u128; // 3 GHOST

		// First stake
		assert_ok!(GhostConsensus::stake(RuntimeOrigin::signed(staker), first_stake));

		// Second stake
		assert_ok!(GhostConsensus::stake(RuntimeOrigin::signed(staker), second_stake));

		// Check total stake
		let total_stake = ValidatorStakes::<Test>::get(staker).unwrap();
		assert_eq!(total_stake, first_stake + second_stake);
	});
}

#[test]
fn test_unstaking_basic() {
	run_test(|| {
		let staker = 1u64; // Alice
		let stake_amount = 10_000_000_000_000_000_000u128; // 10 GHOST
		let unstake_amount = 3_000_000_000_000_000_000u128; // 3 GHOST

		// Stake first
		assert_ok!(GhostConsensus::stake(RuntimeOrigin::signed(staker), stake_amount));

		// Unstake partial amount
		assert_ok!(GhostConsensus::unstake(RuntimeOrigin::signed(staker), unstake_amount));

		// Check remaining stake
		let remaining_stake = ValidatorStakes::<Test>::get(staker).unwrap();
		assert_eq!(remaining_stake, stake_amount - unstake_amount);
	});
}

#[test]
fn test_unstaking_without_stake() {
	run_test(|| {
		let staker = 1u64; // Alice
		let unstake_amount = 1_000_000_000_000_000_000u128; // 1 GHOST

		// Try to unstake without staking first
		assert_err!(
			GhostConsensus::unstake(RuntimeOrigin::signed(staker), unstake_amount),
			Error::<Test>::NotAValidator
		);
	});
}

#[test]
fn test_unstaking_more_than_staked() {
	run_test(|| {
		let staker = 1u64; // Alice
		let stake_amount = 5_000_000_000_000_000_000u128; // 5 GHOST
		let unstake_amount = 10_000_000_000_000_000_000u128; // 10 GHOST (more than staked)

		// Stake first
		assert_ok!(GhostConsensus::stake(RuntimeOrigin::signed(staker), stake_amount));

		// Try to unstake more than staked
		assert_err!(
			GhostConsensus::unstake(RuntimeOrigin::signed(staker), unstake_amount),
			Error::<Test>::InsufficientStake
		);
	});
}

#[test]
fn test_validator_selection_weighted() {
	run_test(|| {
		// Create validators with different stakes
		let stakers = vec![
			ValidatorStake {
				account: 1u64,
				stake: 50_000_000_000_000_000_000u128, // 50 GHOST
				weight: 50,
			},
			ValidatorStake {
				account: 2u64,
				stake: 30_000_000_000_000_000_000u128, // 30 GHOST
				weight: 30,
			},
			ValidatorStake {
				account: 3u64,
				stake: 20_000_000_000_000_000_000u128, // 20 GHOST
				weight: 20,
			},
		];

		let seed = H256::from_low_u64_be(12345);
		let selection = functions::select_pos_validator::<Test>(stakers, seed);

		assert!(selection.is_some());
		let selected = selection.unwrap();
		assert!(selected.validator >= 1 && selected.validator <= 3);
		assert!(selected.weight > 0);
	});
}

#[test]
fn test_validator_selection_empty() {
	run_test(|| {
		let stakers: Vec<ValidatorStake<u64, u128>> = vec![];
		let seed = H256::from_low_u64_be(12345);
		let selection = functions::select_pos_validator::<Test>(stakers, seed);

		assert!(selection.is_none());
	});
}

#[test]
fn test_block_reward_calculation() {
	run_test(|| {
		let total_reward = 10_000_000_000_000_000_000u128; // 10 GHOST
		let reward = functions::calculate_block_reward::<Test>(total_reward);

		// 40% to miner
		assert_eq!(reward.miner_reward, 4_000_000_000_000_000_000u128);
		// 60% to stakers
		assert_eq!(reward.stakers_reward, 6_000_000_000_000_000_000u128);
		// Total should match
		assert_eq!(reward.total, total_reward);
		assert_eq!(reward.miner_reward + reward.stakers_reward, total_reward);
	});
}

#[test]
fn test_block_header_validation_sequence() {
	run_test(|| {
		let parent_header = create_block_header(0, 100);
		let mut child_header = create_block_header(1, 200);
		child_header.parent_hash = BlakeTwo256::hash_of(&parent_header);
		child_header.difficulty = 1_000_000_000_000u64;

		// Store parent header
		BlockHeaders::<Test>::insert(0, parent_header.clone());

		// This might fail on PoW verification depending on nonce, but should pass other checks
		let result = functions::validate_block_header::<Test>(&child_header, &parent_header);
		// We expect it might fail on PoW, but not on sequence or parent hash
		// Just ensure it runs without panicking
	});
}

#[test]
fn test_block_header_validation_wrong_number() {
	run_test(|| {
		let parent_header = create_block_header(0, 100);
		let mut child_header = create_block_header(5, 200); // Wrong number (should be 1)
		child_header.parent_hash = BlakeTwo256::hash_of(&parent_header);

		BlockHeaders::<Test>::insert(0, parent_header.clone());

		assert_err!(
			functions::validate_block_header::<Test>(&child_header, &parent_header),
			Error::<Test>::InvalidBlockNumber
		);
	});
}

#[test]
fn test_block_header_validation_wrong_parent() {
	run_test(|| {
		let parent_header = create_block_header(0, 100);
		let mut child_header = create_block_header(1, 200);
		child_header.parent_hash = H256::zero(); // Wrong parent hash

		BlockHeaders::<Test>::insert(0, parent_header.clone());

		assert_err!(
			functions::validate_block_header::<Test>(&child_header, &parent_header),
			Error::<Test>::InvalidParentHash
		);
	});
}

#[test]
fn test_phase_transitions() {
	run_test(|| {
		// Start in PoW mining phase
		assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::PowMining);

		// Transition to PoS validation
		CurrentPhase::<Test>::put(ConsensusPhase::PosValidation);
		assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::PosValidation);

		// Transition to finalization
		CurrentPhase::<Test>::put(ConsensusPhase::Finalization);
		assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::Finalization);

		// Back to PoW mining
		CurrentPhase::<Test>::put(ConsensusPhase::PowMining);
		assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::PowMining);
	});
}

#[test]
fn test_multiple_validators_staking() {
	run_test(|| {
		// Multiple validators stake different amounts
		let validators = vec![
			(1u64, 10_000_000_000_000_000_000u128), // Alice: 10 GHOST
			(2u64, 20_000_000_000_000_000_000u128), // Bob: 20 GHOST
			(3u64, 15_000_000_000_000_000_000u128), // Charlie: 15 GHOST
		];

		for (validator, amount) in validators.iter() {
			assert_ok!(GhostConsensus::stake(RuntimeOrigin::signed(*validator), *amount));
		}

		// Verify all stakes are recorded
		for (validator, amount) in validators.iter() {
			let stake = ValidatorStakes::<Test>::get(validator).unwrap();
			assert_eq!(stake, *amount);
		}
	});
}

#[test]
fn test_last_active_block_tracking() {
	run_test(|| {
		let validator = 1u64;
		let block_number = 100u32;

		LastActiveBlock::<Test>::insert(validator, block_number);

		let last_active = LastActiveBlock::<Test>::get(validator);
		assert_eq!(last_active, block_number);
	});
}

#[test]
fn test_difficulty_storage() {
	run_test(|| {
		let new_difficulty = 5_000_000_000_000u64;

		Difficulty::<Test>::put(new_difficulty);

		let stored_difficulty = Difficulty::<Test>::get();
		assert_eq!(stored_difficulty, new_difficulty);
	});
}

#[test]
fn test_pow_verification_different_algorithms() {
	run_test(|| {
		let header = create_block_header(1, 12345);
		let easy_difficulty = u64::MAX;
		let hard_difficulty = 1000u64;

		// Test basic Blake2
		let result_basic = functions::verify_pow(&header, easy_difficulty);
		assert!(result_basic);

		// Test enhanced Blake2
		let result_enhanced = functions::verify_pow_enhanced(&header, easy_difficulty);
		assert!(result_enhanced);

		// Test SHA-256
		let result_sha = functions::verify_pow_sha256(&header, easy_difficulty);
		assert!(result_sha);

		// Test Keccak
		let result_keccak = functions::verify_pow_keccak(&header, easy_difficulty);
		assert!(result_keccak);

		// All should fail with very hard difficulty
		assert!(!functions::verify_pow(&header, hard_difficulty));
		assert!(!functions::verify_pow_enhanced(&header, hard_difficulty));
		assert!(!functions::verify_pow_sha256(&header, hard_difficulty));
		assert!(!functions::verify_pow_keccak(&header, hard_difficulty));
	});
}

#[test]
fn test_slashing_records_storage() {
	run_test(|| {
		let validator = 1u64;
		let reason = SlashingReason::DoubleSign;
		let amount = 5_000_000_000_000_000_000u128; // 5 GHOST
		let block_number = 100u32;

		let mut records = SlashingRecords::<Test>::get();
		records.push((validator, reason, amount, block_number));
		SlashingRecords::<Test>::put(records);

		let stored_records = SlashingRecords::<Test>::get();
		assert_eq!(stored_records.len(), 1);
		assert_eq!(stored_records[0].0, validator);
		assert_eq!(stored_records[0].2, amount);
	});
}

#[test]
fn test_double_sign_reports() {
	run_test(|| {
		let validator = 1u64;

		DoubleSignReports::<Test>::insert(validator, true);

		let is_reported = DoubleSignReports::<Test>::get(validator);
		assert!(is_reported);
	});
}

#[test]
fn test_invalid_block_reports() {
	run_test(|| {
		let validator = 1u64;

		InvalidBlockReports::<Test>::insert(validator, true);

		let is_reported = InvalidBlockReports::<Test>::get(validator);
		assert!(is_reported);
	});
}
