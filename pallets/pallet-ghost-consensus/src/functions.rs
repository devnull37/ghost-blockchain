//! Pure helper functions used by the Ghost consensus pallet.

use super::*;
use crate::types::{
    BlockReward, DefaultPqProof, GhostBlockHeader, MisbehaviorEvidence, PosSelection, PqAlgorithm,
    PqProofKind, PqReadinessMetadata, SlashingReason, ValidatorStake,
};
use frame_support::pallet_prelude::*;
use sp_core::{H256, U256};
use sp_runtime::traits::{BlakeTwo256, Hash, SaturatedConversion, Saturating, Zero};

fn is_structurally_known_pq_algorithm(algorithm: &PqAlgorithm) -> bool {
    match algorithm {
        PqAlgorithm::Unknown => false,
        PqAlgorithm::Other(label) => label.iter().any(|byte| *byte != 0),
        _ => true,
    }
}

fn is_structurally_known_pq_proof_kind(proof_kind: &PqProofKind) -> bool {
    !matches!(proof_kind, PqProofKind::Unknown)
}

/// Calculate a simple difficulty adjustment towards the target block time.
pub fn calculate_difficulty_adjustment<T: Config>(
    current_difficulty: u64,
    actual_block_time: u64,
    target_block_time: u64,
) -> u64 {
    if actual_block_time == 0 {
        return current_difficulty;
    }

    current_difficulty
        .saturating_mul(target_block_time)
        .checked_div(actual_block_time)
        .unwrap_or(current_difficulty)
}

/// Whether `hash` satisfies the conventional difficulty `difficulty`.
///
/// Difficulty uses the standard convention: numerically *larger* difficulty is
/// *harder*. The hash is interpreted as a big-endian 256-bit integer and is valid
/// iff `hash <= U256::MAX / difficulty`. The full 256 bits are compared (no
/// truncation), and `difficulty` is clamped to a minimum of 1 to avoid division
/// by zero (difficulty 1 = easiest, every hash passes).
pub fn meets_difficulty(hash: &H256, difficulty: u64) -> bool {
    let work = U256::from_big_endian(hash.as_bytes());
    let target = U256::MAX / U256::from(difficulty.max(1));
    work <= target
}

/// Double Blake2-256 work hash over the header's PoW preimage. This is the
/// canonical Ghost PoW hash and matches the node's miner.
pub fn pow_work_hash(block_header: &GhostBlockHeader) -> H256 {
    let hash_input = (
        block_header.number,
        block_header.parent_hash,
        block_header.state_root,
        block_header.extrinsics_root,
        block_header.nonce,
    );
    let first_hash = BlakeTwo256::hash_of(&hash_input);
    BlakeTwo256::hash_of(&first_hash)
}

/// Verify Proof-of-Work using a single Blake2-256 hash.
pub fn verify_pow(block_header: &GhostBlockHeader, difficulty: u64) -> bool {
    let hash_input = (
        block_header.number,
        block_header.parent_hash,
        block_header.state_root,
        block_header.extrinsics_root,
        block_header.nonce,
    );
    meets_difficulty(&BlakeTwo256::hash_of(&hash_input), difficulty)
}

/// Verify Proof-of-Work using a double Blake2-256 hash (the canonical Ghost PoW).
pub fn verify_pow_enhanced(block_header: &GhostBlockHeader, difficulty: u64) -> bool {
    meets_difficulty(&pow_work_hash(block_header), difficulty)
}

/// Verify Proof-of-Work using double SHA-256.
pub fn verify_pow_sha256(block_header: &GhostBlockHeader, difficulty: u64) -> bool {
    use sp_core::sha2_256;

    let hash_input = (
        block_header.number,
        block_header.parent_hash,
        block_header.state_root,
        block_header.extrinsics_root,
        block_header.nonce,
    );

    let first_hash = sha2_256(&hash_input.encode());
    let final_hash = sha2_256(&first_hash);
    meets_difficulty(&H256::from_slice(&final_hash), difficulty)
}

/// Verify Proof-of-Work using Keccak-256.
pub fn verify_pow_keccak(block_header: &GhostBlockHeader, difficulty: u64) -> bool {
    use sp_core::keccak_256;

    let hash_input = (
        block_header.number,
        block_header.parent_hash,
        block_header.state_root,
        block_header.extrinsics_root,
        block_header.nonce,
    );

    meets_difficulty(&H256::from_slice(&keccak_256(&hash_input.encode())), difficulty)
}

/// Select a validator using stake-weighted sampling.
pub fn select_pos_validator<T: Config>(
    stakers: Vec<ValidatorStake<T::AccountId, BalanceOf<T>>>,
    seed: H256,
) -> Option<PosSelection<T::AccountId>> {
    if stakers.is_empty() {
        return None;
    }

    // Saturating accumulation: weights are u64 and a naive `sum()`/`+=` would
    // overflow (and silently wrap in release) for large validator sets.
    let total_weight: u64 = stakers
        .iter()
        .fold(0u64, |acc, stake| acc.saturating_add(stake.weight));
    if total_weight == 0 {
        return None;
    }

    let random_value =
        u64::from_be_bytes(seed.as_bytes()[0..8].try_into().unwrap_or_default()) % total_weight;

    let mut cumulative_weight = 0u64;
    for staker in stakers {
        cumulative_weight = cumulative_weight.saturating_add(staker.weight);
        if random_value < cumulative_weight {
            return Some(PosSelection {
                validator: staker.account,
                weight: staker.weight,
                round: frame_system::Pallet::<T>::block_number().saturated_into::<u64>(),
            });
        }
    }

    None
}

/// Split a block reward 40/60 between miner and stakers.
pub fn calculate_block_reward<T: Config>(total_reward: BalanceOf<T>) -> BlockReward<BalanceOf<T>> {
    // Saturating arithmetic: a chain must never panic (debug) or silently wrap
    // (release) on a large block reward. 40% to the miner, 60% to stakers.
    let miner_reward = total_reward.saturating_mul(40u32.into()) / 100u32.into();
    let stakers_reward = total_reward.saturating_sub(miner_reward);

    BlockReward {
        total: total_reward,
        miner_reward,
        stakers_reward,
    }
}

/// Validate a submitted Ghost block header against its parent.
pub fn validate_block_header<T: Config>(
    header: &GhostBlockHeader,
    parent_header: &GhostBlockHeader,
) -> DispatchResult {
    // Checked successor: prevents a u32 wrap (parent.number == u32::MAX) from
    // producing header.number == 0 and overwriting the genesis header.
    ensure!(
        parent_header.number.checked_add(1) == Some(header.number),
        Error::<T>::InvalidBlockNumber
    );
    ensure!(
        header.parent_hash == BlakeTwo256::hash_of(parent_header),
        Error::<T>::InvalidParentHash
    );

    // PoW MUST be verified against the canonical on-chain difficulty, never the
    // attacker-supplied `header.difficulty`. The header must also declare exactly
    // the current difficulty, so a miner cannot mine against a stale/easier target.
    let expected_difficulty = Difficulty::<T>::get();
    ensure!(
        header.difficulty == expected_difficulty,
        Error::<T>::DifficultyMismatch
    );
    ensure!(
        verify_pow_enhanced(header, expected_difficulty),
        Error::<T>::InvalidPow
    );

    Ok(())
}

/// Validate slashing evidence before stake can be reduced.
pub fn validate_misbehavior_evidence<T: Config>(
    validator: &T::AccountId,
    reason: &SlashingReason,
    evidence: &MisbehaviorEvidence,
) -> DispatchResult {
    match (reason, evidence) {
        (
            SlashingReason::DoubleSigning,
            MisbehaviorEvidence::DoubleSigning {
                first_vote,
                second_vote,
            },
        ) => {
            ensure!(first_vote != second_vote, Error::<T>::InvalidEvidence);
            ensure!(*first_vote != H256::zero(), Error::<T>::InvalidEvidence);
            ensure!(*second_vote != H256::zero(), Error::<T>::InvalidEvidence);
            ensure!(
                !DoubleSignReports::<T>::get(validator),
                Error::<T>::InvalidEvidence
            );
            Ok(())
        }
        (SlashingReason::InvalidBlock, MisbehaviorEvidence::InvalidBlock { block_number }) => {
            let header = BlockHeaders::<T>::get(*block_number).ok_or(Error::<T>::BlockNotFound)?;

            // Evidence is valid iff the block's PoW does NOT satisfy the difficulty the
            // header itself claims, AND the accused validator is the recorded validator.
            // We verify against the header's OWN difficulty (not the current on-chain
            // difficulty) so a later retarget cannot retroactively make a correctly
            // validated block look slashable.
            ensure!(
                !verify_pow_enhanced(&header, header.difficulty),
                Error::<T>::InvalidEvidence
            );
            ensure!(
                BlockValidators::<T>::get(*block_number).as_ref() == Some(validator),
                Error::<T>::InvalidEvidence
            );
            ensure!(
                !InvalidBlockReports::<T>::get(validator),
                Error::<T>::InvalidEvidence
            );
            Ok(())
        }
        (SlashingReason::Downtime, MisbehaviorEvidence::Downtime) => {
            let current_block = frame_system::Pallet::<T>::block_number().saturated_into::<u32>();
            let last_active = LastActiveBlock::<T>::get(validator);
            ensure!(
                current_block.saturating_sub(last_active) > T::MaxDowntimeBlocks::get(),
                Error::<T>::InvalidEvidence
            );
            Ok(())
        }
        (SlashingReason::Other, MisbehaviorEvidence::Other { proof_hash }) => {
            ensure!(*proof_hash != H256::zero(), Error::<T>::InvalidEvidence);
            Ok(())
        }
        _ => Err(Error::<T>::InvalidEvidence.into()),
    }
}

/// Validate claimed PQ metadata as a structural registry claim only.
///
/// This does not perform cryptographic verification of PQ keys, proofs, or signatures.
pub fn validate_pq_readiness_metadata<T: Config>(
    metadata: &PqReadinessMetadata<BlockNumberFor<T>>,
) -> DispatchResult {
    ensure!(metadata.version > 0, Error::<T>::InvalidPqMetadata);
    ensure!(
        is_structurally_known_pq_algorithm(&metadata.algorithm),
        Error::<T>::InvalidPqMetadata
    );
    ensure!(
        is_structurally_known_pq_proof_kind(&metadata.proof_kind),
        Error::<T>::InvalidPqMetadata
    );
    ensure!(
        metadata.key_strength_bits > 0,
        Error::<T>::InvalidPqMetadata
    );
    ensure!(
        metadata.public_key_commitment != H256::zero(),
        Error::<T>::InvalidPqMetadata
    );
    if let Some(metadata_hash) = metadata.metadata_hash {
        ensure!(metadata_hash != H256::zero(), Error::<T>::InvalidPqMetadata);
    }

    if let Some(level) = metadata.claimed_nist_level {
        ensure!((1..=5).contains(&level), Error::<T>::InvalidPqMetadata);
    }

    if let (Some(issued_at), Some(expires_at)) = (metadata.issued_at, metadata.expires_at) {
        ensure!(issued_at <= expires_at, Error::<T>::InvalidPqMetadata);
    }

    Ok(())
}

/// Validate an opaque PQ proof envelope against claimed PQ metadata.
///
/// This only checks internal consistency and expiry windows so off-chain clients can
/// decide whether to perform full cryptographic verification.
pub fn validate_pq_proof_envelope<T: Config>(
    metadata: &PqReadinessMetadata<BlockNumberFor<T>>,
    proof: &DefaultPqProof<BlockNumberFor<T>>,
) -> DispatchResult {
    validate_pq_readiness_metadata::<T>(metadata)?;

    ensure!(!proof.proof.is_empty(), Error::<T>::InvalidPqProof);
    ensure!(
        proof.proof.iter().any(|byte| *byte != 0),
        Error::<T>::InvalidPqProof
    );
    ensure!(
        proof.statement_hash != H256::zero(),
        Error::<T>::InvalidPqProof
    );
    ensure!(
        proof.public_key_commitment != H256::zero(),
        Error::<T>::InvalidPqProof
    );
    ensure!(
        is_structurally_known_pq_algorithm(&proof.algorithm),
        Error::<T>::InvalidPqProof
    );
    ensure!(
        is_structurally_known_pq_proof_kind(&proof.proof_kind),
        Error::<T>::InvalidPqProof
    );
    ensure!(
        proof.algorithm == metadata.algorithm,
        Error::<T>::PqMetadataMismatch
    );
    ensure!(
        proof.proof_kind == metadata.proof_kind,
        Error::<T>::PqMetadataMismatch
    );
    ensure!(
        proof.public_key_commitment == metadata.public_key_commitment,
        Error::<T>::PqMetadataMismatch
    );
    if let Some(auxiliary_hash) = proof.auxiliary_hash {
        ensure!(auxiliary_hash != H256::zero(), Error::<T>::InvalidPqProof);
    }

    if matches!(
        proof.proof_kind,
        PqProofKind::Attestation | PqProofKind::Transcript
    ) {
        ensure!(!proof.context.is_empty(), Error::<T>::InvalidPqProof);
    }

    if let Some(issued_at) = metadata.issued_at {
        ensure!(proof.submitted_at >= issued_at, Error::<T>::InvalidPqProof);
    }

    if let Some(expires_at) = metadata.expires_at {
        ensure!(
            proof.submitted_at <= expires_at,
            Error::<T>::PqMetadataExpired
        );
    }

    Ok(())
}

/// Distribute rewards to the miner and all current stakers.
pub fn distribute_rewards<T: Config>(
    miner: T::AccountId,
    stakers: Vec<ValidatorStake<T::AccountId, BalanceOf<T>>>,
    reward: BlockReward<BalanceOf<T>>,
) -> DispatchResult {
    let total_stake: BalanceOf<T> = stakers
        .iter()
        .fold(Zero::zero(), |acc, stake| acc.saturating_add(stake.stake));

    if total_stake.is_zero() {
        // No stakers: the miner receives the entire block reward, so none of the reward
        // is silently dropped (the 60% staker share would otherwise be un-minted).
        let _ = pallet_balances::Pallet::<T>::deposit_creating(&miner, reward.total);
        return Ok(());
    }

    let _ = pallet_balances::Pallet::<T>::deposit_creating(&miner, reward.miner_reward);

    // Proportional distribution with saturating math. The final staker absorbs any
    // integer-division dust so the entire stakers_reward is always paid out.
    let mut distributed: BalanceOf<T> = Zero::zero();
    let last_index = stakers.len().saturating_sub(1);
    for (index, staker) in stakers.iter().enumerate() {
        let staker_reward = if index == last_index {
            reward.stakers_reward.saturating_sub(distributed)
        } else {
            reward.stakers_reward.saturating_mul(staker.stake) / total_stake
        };
        distributed = distributed.saturating_add(staker_reward);
        let _ = pallet_balances::Pallet::<T>::deposit_creating(&staker.account, staker_reward);
    }

    Ok(())
}
