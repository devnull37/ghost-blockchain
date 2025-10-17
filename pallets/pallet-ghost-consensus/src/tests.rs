use crate::{mock::*, Error, Event, Pallet, GhostBlockHeader, SlashingReason, ValidatorStakes, TotalStake, BlockMiners, LastActiveBlock};
use frame_support::{assert_ok, assert_noop};
use sp_core::H256;

#[test]
fn test_stake_and_unstake() {
    new_test_ext().execute_with(|| {
        // Stake successfully
        assert_ok!(GhostConsensus::stake(RuntimeOrigin::signed(1), 100));
        assert_eq!(ValidatorStakes::<Test>::get(1), Some(100));
        assert_eq!(TotalStake::<Test>::get(), 100);

        // Unstake successfully
        assert_ok!(GhostConsensus::unstake(RuntimeOrigin::signed(1), 50));
        assert_eq!(ValidatorStakes::<Test>::get(1), Some(50));
        assert_eq!(TotalStake::<Test>::get(), 50);

        // Unstake more than available
        assert_noop!(
            GhostConsensus::unstake(RuntimeOrigin::signed(1), 100),
            Error::<Test>::InsufficientStake
        );
    });
}

#[test]
fn test_submit_block_and_rewards() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        // Create a dummy block header
        let header = GhostBlockHeader {
            number: 1,
            parent_hash: H256::default(),
            state_root: H256::default(),
            extrinsics_root: H256::default(),
            nonce: 0,
            difficulty: 0,
            validator_signature: None,
        };

        // Submit the block
        assert_ok!(GhostConsensus::submit_block(RuntimeOrigin::signed(1), header.clone()));
        assert_eq!(BlockMiners::<Test>::get(1), Some(1));

        // Stake some tokens to become a validator
        assert_ok!(GhostConsensus::stake(RuntimeOrigin::signed(2), 1000));

        // Validate the block
        assert_ok!(GhostConsensus::validate_block(RuntimeOrigin::signed(2), 1));

        // Check rewards
        let miner_balance = Balances::free_balance(1);
        let validator_balance = Balances::free_balance(2);

        // Block reward is 10, miner gets 4 (40%), validator gets 6 (60%)
        assert_eq!(miner_balance, 10004);
        assert_eq!(validator_balance, 9994);
    });
}

#[test]
fn test_slashing_for_downtime() {
    new_test_ext().execute_with(|| {
        // Stake to become a validator
        assert_ok!(GhostConsensus::stake(RuntimeOrigin::signed(1), 1000));

        // Set last active block to 0
        LastActiveBlock::<Test>::insert(1, 0);

        // Run to block 101 to trigger downtime slashing
        run_to_block(101);

        // Check if stake is slashed
        let stake = ValidatorStakes::<Test>::get(1).unwrap_or_default();
        assert!(stake < 1000);
    });
}

#[test]
fn test_report_misbehavior() {
    new_test_ext().execute_with(|| {
        // Stake to become a validator
        assert_ok!(GhostConsensus::stake(RuntimeOrigin::signed(2), 1000));

        // Report double signing
        assert_ok!(GhostConsensus::report_misbehavior(RuntimeOrigin::signed(1), 2, SlashingReason::DoubleSigning));

        // Check if stake is slashed
        let stake = ValidatorStakes::<Test>::get(2).unwrap_or_default();
        assert!(stake < 1000);
    });
}

fn run_to_block(n: u64) {
    while System::block_number() < n {
        if System::block_number() > 1 {
            System::on_finalize(System::block_number());
        }
        System::set_block_number(System::block_number() + 1);
        System::on_initialize(System::block_number());
    }
}

pub fn new_test_ext() -> sp_io::TestExternalities {
	let mut t = frame_system::GenesisConfig::default().build_storage::<Test>().unwrap();
	pallet_balances::GenesisConfig::<Test> {
		balances: vec![(1, 10000), (2, 10000)],
	}
	.assimilate_storage(&mut t)
	.unwrap();
	t.into()
}