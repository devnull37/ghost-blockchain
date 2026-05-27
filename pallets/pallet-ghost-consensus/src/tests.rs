//! Unit tests for the Ghost consensus pallet.

use super::*;
use crate::{mock::*, types::*};
use codec::{Decode, Encode};
use frame_support::{assert_err, assert_ok, traits::Hooks, BoundedVec};
use sp_runtime::traits::{BlakeTwo256, Hash};

fn insert_genesis_header() -> GhostBlockHeader {
    let header = create_block_header(0, 0);
    BlockHeaders::<Test>::insert(0, header.clone());
    header
}

fn mine_test_header(number: u32, parent: &GhostBlockHeader) -> GhostBlockHeader {
    // Conventional difficulty: higher = harder. 16 means target = U256::MAX/16
    // (~1 in 16 hashes pass), so a valid nonce is found quickly and deterministically.
    Difficulty::<Test>::put(16);
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

fn sample_pq_metadata() -> PqReadinessMetadata<u64> {
    PqReadinessMetadata {
        version: 2,
        algorithm: PqAlgorithm::MlDsa65,
        proof_kind: PqProofKind::Attestation,
        key_strength_bits: 192,
        claimed_nist_level: Some(3),
        issued_at: Some(12),
        expires_at: Some(144),
        public_key_commitment: sp_core::H256::repeat_byte(0xAB),
        metadata_hash: Some(sp_core::H256::repeat_byte(0xCD)),
        flags: 0b1010_0101,
    }
}

fn sample_pq_proof() -> DefaultPqProof<u64> {
    DefaultPqProof::<u64> {
        algorithm: PqAlgorithm::MlDsa65,
        proof_kind: PqProofKind::Attestation,
        submitted_at: 42,
        statement_hash: sp_core::H256::repeat_byte(0x11),
        public_key_commitment: sp_core::H256::repeat_byte(0xAB),
        proof: vec![7u8; 128].try_into().expect("proof fits in bound"),
        context: b"validator-2 attests readiness"
            .to_vec()
            .try_into()
            .expect("context fits in bound"),
        auxiliary_hash: Some(sp_core::H256::repeat_byte(0x33)),
    }
}

#[test]
fn genesis_config_is_initialized() {
    run_test(|| {
        assert_eq!(Difficulty::<Test>::get(), 100_000u64);
        assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::PowMining);
        assert_eq!(ValidatorCount::<Test>::get(), 0);
    });
}

#[test]
fn difficulty_adjustment_moves_with_block_time() {
    run_test(|| {
        let increased = functions::calculate_difficulty_adjustment::<Test>(1_000_000, 3, 5);
        let decreased = functions::calculate_difficulty_adjustment::<Test>(1_000_000, 10, 5);

        assert!(increased > 1_000_000);
        assert!(decreased < 1_000_000);
    });
}

#[test]
fn genesis_bootstrap_caps_and_filters_validators() {
    let genesis_header = create_block_header(0, 7);
    ExtBuilder::default()
        .with_genesis_header(genesis_header.clone())
        .with_validator_stakes(vec![
            (1, MinStake::get()),
            (2, MinStake::get() * 2),
            (3, MinStake::get() - 1),
            (4, MinStake::get()),
            (5, MinStake::get()),
            (6, MinStake::get()),
        ])
        .build()
        .execute_with(|| {
            assert_eq!(BlockHeaders::<Test>::get(0), Some(genesis_header));
            assert_eq!(ValidatorCount::<Test>::get(), 3);
            assert_eq!(ValidatorStakes::<Test>::get(1), Some(MinStake::get()));
            assert_eq!(ValidatorStakes::<Test>::get(2), Some(MinStake::get() * 2));
            assert_eq!(ValidatorStakes::<Test>::get(3), None);
            assert_eq!(ValidatorStakes::<Test>::get(4), Some(MinStake::get()));
            assert_eq!(ValidatorStakes::<Test>::get(5), None);
            assert_eq!(ValidatorStakes::<Test>::get(6), None);
            // Genesis now backs each accepted validator stake with real tokens in the
            // pallet account (validators 1, 2, 4 => MinStake + 2*MinStake + MinStake).
            assert_eq!(
                Balances::free_balance(GhostConsensus::account_id()),
                MinStake::get() * 4
            );
            assert_eq!(LastActiveBlock::<Test>::get(1), 0);
            assert_eq!(LastActiveBlock::<Test>::get(5), 0);
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

        assert_ok!(GhostConsensus::unstake(
            RuntimeOrigin::signed(staker),
            amount / 2
        ));
        assert_eq!(ValidatorStakes::<Test>::get(staker), Some(amount / 2));
        assert_eq!(
            Balances::free_balance(GhostConsensus::account_id()),
            amount / 2
        );
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
            ValidatorStake {
                account: 1u64,
                stake: 50,
                weight: 50,
            },
            ValidatorStake {
                account: 2u64,
                stake: 30,
                weight: 30,
            },
            ValidatorStake {
                account: 3u64,
                stake: 20,
                weight: 20,
            },
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
        // A staker must exist for there to be a validator to select; with an empty
        // staker set the block is finalized immediately instead (see the test below).
        assert_ok!(GhostConsensus::stake(
            RuntimeOrigin::signed(2),
            20_000_000_000_000_000_000u128
        ));
        let genesis = insert_genesis_header();
        let header = mine_test_header(1, &genesis);

        assert_ok!(GhostConsensus::submit_block(
            RuntimeOrigin::signed(1),
            header.clone()
        ));
        assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::PosValidation);
        assert_eq!(BlockHeaders::<Test>::get(1), Some(header));
        assert_eq!(BlockMiners::<Test>::get(1), Some(1));
    });
}

#[test]
fn submit_block_with_no_stakers_finalizes_immediately() {
    run_test(|| {
        let genesis = insert_genesis_header();
        let header = mine_test_header(1, &genesis);
        let miner = 1u64;
        let reward = <Test as Config>::BlockReward::get();
        let miner_before = Balances::free_balance(miner);

        // No stakers -> no validator to select. The block is finalized immediately,
        // the miner receives the FULL reward, and the chain stays in PowMining rather
        // than stalling in PosValidation until the validation timeout fires.
        assert_ok!(GhostConsensus::submit_block(
            RuntimeOrigin::signed(miner),
            header
        ));
        assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::PowMining);
        assert_eq!(PendingValidationBlock::<Test>::get(), None);
        assert_eq!(PendingValidator::<Test>::get(), None);
        assert_eq!(Balances::free_balance(miner), miner_before + reward);

        // A second block can be submitted right away: no stall, no timeout required.
        let stored = BlockHeaders::<Test>::get(1).unwrap();
        let header2 = mine_test_header(2, &stored);
        assert_ok!(GhostConsensus::submit_block(
            RuntimeOrigin::signed(miner),
            header2
        ));
        assert_eq!(BlockMiners::<Test>::get(2), Some(miner));
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

        assert_ok!(GhostConsensus::submit_block(
            RuntimeOrigin::signed(miner),
            header
        ));
        assert_ok!(GhostConsensus::validate_block(
            RuntimeOrigin::signed(validator),
            1,
            None
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
        assert_ok!(GhostConsensus::stake(
            RuntimeOrigin::signed(validator),
            amount
        ));
        assert_eq!(ValidatorCount::<Test>::get(), 1);

        assert_ok!(GhostConsensus::report_misbehavior(
            RuntimeOrigin::signed(2),
            validator,
            SlashingReason::DoubleSigning,
            MisbehaviorEvidence::DoubleSigning {
                first_vote: sp_core::H256::repeat_byte(1),
                second_vote: sp_core::H256::repeat_byte(2),
            },
        ));

        let expected_remaining = amount - ((amount * 100u128) / 100u128);
        assert_eq!(ValidatorStakes::<Test>::get(validator), None);
        assert_eq!(expected_remaining, 0);
        assert_eq!(ValidatorCount::<Test>::get(), 0);
        assert!(DoubleSignReports::<Test>::get(validator));
        assert_eq!(SlashingRecords::<Test>::get().len(), 1);
    });
}

#[test]
fn slashing_requires_reason_matched_evidence() {
    run_test(|| {
        let validator = 1u64;
        let amount = 10_000_000_000_000_000_000u128;
        assert_ok!(GhostConsensus::stake(
            RuntimeOrigin::signed(validator),
            amount
        ));

        assert_err!(
            GhostConsensus::report_misbehavior(
                RuntimeOrigin::signed(2),
                validator,
                SlashingReason::DoubleSigning,
                MisbehaviorEvidence::DoubleSigning {
                    first_vote: sp_core::H256::repeat_byte(1),
                    second_vote: sp_core::H256::repeat_byte(1),
                },
            ),
            Error::<Test>::InvalidEvidence
        );
        assert_eq!(ValidatorStakes::<Test>::get(validator), Some(amount));
    });
}

#[test]
fn downtime_slashing_requires_timeout_evidence() {
    run_test(|| {
        let validator = 1u64;
        let amount = 10_000_000_000_000_000_000u128;
        assert_ok!(GhostConsensus::stake(
            RuntimeOrigin::signed(validator),
            amount
        ));

        System::set_block_number(MaxDowntimeBlocks::get().into());
        assert_err!(
            GhostConsensus::report_misbehavior(
                RuntimeOrigin::signed(2),
                validator,
                SlashingReason::Downtime,
                MisbehaviorEvidence::Downtime,
            ),
            Error::<Test>::InvalidEvidence
        );

        System::set_block_number((MaxDowntimeBlocks::get() + 2).into());
        assert_ok!(GhostConsensus::report_misbehavior(
            RuntimeOrigin::signed(2),
            validator,
            SlashingReason::Downtime,
            MisbehaviorEvidence::Downtime,
        ));

        let expected_remaining = amount - ((amount * DowntimeSlashPercentage::get() as u128) / 100);
        assert_eq!(
            ValidatorStakes::<Test>::get(validator),
            Some(expected_remaining)
        );
        assert_eq!(ValidatorCount::<Test>::get(), 1);
    });
}

#[test]
fn downtime_slashing_skips_when_slashing_records_are_full() {
    run_test(|| {
        let validator = 1u64;
        let amount = 10_000_000_000_000_000_000u128;
        assert_ok!(GhostConsensus::stake(
            RuntimeOrigin::signed(validator),
            amount
        ));

        let full_records = (0..MaxSlashingRecords::get())
            .map(|idx| {
                (
                    validator + idx as u64 + 10,
                    SlashingReason::Other,
                    1u128,
                    idx as u64,
                )
            })
            .collect::<Vec<_>>();
        SlashingRecords::<Test>::put(
            BoundedVec::try_from(full_records).expect("records should fill the configured bound"),
        );

        let events_before = System::events().len();
        System::set_block_number((MaxDowntimeBlocks::get() + 11).into());
        GhostConsensus::check_downtime_slashing();

        assert_eq!(ValidatorStakes::<Test>::get(validator), Some(amount));
        assert_eq!(ValidatorCount::<Test>::get(), 1);
        assert_eq!(SlashingRecords::<Test>::get().len(), MaxSlashingRecords::get() as usize);
        assert_eq!(System::events().len(), events_before);
    });
}

#[test]
fn register_pq_readiness_stores_metadata_and_emits_event() {
    run_test(|| {
        let account = 2u64;
        let metadata = sample_pq_metadata();

        assert_ok!(GhostConsensus::register_pq_readiness(
            RuntimeOrigin::signed(account),
            metadata.clone()
        ));

        assert_eq!(PqReadinessRegistry::<Test>::get(account), Some(metadata));
        System::assert_last_event(
            Event::PqReadinessRegistered {
                account,
                algorithm: PqAlgorithm::MlDsa65,
                proof_kind: PqProofKind::Attestation,
            }
            .into(),
        );
    });
}

#[test]
fn register_pq_readiness_rejects_invalid_metadata() {
    run_test(|| {
        let account = 2u64;

        let mut invalid_version = sample_pq_metadata();
        invalid_version.version = 0;
        assert_err!(
            GhostConsensus::register_pq_readiness(RuntimeOrigin::signed(account), invalid_version),
            Error::<Test>::InvalidPqMetadata
        );

        let mut invalid_algorithm = sample_pq_metadata();
        invalid_algorithm.algorithm = PqAlgorithm::Unknown;
        assert_err!(
            GhostConsensus::register_pq_readiness(
                RuntimeOrigin::signed(account),
                invalid_algorithm
            ),
            Error::<Test>::InvalidPqMetadata
        );

        let mut invalid_window = sample_pq_metadata();
        invalid_window.issued_at = Some(20);
        invalid_window.expires_at = Some(19);
        assert_err!(
            GhostConsensus::register_pq_readiness(RuntimeOrigin::signed(account), invalid_window),
            Error::<Test>::InvalidPqMetadata
        );

        assert_eq!(PqReadinessRegistry::<Test>::get(account), None);
    });
}

#[test]
fn pq_readiness_metadata_roundtrips_and_preserves_claims() {
    let metadata = sample_pq_metadata();

    let encoded = metadata.encode();
    let decoded = PqReadinessMetadata::<u64>::decode(&mut &encoded[..])
        .expect("metadata should decode after encoding");

    assert_eq!(decoded, metadata);
    assert_eq!(decoded.algorithm, PqAlgorithm::MlDsa65);
    assert_eq!(decoded.proof_kind, PqProofKind::Attestation);
    assert_eq!(decoded.claimed_nist_level, Some(3));
}

#[test]
fn default_pq_proof_alias_roundtrips_with_attestation_payload() {
    let mut proof = sample_pq_proof();
    proof.algorithm = PqAlgorithm::Hybrid;
    proof.public_key_commitment = sp_core::H256::repeat_byte(0x22);

    let encoded = proof.encode();
    let decoded = DefaultPqProof::<u64>::decode(&mut &encoded[..])
        .expect("proof should decode after encoding");

    assert_eq!(decoded, proof);
    assert_eq!(decoded.proof.len(), 128);
    assert_eq!(decoded.context.as_slice(), b"validator-2 attests readiness");
    assert_eq!(decoded.proof_kind, PqProofKind::Attestation);
}

#[test]
fn attest_pq_readiness_stores_proof_and_emits_event() {
    run_test(|| {
        let account = 2u64;
        let metadata = sample_pq_metadata();
        let proof = sample_pq_proof();

        assert_ok!(GhostConsensus::register_pq_readiness(
            RuntimeOrigin::signed(account),
            metadata
        ));
        assert_ok!(GhostConsensus::attest_pq_readiness(
            RuntimeOrigin::signed(account),
            proof.clone()
        ));

        assert_eq!(PqReadinessAttestations::<Test>::get(account), Some(proof));
        System::assert_last_event(
            Event::PqReadinessAttested {
                account,
                algorithm: PqAlgorithm::MlDsa65,
                proof_kind: PqProofKind::Attestation,
                statement_hash: sp_core::H256::repeat_byte(0x11),
            }
            .into(),
        );
    });
}

#[test]
fn attest_pq_readiness_rejects_missing_or_mismatched_metadata() {
    run_test(|| {
        let account = 2u64;
        let proof = sample_pq_proof();

        assert_err!(
            GhostConsensus::attest_pq_readiness(RuntimeOrigin::signed(account), proof.clone()),
            Error::<Test>::PqReadinessNotFound
        );

        let metadata = sample_pq_metadata();
        assert_ok!(GhostConsensus::register_pq_readiness(
            RuntimeOrigin::signed(account),
            metadata
        ));

        let mut mismatched_proof = proof;
        mismatched_proof.public_key_commitment = sp_core::H256::repeat_byte(0xFE);
        assert_err!(
            GhostConsensus::attest_pq_readiness(RuntimeOrigin::signed(account), mismatched_proof),
            Error::<Test>::PqMetadataMismatch
        );

        assert_eq!(PqReadinessAttestations::<Test>::get(account), None);
    });
}

#[test]
fn remove_pq_readiness_clears_registry_and_attestations() {
    run_test(|| {
        let account = 2u64;
        let metadata = sample_pq_metadata();
        let proof = sample_pq_proof();

        assert_ok!(GhostConsensus::register_pq_readiness(
            RuntimeOrigin::signed(account),
            metadata
        ));
        assert_ok!(GhostConsensus::attest_pq_readiness(
            RuntimeOrigin::signed(account),
            proof
        ));

        assert_ok!(GhostConsensus::remove_pq_readiness(RuntimeOrigin::signed(
            account
        )));

        assert_eq!(PqReadinessRegistry::<Test>::get(account), None);
        assert_eq!(PqReadinessAttestations::<Test>::get(account), None);
        System::assert_last_event(Event::PqReadinessRemoved { account }.into());
    });
}

#[test]
fn default_pq_proof_alias_enforces_payload_bounds() {
    let oversized_proof = vec![1u8; 4097];
    let oversized_context = vec![2u8; 257];

    assert!(
        BoundedVec::<u8, frame_support::traits::ConstU32<4096>>::try_from(oversized_proof).is_err()
    );
    assert!(
        BoundedVec::<u8, frame_support::traits::ConstU32<256>>::try_from(oversized_context)
            .is_err()
    );
    assert_eq!(PqAlgorithm::default(), PqAlgorithm::Unknown);
    assert_eq!(PqProofKind::default(), PqProofKind::Unknown);
}

#[test]
fn validator_count_is_bounded() {
    run_test(|| {
        for account in 1..=4 {
            assert_ok!(GhostConsensus::stake(
                RuntimeOrigin::signed(account),
                1_000_000_000_000_000_000u128
            ));
        }

        assert_err!(
            GhostConsensus::stake(RuntimeOrigin::signed(5), 1_000_000_000_000_000_000u128),
            Error::<Test>::TooManyValidators
        );
    });
}

#[test]
fn existing_validator_can_top_up_at_validator_cap() {
    run_test(|| {
        for account in 1..=4 {
            assert_ok!(GhostConsensus::stake(
                RuntimeOrigin::signed(account),
                1_000_000_000_000_000_000u128
            ));
        }

        assert_ok!(GhostConsensus::stake(
            RuntimeOrigin::signed(1),
            2_000_000_000_000_000_000u128
        ));

        assert_eq!(ValidatorCount::<Test>::get(), 4);
        assert_eq!(
            ValidatorStakes::<Test>::get(1),
            Some(3_000_000_000_000_000_000u128)
        );
    });
}

#[test]
fn validation_timeout_waits_for_strict_boundary_then_recovers_pow_phase() {
    run_test(|| {
        // Stake so the block enters PosValidation with a selected validator; the
        // validator then never validates, which is exactly the timeout path under test.
        assert_ok!(GhostConsensus::stake(
            RuntimeOrigin::signed(2),
            20_000_000_000_000_000_000u128
        ));
        let genesis = insert_genesis_header();
        let header = mine_test_header(1, &genesis);

        assert_ok!(GhostConsensus::submit_block(
            RuntimeOrigin::signed(1),
            header
        ));
        assert_eq!(PhaseStartedAt::<Test>::get(), 1);

        let boundary_block = 1 + MaxValidationBlocks::get();
        System::set_block_number(boundary_block.into());
        GhostConsensus::on_initialize(boundary_block.into());

        assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::PosValidation);
        assert_eq!(PendingValidationBlock::<Test>::get(), Some(1));

        let timed_out_block = boundary_block + 1;
        System::set_block_number(timed_out_block.into());
        GhostConsensus::on_initialize(timed_out_block.into());

        assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::PowMining);
        assert_eq!(PendingValidationBlock::<Test>::get(), None);
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

#[test]
fn validate_block_enforces_real_ml_dsa_signature_when_registered() {
    use fips204::traits::{SerDes, Signer};
    run_test(|| {
        let miner = 1u64;
        let validator = 2u64;

        assert_ok!(GhostConsensus::stake(
            RuntimeOrigin::signed(validator),
            20_000_000_000_000_000_000u128
        ));

        // Register a real ML-DSA-87 ("Dilithium-5") key for the validator.
        let (pk, sk) = fips204::ml_dsa_87::try_keygen().expect("keygen");
        let pk_bytes = pk.into_bytes();
        assert_ok!(GhostConsensus::register_ml_dsa_key(
            RuntimeOrigin::signed(validator),
            PqAlgorithm::MlDsa87,
            pk_bytes.to_vec().try_into().expect("pk fits bound"),
        ));

        let genesis = insert_genesis_header();
        let header = mine_test_header(1, &genesis);
        assert_ok!(GhostConsensus::submit_block(
            RuntimeOrigin::signed(miner),
            header
        ));
        assert_eq!(PendingValidator::<Test>::get(), Some(validator));

        // The validator signs the canonical message: (stored header, validator id).
        let stored = BlockHeaders::<Test>::get(1).unwrap();
        let message = (
            stored.number,
            stored.parent_hash,
            stored.state_root,
            stored.extrinsics_root,
            stored.nonce,
            stored.difficulty,
            validator,
        )
            .encode();
        let good_sig = sk.try_sign(&message, GHOST_VALIDATOR_CTX).expect("sign");

        // A registered validator cannot validate without a signature.
        assert_err!(
            GhostConsensus::validate_block(RuntimeOrigin::signed(validator), 1, None),
            Error::<Test>::MlDsaSignatureMissing
        );

        // A tampered signature is rejected by on-chain ML-DSA verification.
        let mut bad_sig = good_sig;
        bad_sig[10] ^= 0xFF;
        assert_err!(
            GhostConsensus::validate_block(
                RuntimeOrigin::signed(validator),
                1,
                Some(bad_sig.to_vec().try_into().unwrap())
            ),
            Error::<Test>::MlDsaSignatureInvalid
        );

        // The correct ML-DSA signature validates the block.
        assert_ok!(GhostConsensus::validate_block(
            RuntimeOrigin::signed(validator),
            1,
            Some(good_sig.to_vec().try_into().unwrap())
        ));
        assert_eq!(CurrentPhase::<Test>::get(), ConsensusPhase::Finalization);
        assert_eq!(BlockValidators::<Test>::get(1), Some(validator));
    });
}

#[test]
fn verify_pq_signature_extrinsic_checks_real_ml_dsa() {
    use fips204::traits::{SerDes, Signer};
    run_test(|| {
        let attester = 2u64;
        let (pk, sk) = fips204::ml_dsa_87::try_keygen().expect("keygen");
        assert_ok!(GhostConsensus::register_ml_dsa_key(
            RuntimeOrigin::signed(attester),
            PqAlgorithm::MlDsa87,
            pk.into_bytes().to_vec().try_into().unwrap(),
        ));

        let message = b"ghost attests a statement".to_vec();
        let ctx = b"attestation-ctx".to_vec();
        let sig = sk.try_sign(&message, &ctx).expect("sign");

        // A real, valid ML-DSA signature is accepted.
        assert_ok!(GhostConsensus::verify_pq_signature(
            RuntimeOrigin::signed(attester),
            message.clone().try_into().unwrap(),
            sig.to_vec().try_into().unwrap(),
            ctx.clone().try_into().unwrap(),
        ));

        // A bit-flipped signature is rejected.
        let mut bad = sig;
        bad[5] ^= 0xFF;
        assert_err!(
            GhostConsensus::verify_pq_signature(
                RuntimeOrigin::signed(attester),
                message.try_into().unwrap(),
                bad.to_vec().try_into().unwrap(),
                ctx.try_into().unwrap(),
            ),
            Error::<Test>::MlDsaSignatureInvalid
        );
    });
}

#[test]
fn verify_pq_signature_rejects_replay() {
    use fips204::traits::{SerDes, Signer};
    run_test(|| {
        let attester = 2u64;
        let (pk, sk) = fips204::ml_dsa_87::try_keygen().expect("keygen");
        assert_ok!(GhostConsensus::register_ml_dsa_key(
            RuntimeOrigin::signed(attester),
            PqAlgorithm::MlDsa87,
            pk.into_bytes().to_vec().try_into().unwrap(),
        ));

        let message = b"replayable statement".to_vec();
        let ctx = b"attestation-ctx".to_vec();
        let sig = sk.try_sign(&message, &ctx).expect("sign");

        // First submission of a valid attestation is recorded.
        assert_ok!(GhostConsensus::verify_pq_signature(
            RuntimeOrigin::signed(attester),
            message.clone().try_into().unwrap(),
            sig.to_vec().try_into().unwrap(),
            ctx.clone().try_into().unwrap(),
        ));

        // Replaying the exact same valid attestation is rejected by the replay guard.
        assert_err!(
            GhostConsensus::verify_pq_signature(
                RuntimeOrigin::signed(attester),
                message.try_into().unwrap(),
                sig.to_vec().try_into().unwrap(),
                ctx.try_into().unwrap(),
            ),
            Error::<Test>::AttestationAlreadyRecorded
        );
    });
}

#[test]
fn register_ml_dsa_key_rejects_invalid_key() {
    run_test(|| {
        // Wrong length for ML-DSA-87.
        let too_short: BoundedVec<u8, frame_support::traits::ConstU32<2592>> =
            vec![0u8; 100].try_into().unwrap();
        assert_err!(
            GhostConsensus::register_ml_dsa_key(
                RuntimeOrigin::signed(1),
                PqAlgorithm::MlDsa87,
                too_short
            ),
            Error::<Test>::MlDsaKeyInvalid
        );
    });
}

#[test]
fn difficulty_retargets_toward_target_block_time() {
    run_test(|| {
        let start = Difficulty::<Test>::get();

        // Establish the retarget baseline at block 1, t = 1000ms.
        System::set_block_number(1);
        Timestamp::set_timestamp(1_000);
        GhostConsensus::adjust_difficulty();
        assert_eq!(LastRetargetBlock::<Test>::get(), 1);

        // 20 blocks but only 20_000ms elapsed => 1000ms/block, far faster than the
        // 5000ms target => difficulty must rise (clamped to 4x).
        System::set_block_number(21);
        Timestamp::set_timestamp(21_000);
        GhostConsensus::adjust_difficulty();
        let faster = Difficulty::<Test>::get();
        assert!(faster > start, "fast blocks should raise difficulty");

        // 20 blocks with 400_000ms elapsed => 20_000ms/block, far slower than target
        // => difficulty must fall.
        System::set_block_number(41);
        Timestamp::set_timestamp(421_000);
        GhostConsensus::adjust_difficulty();
        let slower = Difficulty::<Test>::get();
        assert!(slower < faster, "slow blocks should lower difficulty");
    });
}
