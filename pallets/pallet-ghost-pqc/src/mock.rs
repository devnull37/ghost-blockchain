//! Mock runtime for `pallet-ghost-pqc` tests.

use crate as pallet_ghost_pqc;
use frame_support::derive_impl;
use sp_runtime::BuildStorage;

type Block = frame_system::mocking::MockBlock<Test>;

frame_support::construct_runtime!(
    pub struct Test {
        System: frame_system,
        GhostPqc: pallet_ghost_pqc,
    }
);

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
    type Block = Block;
    type AccountId = sp_core::crypto::AccountId32;
    type Lookup = sp_runtime::traits::IdentityLookup<Self::AccountId>;
}

/// Mock bonded-validator lookup: everyone is bonded except `NOT_BONDED`,
/// letting tests cover both branches of the `pqc_attest` gate.
pub const NOT_BONDED: sp_core::crypto::AccountId32 = sp_core::crypto::AccountId32::new([0xEE; 32]);

pub struct TestBonded;
impl crate::BondedValidatorProvider<sp_core::crypto::AccountId32> for TestBonded {
    fn is_bonded_validator(who: &sp_core::crypto::AccountId32) -> bool {
        *who != NOT_BONDED
    }
}

impl pallet_ghost_pqc::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = ();
    type BondedValidators = TestBonded;
    #[cfg(feature = "runtime-benchmarks")]
    type BenchmarkHelper = ();
}

/// Build test externalities with an empty registry.
pub fn new_test_ext() -> sp_io::TestExternalities {
    let storage = frame_system::GenesisConfig::<Test>::default()
        .build_storage()
        .expect("mock genesis builds");
    storage.into()
}
