//! Benchmark coverage for the Ghost consensus pallet.
//!
//! This module focuses on the hardened dispatchables that exist today and keeps
//! PQ readiness fixtures close at hand so future registry/attestation calls can
//! reuse the same setup without reshaping benchmark inputs.

use super::*;
use crate::functions::{
    validate_pq_proof_envelope, validate_pq_readiness_metadata, verify_pow_enhanced,
};
use crate::types::{
    DefaultPqProof, GhostBlockHeader, MisbehaviorEvidence, PqAlgorithm, PqProofKind,
    PqReadinessMetadata, SlashingReason,
};
use frame_benchmarking::{account, v2::*};
use frame_support::{assert_ok, BoundedVec};
use frame_system::RawOrigin;
use sp_core::H256;
use sp_runtime::traits::{BlakeTwo256, Hash, Saturating, Zero};

const SEED: u32 = 0;
const MAX_NONCE_SEARCH: u64 = 10_000;

fn repeated_min_stake<T: Config>(multiplier: u32) -> BalanceOf<T> {
    let mut total: BalanceOf<T> = Zero::zero();
    for _ in 0..multiplier {
        total = total.saturating_add(T::MinStake::get());
    }
    total
}

fn fund_account<T: Config>(account: &T::AccountId, amount: BalanceOf<T>) {
    let _ = pallet_balances::Pallet::<T>::deposit_creating(account, amount);
}

fn genesis_header() -> GhostBlockHeader {
    GhostBlockHeader {
        number: 0,
        parent_hash: H256::zero(),
        state_root: BlakeTwo256::hash_of(&(0u32, "state")),
        extrinsics_root: BlakeTwo256::hash_of(&(0u32, "extrinsics")),
        nonce: 0,
        difficulty: 1_000_000_000_000,
        validator_signature: None,
    }
}

fn mine_header<T: Config>(number: u32, parent: &GhostBlockHeader) -> GhostBlockHeader {
    // Conventional difficulty: higher = harder. 16 keeps the PoW search cheap.
    Difficulty::<T>::put(16);

    let mut header = GhostBlockHeader {
        number,
        parent_hash: BlakeTwo256::hash_of(parent),
        state_root: BlakeTwo256::hash_of(&(number, "state")),
        extrinsics_root: BlakeTwo256::hash_of(&(number, "extrinsics")),
        nonce: 0,
        difficulty: Difficulty::<T>::get(),
        validator_signature: None,
    };

    for nonce in 0..MAX_NONCE_SEARCH {
        header.nonce = nonce;
        if verify_pow_enhanced(&header, header.difficulty) {
            return header;
        }
    }

    panic!("failed to find benchmark nonce");
}

fn setup_validator<T: Config>(name: &'static str, index: u32) -> T::AccountId {
    let validator = account(name, index, SEED);
    let endowment = repeated_min_stake::<T>(8);
    let stake = repeated_min_stake::<T>(2);
    fund_account::<T>(&validator, endowment);
    assert_ok!(Pallet::<T>::stake(
        RawOrigin::Signed(validator.clone()).into(),
        stake
    ));
    validator
}

fn sample_pq_metadata<T: Config>() -> PqReadinessMetadata<BlockNumberFor<T>> {
    let now = frame_system::Pallet::<T>::block_number();

    PqReadinessMetadata {
        version: 1,
        algorithm: PqAlgorithm::MlDsa65,
        proof_kind: PqProofKind::Attestation,
        key_strength_bits: 192,
        claimed_nist_level: Some(3),
        issued_at: Some(now),
        expires_at: Some(now.saturating_add(10u32.into())),
        public_key_commitment: H256::repeat_byte(0xAB),
        metadata_hash: Some(H256::repeat_byte(0xCD)),
        flags: 0b0000_0011,
    }
}

fn sample_pq_proof<T: Config>() -> DefaultPqProof<BlockNumberFor<T>> {
    DefaultPqProof {
        algorithm: PqAlgorithm::MlDsa65,
        proof_kind: PqProofKind::Attestation,
        submitted_at: frame_system::Pallet::<T>::block_number().saturating_add(1u32.into()),
        statement_hash: H256::repeat_byte(0x11),
        public_key_commitment: H256::repeat_byte(0xAB),
        proof: BoundedVec::try_from(vec![7u8; 96]).expect("proof fits benchmark bound"),
        context: BoundedVec::try_from(b"ghost-pq-attestation".to_vec())
            .expect("context fits benchmark bound"),
        auxiliary_hash: Some(H256::repeat_byte(0x22)),
    }
}

#[benchmarks]
mod benchmarks {
    use super::*;

    #[benchmark]
    fn stake() {
        let caller: T::AccountId = account("staker", 0, SEED);
        let amount = repeated_min_stake::<T>(2);
        fund_account::<T>(&caller, repeated_min_stake::<T>(8));

        #[extrinsic_call]
        _(RawOrigin::Signed(caller.clone()), amount);

        assert_eq!(ValidatorStakes::<T>::get(&caller), Some(amount));
        assert_eq!(ValidatorCount::<T>::get(), 1);
    }

    #[benchmark]
    fn unstake() {
        let caller = setup_validator::<T>("staker", 1);
        let amount = repeated_min_stake::<T>(1);

        #[extrinsic_call]
        _(RawOrigin::Signed(caller.clone()), amount);

        assert_eq!(ValidatorStakes::<T>::get(&caller), Some(amount));
    }

    #[benchmark]
    fn submit_block() {
        let miner: T::AccountId = account("miner", 0, SEED);
        let parent = genesis_header();
        BlockHeaders::<T>::insert(0, parent.clone());
        let header = mine_header::<T>(1, &parent);

        #[extrinsic_call]
        _(RawOrigin::Signed(miner.clone()), header.clone());

        assert_eq!(CurrentPhase::<T>::get(), ConsensusPhase::PosValidation);
        assert_eq!(PendingValidationBlock::<T>::get(), Some(header.number));
        assert_eq!(BlockMiners::<T>::get(header.number), Some(miner));
    }

    #[benchmark]
    fn validate_block() {
        let miner: T::AccountId = account("miner", 1, SEED);
        let validator = setup_validator::<T>("validator", 0);
        let parent = genesis_header();
        BlockHeaders::<T>::insert(0, parent.clone());
        let header = mine_header::<T>(1, &parent);
        assert_ok!(Pallet::<T>::submit_block(
            RawOrigin::Signed(miner.clone()).into(),
            header
        ));

        #[extrinsic_call]
        _(RawOrigin::Signed(validator.clone()), 1u32, None);

        let stored_header = BlockHeaders::<T>::get(1).expect("validated block exists");
        assert!(stored_header.validator_signature.is_some());
        assert_eq!(CurrentPhase::<T>::get(), ConsensusPhase::Finalization);
    }

    #[benchmark]
    fn report_misbehavior() {
        let reporter: T::AccountId = account("reporter", 0, SEED);
        let validator = setup_validator::<T>("validator", 2);
        fund_account::<T>(&reporter, repeated_min_stake::<T>(2));

        #[extrinsic_call]
        _(RawOrigin::Signed(reporter), validator.clone(), SlashingReason::DoubleSigning, MisbehaviorEvidence::DoubleSigning {
            first_vote: H256::repeat_byte(0x01),
            second_vote: H256::repeat_byte(0x02),
        });

        assert!(DoubleSignReports::<T>::get(&validator));
        assert_eq!(SlashingRecords::<T>::get().len(), 1);
    }

    #[benchmark]
    fn register_pq_readiness() {
        let caller: T::AccountId = account("pq-account", 0, SEED);
        let metadata = sample_pq_metadata::<T>();

        #[extrinsic_call]
        _(RawOrigin::Signed(caller.clone()), metadata.clone());

        assert_eq!(PqReadinessRegistry::<T>::get(&caller), Some(metadata));
    }

    #[benchmark]
    fn attest_pq_readiness() {
        let caller: T::AccountId = account("pq-account", 1, SEED);
        let metadata = sample_pq_metadata::<T>();
        let proof = sample_pq_proof::<T>();

        assert_ok!(Pallet::<T>::register_pq_readiness(
            RawOrigin::Signed(caller.clone()).into(),
            metadata,
        ));

        #[extrinsic_call]
        _(RawOrigin::Signed(caller.clone()), proof.clone());

        assert_eq!(PqReadinessAttestations::<T>::get(&caller), Some(proof));
    }

    #[benchmark]
    fn remove_pq_readiness() {
        let caller: T::AccountId = account("pq-account", 2, SEED);
        let metadata = sample_pq_metadata::<T>();
        let proof = sample_pq_proof::<T>();

        assert_ok!(Pallet::<T>::register_pq_readiness(
            RawOrigin::Signed(caller.clone()).into(),
            metadata,
        ));
        assert_ok!(Pallet::<T>::attest_pq_readiness(
            RawOrigin::Signed(caller.clone()).into(),
            proof,
        ));

        #[extrinsic_call]
        _(RawOrigin::Signed(caller.clone()));

        assert!(!PqReadinessRegistry::<T>::contains_key(&caller));
        assert!(!PqReadinessAttestations::<T>::contains_key(&caller));
    }

    #[benchmark]
    fn pq_readiness_attestation_fixture() {
        let metadata = sample_pq_metadata::<T>();
        let proof = sample_pq_proof::<T>();

        #[block]
        {
            assert_ok!(validate_pq_readiness_metadata::<T>(&metadata));
            assert_ok!(validate_pq_proof_envelope::<T>(&metadata, &proof));
        }
    }
}
