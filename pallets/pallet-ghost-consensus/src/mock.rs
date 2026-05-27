//! Test mock for the Ghost consensus pallet.

use frame_support::{
    construct_runtime, derive_impl, parameter_types,
    traits::{ConstU128, ConstU64},
};
use pallet_balances;
use sp_core::H256;
use sp_runtime::{traits::BlakeTwo256, BuildStorage};

use crate as pallet_ghost_consensus;
use crate::types::{GenesisHeaderInit, GhostBlockHeader};

type Block = frame_system::mocking::MockBlock<Test>;

construct_runtime!(
    pub enum Test {
        System: frame_system,
        Timestamp: pallet_timestamp,
        Balances: pallet_balances,
        GhostConsensus: pallet_ghost_consensus,
    }
);

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type Block = Block;
    type Hash = H256;
    type Hashing = BlakeTwo256;
    type AccountId = u64;
    type AccountData = pallet_balances::AccountData<u128>;
}

#[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
impl pallet_balances::Config for Test {
    type Balance = u128;
    type AccountStore = System;
    type ExistentialDeposit = ConstU128<1>;
}

impl pallet_timestamp::Config for Test {
    type Moment = u64;
    type OnTimestampSet = ();
    type MinimumPeriod = ConstU64<1>;
    type WeightInfo = ();
}

parameter_types! {
    pub const BlockReward: u128 = 10_000_000_000_000_000_000;
    pub const TargetBlockTime: u64 = 5_000;
    pub const DifficultyAdjustmentPeriod: u32 = 20;
    pub const MinStake: u128 = 1_000_000_000_000_000_000;
    pub const MaxDowntimeBlocks: u32 = 100;
    pub const MaxValidationBlocks: u32 = 20;
    pub const MaxValidators: u32 = 4;
    pub const MaxSlashingRecords: u32 = 16;
    pub const DoubleSignSlashPercentage: u8 = 100;
    pub const InvalidBlockSlashPercentage: u8 = 50;
    pub const DowntimeSlashPercentage: u8 = 10;
    pub const GhostPalletId: frame_support::PalletId = frame_support::PalletId(*b"py/ghost");
}

impl pallet_ghost_consensus::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = ();
    type TargetBlockTime = TargetBlockTime;
    type DifficultyAdjustmentPeriod = DifficultyAdjustmentPeriod;
    type BlockReward = BlockReward;
    type MinStake = MinStake;
    type MaxDowntimeBlocks = MaxDowntimeBlocks;
    type MaxValidationBlocks = MaxValidationBlocks;
    type MaxValidators = MaxValidators;
    type MaxSlashingRecords = MaxSlashingRecords;
    type DoubleSignSlashPercentage = DoubleSignSlashPercentage;
    type InvalidBlockSlashPercentage = InvalidBlockSlashPercentage;
    type DowntimeSlashPercentage = DowntimeSlashPercentage;
    type PalletId = GhostPalletId;
}

pub struct ExtBuilder {
    balances: Vec<(u64, u128)>,
    genesis_header: Option<GenesisHeaderInit>,
    validator_stakes: Vec<(u64, u128)>,
}

impl Default for ExtBuilder {
    fn default() -> Self {
        Self {
            balances: vec![
                (1, 100_000_000_000_000_000_000),
                (2, 100_000_000_000_000_000_000),
                (3, 100_000_000_000_000_000_000),
                (4, 50_000_000_000_000_000_000),
                (5, 50_000_000_000_000_000_000),
                (6, 50_000_000_000_000_000_000),
            ],
            genesis_header: None,
            validator_stakes: Vec::new(),
        }
    }
}

impl ExtBuilder {
    pub fn with_balances(mut self, balances: Vec<(u64, u128)>) -> Self {
        self.balances = balances;
        self
    }

    pub fn with_genesis_header(mut self, header: GhostBlockHeader) -> Self {
        let header_init: GenesisHeaderInit = (
            header.number,
            header.parent_hash,
            header.state_root,
            header.extrinsics_root,
            header.nonce,
            header.difficulty,
            header.validator_signature,
        );
        self.genesis_header = Some(header_init);
        self
    }

    pub fn with_validator_stakes(mut self, validator_stakes: Vec<(u64, u128)>) -> Self {
        self.validator_stakes = validator_stakes;
        self
    }

    pub fn build(self) -> sp_io::TestExternalities {
        let mut storage = frame_system::GenesisConfig::<Test>::default()
            .build_storage()
            .unwrap();

        pallet_balances::GenesisConfig::<Test> {
            balances: self.balances,
        }
        .assimilate_storage(&mut storage)
        .unwrap();

        pallet_ghost_consensus::GenesisConfig::<Test> {
            genesis_header: self.genesis_header,
            validator_stakes: self.validator_stakes,
        }
        .assimilate_storage(&mut storage)
        .unwrap();

        let mut ext = sp_io::TestExternalities::new(storage);
        ext.execute_with(|| System::set_block_number(1));
        ext
    }
}

pub fn run_test<T>(test: impl FnOnce() -> T) -> T {
    ExtBuilder::default().build().execute_with(test)
}

pub fn create_block_header(
    number: u32,
    nonce: u64,
) -> pallet_ghost_consensus::types::GhostBlockHeader {
    use sp_runtime::traits::Hash;

    let parent_hash = if number == 0 {
        H256::zero()
    } else {
        BlakeTwo256::hash_of(&(number - 1, "parent"))
    };

    pallet_ghost_consensus::types::GhostBlockHeader {
        number,
        parent_hash,
        state_root: BlakeTwo256::hash_of(&(number, "state")),
        extrinsics_root: BlakeTwo256::hash_of(&(number, "extrinsics")),
        nonce,
        difficulty: 1_000_000_000_000,
        validator_signature: None,
    }
}
