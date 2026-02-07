//! Test mock for Ghost Consensus Pallet

use frame_support::{
	construct_runtime, derive_impl, parameter_types,
	traits::{ConstU128, ConstU32, ConstU64, ConstU8},
};
use frame_system as system;
use pallet_balances;
use sp_core::H256;
use sp_runtime::{
	traits::{BlakeTwo256, IdentityLookup},
	BuildStorage,
};

use crate as pallet_ghost_consensus;

type Block = frame_system::mocking::MockBlock<Test>;

frame_support::construct_runtime!(
	pub enum Test {
		System: frame_system,
		Balances: pallet_balances,
		GhostConsensus: pallet_ghost_consensus,
	}
);

#[derive_impl(frame_system::config_with_system_defaults as frame_system::DefaultConfig)]
impl frame_system::Config for Test {
	type Block = Block;
	type BaseCallFilter = frame_support::traits::Everything;
	type BlockWeights = ();
	type BlockLength = ();
	type DbWeight = ();
	type RuntimeOrigin = RuntimeOrigin;
	type RuntimeCall = RuntimeCall;
	type RuntimeEvent = RuntimeEvent;
	type Hash = H256;
	type Hashing = BlakeTwo256;
	type AccountId = u64;
	type Lookup = IdentityLookup<Self::AccountId>;
	type MaxConsumers = ConstU32<16>;
}

#[derive_impl(pallet_balances::config_with_system_defaults as pallet_balances::DefaultConfig)]
impl pallet_balances::Config for Test {
	type MaxLocks = ConstU32<50>;
	type MaxReserves = ConstU32<50>;
	type ReserveIdentifier = [u8; 8];
}

parameter_types! {
	pub const BlockReward: u128 = 10_000_000_000_000_000_000; // 10 GHOST tokens
	pub const MinStake: u128 = 1_000_000_000_000_000_000; // 1 GHOST token
	pub const MaxDowntimeBlocks: u32 = 100;
	pub const DoubleSignSlashPercentage: u8 = 100; // 100% slash for double signing
	pub const InvalidBlockSlashPercentage: u8 = 50; // 50% slash for invalid block
	pub const DowntimeSlashPercentage: u8 = 10; // 10% slash for downtime
	pub const GhostPalletId: frame_support::PalletId = frame_support::PalletId(*b"py/ghost");
}

impl pallet_ghost_consensus::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = ();
	type BlockReward = BlockReward;
	type MinStake = MinStake;
	type MaxDowntimeBlocks = MaxDowntimeBlocks;
	type DoubleSignSlashPercentage = DoubleSignSlashPercentage;
	type InvalidBlockSlashPercentage = InvalidBlockSlashPercentage;
	type DowntimeSlashPercentage = DowntimeSlashPercentage;
	type PalletId = GhostPalletId;
}

pub struct ExtBuilder;

impl Default for ExtBuilder {
	fn default() -> Self {
		ExtBuilder
	}
}

impl ExtBuilder {
	pub fn build(self) -> sp_io::TestExternalities {
		let mut t = frame_system::GenesisConfig::<Test>::default()
			.build_storage()
			.unwrap();

		pallet_balances::GenesisConfig::<Test> {
			balances: vec![
				(1, 100_000_000_000_000_000_000), // Alice: 100 GHOST
				(2, 100_000_000_000_000_000_000), // Bob: 100 GHOST
				(3, 100_000_000_000_000_000_000), // Charlie: 100 GHOST
				(4, 50_000_000_000_000_000_000),  // Dave: 50 GHOST
				(5, 25_000_000_000_000_000_000),  // Eve: 25 GHOST
			],
		}
		.assimilate_storage(&mut t)
		.unwrap();

		pallet_ghost_consensus::Pallet::<Test>::on_initialize(0);

		let mut ext = sp_io::TestExternalities::new(t);
		ext.execute_with(|| {
			// Initialize storage values for tests
			pallet_ghost_consensus::Difficulty::<Test>::put(1_000_000_000_000u64); // Initial difficulty
			pallet_ghost_consensus::CurrentPhase::<Test>::put(pallet_ghost_consensus::types::ConsensusPhase::PowMining);
		});
		ext
	}
}

/// Helper function to run a test with the ExtBuilder
pub fn run_test<T>(f: impl FnOnce() -> T) -> T {
	ExtBuilder::default().build().execute_with(f)
}

/// Helper to create a valid block header for testing
pub fn create_block_header(number: u32, nonce: u64) -> pallet_ghost_consensus::types::GhostBlockHeader {
	use sp_runtime::traits::Hash;

	let parent_hash = if number == 0 {
		H256::zero()
	} else {
		BlakeTwo256::hash_of(&(number - 1))
	};

	pallet_ghost_consensus::types::GhostBlockHeader {
		number,
		parent_hash,
		state_root: BlakeTwo256::hash_of(&(number, "state")),
		extrinsics_root: BlakeTwo256::hash_of(&(number, "extrinsics")),
		nonce,
		difficulty: 1_000_000_000_000u64,
		validator_signature: None,
	}
}
