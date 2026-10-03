//! Test mock runtime for the Ghost consensus pallet.
//!
//! Minimal runtime: system + balances + timestamp + this pallet. Session-key
//! and PQC-key lookups are backed by thread-local flags so tests can toggle
//! them without standing up `pallet-session`/`pallet-ghost-pqc`.

use crate as pallet_ghost_consensus;
use codec::Encode;
use frame_support::{
    derive_impl, parameter_types,
    traits::{ConstU128, ConstU32, ConstU64},
    BoundedVec, PalletId,
};
use sp_core::U256;
use sp_runtime::{traits::IdentityLookup, BuildStorage};
use std::{cell::RefCell, collections::BTreeSet};

pub type AccountId = u64;
pub type Balance = u128;
pub type Block = frame_system::mocking::MockBlock<Test>;

frame_support::construct_runtime!(
    pub enum Test {
        System: frame_system,
        Balances: pallet_balances,
        Timestamp: pallet_timestamp,
        GhostConsensus: pallet_ghost_consensus,
    }
);

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type BaseCallFilter = frame_support::traits::Everything;
    type BlockWeights = ();
    type BlockLength = ();
    type DbWeight = ();
    type RuntimeOrigin = RuntimeOrigin;
    type RuntimeCall = RuntimeCall;
    type Block = Block;
    type Hash = sp_core::H256;
    type AccountId = AccountId;
    type Lookup = IdentityLookup<AccountId>;
    type RuntimeEvent = RuntimeEvent;
    type AccountData = pallet_balances::AccountData<Balance>;
}

#[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
impl pallet_balances::Config for Test {
    type Balance = Balance;
    type DustRemoval = ();
    type RuntimeEvent = RuntimeEvent;
    type ExistentialDeposit = ConstU128<1>;
    type AccountStore = System;
    type WeightInfo = ();
    type MaxLocks = ConstU32<50>;
    type MaxReserves = ConstU32<50>;
    type ReserveIdentifier = [u8; 8];
    type FreezeIdentifier = ();
    type MaxFreezes = ConstU32<0>;
    type RuntimeHoldReason = RuntimeHoldReason;
    type RuntimeFreezeReason = RuntimeFreezeReason;
}

impl pallet_timestamp::Config for Test {
    type Moment = u64;
    type OnTimestampSet = ();
    type MinimumPeriod = ConstU64<5>;
    type WeightInfo = ();
}

// -- Switchable lookups for validate() gating ---------------------------------

thread_local! {
    static REGISTERED_KEYS: RefCell<BTreeSet<AccountId>> = RefCell::new(BTreeSet::new());
    static PQC_KEYS: RefCell<BTreeSet<AccountId>> = RefCell::new(BTreeSet::new());
    static REQUIRE_PQC: RefCell<bool> = const { RefCell::new(false) };
}

pub fn set_keys_registered(who: AccountId, registered: bool) {
    REGISTERED_KEYS.with(|s| {
        if registered {
            s.borrow_mut().insert(who);
        } else {
            s.borrow_mut().remove(&who);
        }
    });
}

pub fn set_pqc_key(who: AccountId, has: bool) {
    PQC_KEYS.with(|s| {
        if has {
            s.borrow_mut().insert(who);
        } else {
            s.borrow_mut().remove(&who);
        }
    });
}

pub fn set_require_pqc(require: bool) {
    REQUIRE_PQC.with(|r| *r.borrow_mut() = require);
}

pub struct MockSessionKeysLookup;
impl crate::SessionKeysLookup<AccountId> for MockSessionKeysLookup {
    fn keys_registered(who: &AccountId) -> bool {
        REGISTERED_KEYS.with(|s| s.borrow().contains(who))
    }
}

pub struct MockPqcProvider;
impl crate::PqcKeyProvider<AccountId> for MockPqcProvider {
    fn has_pqc_key(who: &AccountId) -> bool {
        PQC_KEYS.with(|s| s.borrow().contains(who))
    }
    fn pqc_key(who: &AccountId) -> Option<BoundedVec<u8, ConstU32<{ crate::MAX_PQC_KEY_SIZE }>>> {
        use frame_support::BoundedVec;
        PQC_KEYS.with(|s| {
            if s.borrow().contains(who) {
                Some(BoundedVec::truncate_from(vec![7u8; 32]))
            } else {
                None
            }
        })
    }
}

pub struct RequirePqcFlag;
impl frame_support::traits::Get<bool> for RequirePqcFlag {
    fn get() -> bool {
        REQUIRE_PQC.with(|r| *r.borrow())
    }
}

// -- Pallet config ------------------------------------------------------------

pub const MIN_STAKE: Balance = 10;
pub const MAX_STAKE: Balance = 1_000_000;
pub const UNBONDING_PERIOD: u64 = 5;
pub const BLOCK_REWARD: Balance = 100;
pub const RETARGET_INTERVAL: u64 = 100;
pub const TARGET_BLOCK_TIME_MS: u64 = 5_000;

parameter_types! {
    pub const GhostPalletId: PalletId = PalletId(*b"py/ghstc");
    pub MinDifficultyValue: U256 = U256::from(100u64);
}

impl pallet_ghost_consensus::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type Currency = Balances;
    type RuntimeHoldReason = RuntimeHoldReason;
    type WeightInfo = ();
    type PalletId = GhostPalletId;
    type BlockReward = ConstU128<BLOCK_REWARD>;
    type MinStake = ConstU128<MIN_STAKE>;
    type MaxStake = ConstU128<MAX_STAKE>;
    type MaxValidators = ConstU32<3>;
    type MaxValidatorCandidates = ConstU32<8>;
    type UnbondingPeriod = ConstU64<UNBONDING_PERIOD>;
    type MaxUnbondingChunks = ConstU32<4>;
    type RetargetInterval = ConstU32<{ RETARGET_INTERVAL as u32 }>;
    type TargetBlockTimeMs = ConstU64<TARGET_BLOCK_TIME_MS>;
    type MinDifficulty = MinDifficultyValue;
    type SessionKeysLookup = MockSessionKeysLookup;
    type PqcProvider = MockPqcProvider;
    type RequirePqcKey = RequirePqcFlag;
}

pub const ALICE: AccountId = 1;
pub const BOB: AccountId = 2;
pub const CHARLIE: AccountId = 3;
pub const DAVE: AccountId = 4;
pub const EVE: AccountId = 5;

/// Build test externalities: endowed accounts + initial difficulty.
pub fn new_test_ext() -> sp_io::TestExternalities {
    let mut storage = frame_system::GenesisConfig::<Test>::default()
        .build_storage()
        .unwrap();

    pallet_balances::GenesisConfig::<Test> {
        balances: vec![
            (ALICE, 10_000),
            (BOB, 10_000),
            (CHARLIE, 10_000),
            (DAVE, 10_000),
            (EVE, 10_000),
        ],
        ..Default::default()
    }
    .assimilate_storage(&mut storage)
    .unwrap();

    pallet_ghost_consensus::GenesisConfig::<Test> {
        difficulty: U256::from(1_000u64),
        stakers: vec![],
        initial_validators: vec![],
        ..Default::default()
    }
    .assimilate_storage(&mut storage)
    .unwrap();

    let mut ext = sp_io::TestExternalities::new(storage);
    ext.execute_with(|| {
        System::set_block_number(1);
        REGISTERED_KEYS.with(|s| s.borrow_mut().clear());
        PQC_KEYS.with(|s| s.borrow_mut().clear());
        REQUIRE_PQC.with(|r| *r.borrow_mut() = false);
    });
    ext
}

/// Advance to block `n`, running timestamp set + on_initialize/on_finalize so
/// hooks behave like a real chain. Each step moves time `delta_ms` per block.
pub fn run_to_block(n: u64, delta_ms: u64) {
    while System::block_number() < n {
        <GhostConsensus as frame_support::traits::Hooks<u64>>::on_finalize(System::block_number());
        System::set_block_number(System::block_number() + 1);
        Timestamp::set_timestamp(Timestamp::get().saturating_add(delta_ms));
        <GhostConsensus as frame_support::traits::Hooks<u64>>::on_initialize(System::block_number());
    }
}

/// Stamp a PoW author pre-runtime digest for `author` into the current block.
pub fn set_author(author: AccountId) {
    System::deposit_log(sp_runtime::DigestItem::PreRuntime(
        *b"pow_",
        author.encode(),
    ));
}
