//! Unit tests for the Ghost consensus pallet.

use super::*;
use crate::{mock::*, types::*};
use frame_support::{assert_err, assert_ok, traits::Hooks};
use sp_runtime::traits::{BlakeTwo256, Hash};

fn insert_genesis_header() -> GhostBlockHeader {
	let header = create_block_header(0, 0);
	BlockHeaders::<Test>::insert(0, header.clone());
	header
}

fn mine_test_header(number: u32, parent: &GhostBlockHeader) -> GhostBlockHeader {
	Difficulty::<Test>::put(u64::MAX / 2);
	let mut header = create_block_header(number, 0);
	header.parent_hash = BlakeTwo256::hash_of(parent);
	header.difficulty = Difficulty::<Test>::get();

	for nonce in 0..10_000 {
		header.nonce = nonce;
		if functions::verify_pow_enhanced(&header, header.difficulty) {
			return header;
		}
	}

	panic!("failed to find a valid nonce for test block");
}

#[test]
fn genesis_config_is_initialized() {
	run_test(|| {
		assert_eq!(Difficulty::<Test>::get(), 1_000_000_000_000u64);
		assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::PowMining);
	});
}

#[test]
fn difficulty_adjustment_moves_with_block_time() {
	run_test(|| {
		let increased =
			functions::calculate_difficulty_adjustment::<Test>(1_000_000, 3, 5);
		let decreased =
			functions::calculate_difficulty_adjustment::<Test>(1_000_000, 10, 5);

		assert!(increased > 1_000_000);
		assert!(decreased < 1_000_000);
	});
}

#[test]
fn stake_and_unstake_move_balances() {
	run_test(|| {
		let staker = 1u64;
		let amount = 10_000_000_000_000_000_000u128;
		let starting_balance = Balances::free_balance(staker);

		assert_ok!(GhostConsensus::stake(RuntimeOrigin::signed(staker), amount));
		assert_eq!(ValidatorStakes::<Test>::get(staker), Some(amount));
		assert_eq!(Balances::free_balance(staker), starting_balance - amount);
		assert_eq!(Balances::free_balance(GhostConsensus::account_id()), amount);

		assert_ok!(GhostConsensus::unstake(RuntimeOrigin::signed(staker), amount / 2));
		assert_eq!(ValidatorStakes::<Test>::get(staker), Some(amount / 2));
		assert_eq!(Balances::free_balance(GhostConsensus::account_id()), amount / 2);
	});
}

#[test]
fn stake_below_minimum_fails() {
	run_test(|| {
		assert_err!(
			GhostConsensus::stake(RuntimeOrigin::signed(1), 500_000_000_000_000_000u128),
			Error::<Test>::InsufficientStake
		);
	});
}

#[test]
fn validator_selection_returns_weighted_candidate() {
	run_test(|| {
		let stakers = vec![
			ValidatorStake { account: 1u64, stake: 50, weight: 50 },
			ValidatorStake { account: 2u64, stake: 30, weight: 30 },
			ValidatorStake { account: 3u64, stake: 20, weight: 20 },
		];
		let seed = sp_core::H256::from_low_u64_be(12345);
		let selected = functions::select_pos_validator::<Test>(stakers, seed).unwrap();

		assert!((1..=3).contains(&selected.validator));
		assert!(selected.weight > 0);
	});
}

#[test]
fn submit_block_transitions_to_pos_validation() {
	run_test(|| {
		let genesis = insert_genesis_header();
		let header = mine_test_header(1, &genesis);

		assert_ok!(GhostConsensus::submit_block(RuntimeOrigin::signed(1), header.clone()));
		assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::PosValidation);
		assert_eq!(BlockHeaders::<Test>::get(1), Some(header));
		assert_eq!(BlockMiners::<Test>::get(1), Some(1));
	});
}

#[test]
fn validate_block_selects_validator_and_distributes_rewards() {
	run_test(|| {
		let genesis = insert_genesis_header();
		let header = mine_test_header(1, &genesis);
		let miner = 1u64;
		let validator = 2u64;
		let reward = <Test as Config>::BlockReward::get();
		let reward_split = functions::calculate_block_reward::<Test>(reward);

		assert_ok!(GhostConsensus::stake(
			RuntimeOrigin::signed(validator),
			20_000_000_000_000_000_000u128
		));
		let miner_before = Balances::free_balance(miner);
		let validator_before = Balances::free_balance(validator);

		assert_ok!(GhostConsensus::submit_block(RuntimeOrigin::signed(miner), header));
		assert_ok!(GhostConsensus::validate_block(
			RuntimeOrigin::signed(validator),
			1
		));

		let stored_header = BlockHeaders::<Test>::get(1).unwrap();
		assert!(stored_header.validator_signature.is_some());
		assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::Finalization);
		assert_eq!(
			Balances::free_balance(miner),
			miner_before + reward_split.miner_reward
		);
		assert_eq!(
			Balances::free_balance(validator),
			validator_before + reward_split.stakers_reward
		);
	});
}

#[test]
fn report_misbehavior_slashes_stake_and_records_reason() {
	run_test(|| {
		let validator = 1u64;
		let amount = 10_000_000_000_000_000_000u128;
		assert_ok!(GhostConsensus::stake(RuntimeOrigin::signed(validator), amount));

		assert_ok!(GhostConsensus::report_misbehavior(
			RuntimeOrigin::signed(2),
			validator,
			SlashingReason::InvalidBlock,
		));

		let expected_remaining = amount - ((amount * 50u128) / 100u128);
		assert_eq!(ValidatorStakes::<Test>::get(validator), Some(expected_remaining));
		assert!(InvalidBlockReports::<Test>::get(validator));
		assert_eq!(SlashingRecords::<Test>::get().len(), 1);
	});
}

#[test]
fn on_finalize_resets_phase_back_to_pow() {
	run_test(|| {
		CurrentPhase::<Test>::put(ConsensusPhase::Finalization);
		GhostConsensus::on_finalize(1);
		assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::PowMining);
	});
}
