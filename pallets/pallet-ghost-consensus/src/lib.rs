//! Ghost consensus pallet — the Proof-of-Stake economic layer of the hybrid chain.
//!
//! Real block production is Proof-of-Work, authored by the node via `sc-consensus-pow`
//! (Aura and GRANDPA have been removed from the node and runtime). This pallet is the
//! on-chain economic + validation layer on top of that PoW:
//! 1. A miner submits the PoW block header it solved.
//! 2. A stake-weighted validator finalizes that header (optionally attesting with a real
//!    ML-DSA-87 / FIPS 204 post-quantum signature that is verified on-chain) and rewards
//!    are paid out (40% miner / 60% stakers); misbehavior is slashed and the funds burned.
//!
//! When the active staker set is empty there is no validator to select, so a submitted
//! block is finalized immediately with the full reward paid to the miner. The pallet also
//! exposes the current mining difficulty to the node through `DifficultyApi`.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use codec::Encode;
pub use pallet::*;

pub mod functions;
pub mod pq_verify;
pub mod types;
pub mod weights;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

use alloc::vec::Vec;
use frame_support::{
    pallet_prelude::*,
    traits::{Currency, ExistenceRequirement, WithdrawReasons},
};
use frame_system::pallet_prelude::*;
use sp_runtime::traits::{
    AccountIdConversion, BlakeTwo256, Hash, SaturatedConversion, Saturating, Zero,
};

use crate::functions::{
    calculate_block_reward, calculate_difficulty_adjustment, distribute_rewards,
    select_pos_validator, validate_block_header, validate_misbehavior_evidence,
    validate_pq_proof_envelope, validate_pq_readiness_metadata,
};
use crate::types::{
    ConsensusPhase, DefaultPqProof, GenesisHeaderInit, GhostBlockHeader, MisbehaviorEvidence,
    PqAlgorithm, PqProofKind, PqReadinessMetadata, SlashingReason, ValidatorStake,
};

pub type BalanceOf<T> = <T as pallet_balances::Config>::Balance;

/// FIPS 204 context string binding validator attestation signatures to this chain
/// and use-case (domain separation), per ML-DSA's context-string parameter.
pub const GHOST_VALIDATOR_CTX: &[u8] = b"ghost-validator-v1";

/// Number of recent Ghost blocks whose header/miner/validator records are retained.
/// Older entries are pruned on finalization to bound on-chain state growth.
pub const HEADER_RETENTION: u32 = 1024;

#[frame_support::pallet]
pub mod pallet {
    use super::*;

    #[pallet::config]
    pub trait Config:
        frame_system::Config + pallet_balances::Config + pallet_timestamp::Config
    {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type WeightInfo: WeightInfo;

        /// Target average block time in milliseconds. PoW difficulty retargets toward this.
        #[pallet::constant]
        type TargetBlockTime: Get<u64>;
        /// How often (in blocks) PoW difficulty is retargeted.
        #[pallet::constant]
        type DifficultyAdjustmentPeriod: Get<u32>;

        #[pallet::constant]
        type BlockReward: Get<BalanceOf<Self>>;
        #[pallet::constant]
        type MinStake: Get<BalanceOf<Self>>;
        #[pallet::constant]
        type MaxDowntimeBlocks: Get<u32>;
        #[pallet::constant]
        type MaxValidationBlocks: Get<u32>;
        #[pallet::constant]
        type MaxValidators: Get<u32>;
        #[pallet::constant]
        type MaxSlashingRecords: Get<u32>;
        #[pallet::constant]
        type DoubleSignSlashPercentage: Get<u8>;
        #[pallet::constant]
        type InvalidBlockSlashPercentage: Get<u8>;
        #[pallet::constant]
        type DowntimeSlashPercentage: Get<u8>;
        #[pallet::constant]
        type PalletId: Get<frame_support::PalletId>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::type_value]
    pub fn DefaultDifficulty<T: Config>() -> u64 {
        // Conventional difficulty (higher = harder). A modest devnet default a CPU can mine
        // quickly at genesis; on-chain retargeting then tunes it toward the target block time.
        100_000
    }

    #[pallet::type_value]
    pub fn DefaultPhase<T: Config>() -> ConsensusPhase {
        ConsensusPhase::PowMining
    }

    #[pallet::storage]
    pub type Difficulty<T: Config> = StorageValue<_, u64, ValueQuery, DefaultDifficulty<T>>;

    /// Block number recorded at the last difficulty retarget.
    #[pallet::storage]
    pub type LastRetargetBlock<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Timestamp (ms) recorded at the last difficulty retarget.
    #[pallet::storage]
    pub type LastRetargetMoment<T: Config> = StorageValue<_, u64, ValueQuery>;

    #[pallet::storage]
    pub type CurrentPhase<T: Config> = StorageValue<_, ConsensusPhase, ValueQuery, DefaultPhase<T>>;

    #[pallet::storage]
    pub type BlockHeaders<T: Config> = StorageMap<_, Blake2_128Concat, u32, GhostBlockHeader>;

    #[pallet::storage]
    pub type BlockMiners<T: Config> = StorageMap<_, Blake2_128Concat, u32, T::AccountId>;

    #[pallet::storage]
    pub type PendingValidationBlock<T: Config> = StorageValue<_, u32>;

    #[pallet::storage]
    pub type PhaseStartedAt<T: Config> = StorageValue<_, u32, ValueQuery>;

    #[pallet::storage]
    pub type ValidatorStakes<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>>;

    #[pallet::storage]
    pub type ValidatorCount<T: Config> = StorageValue<_, u32, ValueQuery>;

    #[pallet::storage]
    pub type LastActiveBlock<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    #[pallet::storage]
    pub type DoubleSignReports<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, bool, ValueQuery>;

    #[pallet::storage]
    pub type InvalidBlockReports<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, bool, ValueQuery>;

    #[pallet::storage]
    pub type SlashingRecords<T: Config> = StorageValue<
        _,
        BoundedVec<
            (
                T::AccountId,
                SlashingReason,
                BalanceOf<T>,
                BlockNumberFor<T>,
            ),
            T::MaxSlashingRecords,
        >,
        ValueQuery,
    >;

    #[pallet::storage]
    pub type PqReadinessRegistry<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, PqReadinessMetadata<BlockNumberFor<T>>>;

    #[pallet::storage]
    pub type PqReadinessAttestations<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, DefaultPqProof<BlockNumberFor<T>>>;

    /// The validator selected (at submit time) to validate the pending block.
    /// Storing the selection removes the race where the selected validator could
    /// change between `validate_block` attempts.
    #[pallet::storage]
    pub type PendingValidator<T: Config> = StorageValue<_, T::AccountId>;

    /// Which validator validated each block. Used for slashing attribution.
    #[pallet::storage]
    pub type BlockValidators<T: Config> = StorageMap<_, Blake2_128Concat, u32, T::AccountId>;

    /// Registered ML-DSA (FIPS 204) public key per validator. The bound (2592 bytes)
    /// fits the largest parameter set, ML-DSA-87 ("Dilithium-5").
    #[pallet::storage]
    pub type ValidatorMlDsaKey<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BoundedVec<u8, ConstU32<2592>>>;

    /// The ML-DSA parameter set a validator registered their key under.
    #[pallet::storage]
    pub type ValidatorMlDsaAlgo<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, PqAlgorithm>;

    /// Verified post-quantum attestations: (attester, statement hash) -> block recorded.
    #[pallet::storage]
    pub type PqVerifiedAttestations<T: Config> =
        StorageMap<_, Blake2_128Concat, (T::AccountId, sp_core::H256), BlockNumberFor<T>>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        BlockMined {
            block_number: u32,
            miner: T::AccountId,
            nonce: u64,
        },
        ValidatorSelected {
            validator: T::AccountId,
            weight: u64,
        },
        RewardsDistributed {
            miner: T::AccountId,
            miner_reward: BalanceOf<T>,
            stakers_reward: BalanceOf<T>,
        },
        ValidatorSlashed {
            validator: T::AccountId,
            reason: SlashingReason,
            amount: BalanceOf<T>,
        },
        DifficultyAdjusted {
            old_difficulty: u64,
            new_difficulty: u64,
        },
        ValidationTimedOut {
            block_number: u32,
        },
        PqReadinessRegistered {
            account: T::AccountId,
            algorithm: PqAlgorithm,
            proof_kind: PqProofKind,
        },
        PqReadinessAttested {
            account: T::AccountId,
            algorithm: PqAlgorithm,
            proof_kind: PqProofKind,
            statement_hash: sp_core::H256,
        },
        PqReadinessRemoved {
            account: T::AccountId,
        },
        Staked {
            staker: T::AccountId,
            amount: BalanceOf<T>,
            total_stake: BalanceOf<T>,
        },
        Unstaked {
            staker: T::AccountId,
            amount: BalanceOf<T>,
            remaining: BalanceOf<T>,
        },
        ValidatorMlDsaKeyRegistered {
            account: T::AccountId,
            algorithm: PqAlgorithm,
        },
        PqSignatureVerified {
            attester: T::AccountId,
            algorithm: PqAlgorithm,
            statement_hash: sp_core::H256,
        },
    }

    #[pallet::error]
    pub enum Error<T> {
        InvalidBlockNumber,
        InvalidParentHash,
        InvalidPow,
        DifficultyTooLow,
        DifficultyTooHigh,
        DifficultyMismatch,
        InsufficientStake,
        NotAValidator,
        BlockNotFound,
        InvalidPhaseTransition,
        InvalidEvidence,
        TooManyValidators,
        TooManySlashingRecords,
        PhaseTimeoutNotReached,
        PqReadinessNotFound,
        InvalidPqMetadata,
        InvalidPqProof,
        PqMetadataMismatch,
        PqMetadataExpired,
        MlDsaKeyInvalid,
        MlDsaSignatureInvalid,
        MlDsaSignatureMissing,
        MlDsaNotRegistered,
        NotSelectedValidator,
        AttestationAlreadyRecorded,
    }

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub genesis_header: Option<GenesisHeaderInit>,
        pub validator_stakes: Vec<(T::AccountId, BalanceOf<T>)>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            if let Some(header) = &self.genesis_header {
                let header = GhostBlockHeader {
                    number: header.0,
                    parent_hash: header.1,
                    state_root: header.2,
                    extrinsics_root: header.3,
                    nonce: header.4,
                    difficulty: header.5,
                    validator_signature: header.6,
                };
                BlockHeaders::<T>::insert(header.number, header);
            }

            let mut count = 0u32;
            for (account, stake) in self
                .validator_stakes
                .iter()
                .take(T::MaxValidators::get() as usize)
            {
                if *stake >= T::MinStake::get() {
                    ValidatorStakes::<T>::insert(account, stake);
                    LastActiveBlock::<T>::insert(account, 0u32);
                    // Back the genesis stake with real tokens in the pallet account so
                    // that unstaking and slashing have funds to move/burn.
                    let _ = pallet_balances::Pallet::<T>::deposit_creating(
                        &Pallet::<T>::account_id(),
                        *stake,
                    );
                    count = count.saturating_add(1);
                }
            }
            ValidatorCount::<T>::put(count);
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(<T as Config>::WeightInfo::submit_block())]
        pub fn submit_block(
            origin: OriginFor<T>,
            block_header: GhostBlockHeader,
        ) -> DispatchResult {
            let miner = ensure_signed(origin)?;
            ensure!(
                CurrentPhase::<T>::get() == ConsensusPhase::PowMining,
                Error::<T>::InvalidPhaseTransition
            );

            let parent_header = block_header
                .number
                .checked_sub(1)
                .and_then(BlockHeaders::<T>::get)
                .ok_or(Error::<T>::BlockNotFound)?;

            validate_block_header::<T>(&block_header, &parent_header)?;

            BlockMiners::<T>::insert(block_header.number, miner.clone());
            BlockHeaders::<T>::insert(block_header.number, block_header.clone());
            LastActiveBlock::<T>::insert(&miner, block_header.number);

            Self::deposit_event(Event::BlockMined {
                block_number: block_header.number,
                miner: miner.clone(),
                nonce: block_header.nonce,
            });

            // Select the PoS validator now (at submit time) using the parent block
            // hash as entropy. Storing the selection binds it to a value the submitter
            // cannot grind and removes the per-call re-selection race in validate_block.
            let stakers = Self::all_stakers();
            let seed = BlakeTwo256::hash_of(&(
                block_header.number,
                frame_system::Pallet::<T>::parent_hash(),
            ));
            match select_pos_validator::<T>(stakers, seed) {
                Some(selection) => {
                    // Stakers exist: enter PoS validation and await the selected validator.
                    PendingValidationBlock::<T>::put(block_header.number);
                    PhaseStartedAt::<T>::put(
                        frame_system::Pallet::<T>::block_number().saturated_into::<u32>(),
                    );
                    PendingValidator::<T>::put(selection.validator);
                    CurrentPhase::<T>::put(ConsensusPhase::PosValidation);
                }
                None => {
                    // No stakers means there is no validator to select and no PoS
                    // validation to perform. Finalize the PoW block immediately and pay
                    // the miner the full reward instead of entering PosValidation and
                    // stalling until the validation timeout fires; the chain stays in
                    // PowMining, ready for the next block.
                    Self::finalize_without_validator(block_header.number, &miner)?;
                }
            }

            Ok(())
        }

        #[pallet::call_index(1)]
        #[pallet::weight(<T as Config>::WeightInfo::stake())]
        pub fn stake(origin: OriginFor<T>, amount: BalanceOf<T>) -> DispatchResult {
            let staker = ensure_signed(origin)?;
            ensure!(amount >= T::MinStake::get(), Error::<T>::InsufficientStake);
            let is_new_validator = !ValidatorStakes::<T>::contains_key(&staker);
            if is_new_validator {
                ensure!(
                    ValidatorCount::<T>::get() < T::MaxValidators::get(),
                    Error::<T>::TooManyValidators
                );
            }

            <pallet_balances::Pallet<T> as Currency<T::AccountId>>::transfer(
                &staker,
                &Self::account_id(),
                amount,
                ExistenceRequirement::AllowDeath,
            )?;

            let current_stake = ValidatorStakes::<T>::get(&staker).unwrap_or_default();
            let new_total = current_stake.saturating_add(amount);
            ValidatorStakes::<T>::insert(&staker, new_total);
            if is_new_validator {
                ValidatorCount::<T>::mutate(|count| *count = count.saturating_add(1));
            }

            Self::deposit_event(Event::Staked {
                staker,
                amount,
                total_stake: new_total,
            });
            Ok(())
        }

        #[pallet::call_index(2)]
        #[pallet::weight(<T as Config>::WeightInfo::unstake())]
        pub fn unstake(origin: OriginFor<T>, amount: BalanceOf<T>) -> DispatchResult {
            let staker = ensure_signed(origin)?;
            let current_stake =
                ValidatorStakes::<T>::get(&staker).ok_or(Error::<T>::NotAValidator)?;
            ensure!(current_stake >= amount, Error::<T>::InsufficientStake);

            let new_stake = current_stake - amount;
            // A partial unstake must not leave a sub-minimum stake registered (which would
            // keep an under-collateralized validator active). Check before any state change.
            if !new_stake.is_zero() {
                ensure!(
                    new_stake >= T::MinStake::get(),
                    Error::<T>::InsufficientStake
                );
            }

            // Return the staked funds FIRST: if the transfer fails we exit before mutating
            // the stake records, so records can never be reduced without returning tokens.
            <pallet_balances::Pallet<T> as Currency<T::AccountId>>::transfer(
                &Self::account_id(),
                &staker,
                amount,
                ExistenceRequirement::AllowDeath,
            )?;

            if new_stake.is_zero() {
                ValidatorStakes::<T>::remove(&staker);
                ValidatorCount::<T>::mutate(|count| *count = count.saturating_sub(1));
            } else {
                ValidatorStakes::<T>::insert(&staker, new_stake);
            }

            Self::deposit_event(Event::Unstaked {
                staker,
                amount,
                remaining: new_stake,
            });
            Ok(())
        }

        #[pallet::call_index(3)]
        #[pallet::weight(<T as Config>::WeightInfo::validate_block())]
        pub fn validate_block(
            origin: OriginFor<T>,
            block_number: u32,
            pq_signature: Option<BoundedVec<u8, ConstU32<4627>>>,
        ) -> DispatchResult {
            let validator = ensure_signed(origin)?;
            ensure!(
                CurrentPhase::<T>::get() == ConsensusPhase::PosValidation,
                Error::<T>::InvalidPhaseTransition
            );
            ensure!(
                ValidatorStakes::<T>::contains_key(&validator),
                Error::<T>::NotAValidator
            );
            ensure!(
                PendingValidationBlock::<T>::get() == Some(block_number),
                Error::<T>::InvalidPhaseTransition
            );
            // Only the validator selected (and stored) at submit time may validate.
            ensure!(
                PendingValidator::<T>::get().as_ref() == Some(&validator),
                Error::<T>::NotSelectedValidator
            );

            let block_header =
                BlockHeaders::<T>::get(block_number).ok_or(Error::<T>::BlockNotFound)?;

            // Real post-quantum attestation. If the validator has registered an
            // ML-DSA (FIPS 204) key, they MUST provide a signature over the canonical
            // header message, which is verified on-chain. Validators without a
            // registered key use the transitional commitment path.
            let commitment = if let Some(algorithm) = ValidatorMlDsaAlgo::<T>::get(&validator) {
                let public_key =
                    ValidatorMlDsaKey::<T>::get(&validator).ok_or(Error::<T>::MlDsaNotRegistered)?;
                let signature = pq_signature
                    .as_ref()
                    .ok_or(Error::<T>::MlDsaSignatureMissing)?;
                // Bind the signature to the immutable header fields + the validator id.
                // Excludes `validator_signature` (mutated by this call) and ties the
                // signature to this specific block so it cannot be replayed elsewhere.
                let message = (
                    block_header.number,
                    block_header.parent_hash,
                    block_header.state_root,
                    block_header.extrinsics_root,
                    block_header.nonce,
                    block_header.difficulty,
                    validator.clone(),
                )
                    .encode();
                ensure!(
                    pq_verify::verify_ml_dsa(
                        &algorithm,
                        public_key.as_slice(),
                        &message,
                        signature.as_slice(),
                        GHOST_VALIDATOR_CTX,
                    ),
                    Error::<T>::MlDsaSignatureInvalid
                );
                // Compact on-chain commitment proving a valid PQ signature was verified.
                BlakeTwo256::hash_of(signature)
            } else {
                BlakeTwo256::hash_of(&("ghost-validator-v1", validator.clone(), block_number))
            };

            let weight = ValidatorStakes::<T>::get(&validator)
                .unwrap_or_default()
                .saturated_into::<u64>();

            let mut signed_header = block_header;
            signed_header.validator_signature = Some(commitment);
            BlockHeaders::<T>::insert(block_number, signed_header);
            BlockValidators::<T>::insert(block_number, validator.clone());

            let reward = calculate_block_reward::<T>(T::BlockReward::get());
            let miner = Self::get_miner_for_block(block_number)?;
            let stakers = Self::all_stakers();
            distribute_rewards::<T>(miner.clone(), stakers, reward.clone())?;

            LastActiveBlock::<T>::insert(&validator, block_number);
            CurrentPhase::<T>::put(ConsensusPhase::Finalization);

            Self::deposit_event(Event::ValidatorSelected { validator, weight });
            Self::deposit_event(Event::RewardsDistributed {
                miner,
                miner_reward: reward.miner_reward,
                stakers_reward: reward.stakers_reward,
            });

            Ok(())
        }

        #[pallet::call_index(4)]
        #[pallet::weight(<T as Config>::WeightInfo::report_misbehavior())]
        pub fn report_misbehavior(
            origin: OriginFor<T>,
            validator: T::AccountId,
            reason: SlashingReason,
            evidence: MisbehaviorEvidence,
        ) -> DispatchResult {
            let _reporter = ensure_signed(origin)?;
            let current_stake =
                ValidatorStakes::<T>::get(&validator).ok_or(Error::<T>::NotAValidator)?;
            validate_misbehavior_evidence::<T>(&validator, &reason, &evidence)?;

            match reason {
                SlashingReason::DoubleSigning => DoubleSignReports::<T>::insert(&validator, true),
                SlashingReason::InvalidBlock => InvalidBlockReports::<T>::insert(&validator, true),
                SlashingReason::Downtime | SlashingReason::Other => {}
            }

            let slash_percentage = match reason {
                SlashingReason::DoubleSigning => T::DoubleSignSlashPercentage::get(),
                SlashingReason::InvalidBlock => T::InvalidBlockSlashPercentage::get(),
                SlashingReason::Downtime => T::DowntimeSlashPercentage::get(),
                SlashingReason::Other => 10,
            };

            // Saturating multiply: a naive `current_stake * pct` overflows u128 for
            // large stakes (panics in debug, wraps in release).
            let slash_amount = current_stake.saturating_mul(slash_percentage.into()) / 100u32.into();

            // Record the slash first so a full records buffer fails before any state
            // change (the dispatch is transactional, but this keeps intent explicit).
            let mut records = SlashingRecords::<T>::get();
            records
                .try_push((
                    validator.clone(),
                    reason.clone(),
                    slash_amount,
                    frame_system::Pallet::<T>::block_number(),
                ))
                .map_err(|_| Error::<T>::TooManySlashingRecords)?;
            SlashingRecords::<T>::put(records);

            Self::reduce_stake_and_burn(&validator, slash_amount);

            Self::deposit_event(Event::ValidatorSlashed {
                validator,
                reason,
                amount: slash_amount,
            });

            Ok(())
        }

        #[pallet::call_index(5)]
        #[pallet::weight(<T as Config>::WeightInfo::register_pq_readiness(
            metadata.encoded_size().saturated_into::<u32>()
        ))]
        pub fn register_pq_readiness(
            origin: OriginFor<T>,
            metadata: PqReadinessMetadata<BlockNumberFor<T>>,
        ) -> DispatchResult {
            let account = ensure_signed(origin)?;
            validate_pq_readiness_metadata::<T>(&metadata)?;

            PqReadinessRegistry::<T>::insert(&account, metadata.clone());

            Self::deposit_event(Event::PqReadinessRegistered {
                account,
                algorithm: metadata.algorithm,
                proof_kind: metadata.proof_kind,
            });

            Ok(())
        }

        #[pallet::call_index(6)]
        #[pallet::weight(<T as Config>::WeightInfo::attest_pq_readiness(
            proof.encoded_size().saturated_into::<u32>()
        ))]
        pub fn attest_pq_readiness(
            origin: OriginFor<T>,
            proof: DefaultPqProof<BlockNumberFor<T>>,
        ) -> DispatchResult {
            let account = ensure_signed(origin)?;
            let metadata =
                PqReadinessRegistry::<T>::get(&account).ok_or(Error::<T>::PqReadinessNotFound)?;
            validate_pq_proof_envelope::<T>(&metadata, &proof)?;

            PqReadinessAttestations::<T>::insert(&account, proof.clone());

            Self::deposit_event(Event::PqReadinessAttested {
                account,
                algorithm: proof.algorithm,
                proof_kind: proof.proof_kind,
                statement_hash: proof.statement_hash,
            });

            Ok(())
        }

        #[pallet::call_index(7)]
        #[pallet::weight(<T as Config>::WeightInfo::remove_pq_readiness())]
        pub fn remove_pq_readiness(origin: OriginFor<T>) -> DispatchResult {
            let account = ensure_signed(origin)?;

            PqReadinessRegistry::<T>::remove(&account);
            PqReadinessAttestations::<T>::remove(&account);

            Self::deposit_event(Event::PqReadinessRemoved { account });

            Ok(())
        }

        /// Register an ML-DSA (FIPS 204) public key for the caller. The key bytes are
        /// validated for the correct length and a decodable FIPS 204 encoding before
        /// being stored. Once registered, the caller's `validate_block` attestations
        /// and `verify_pq_signature` calls are checked against this key. ML-DSA-87 is
        /// "Dilithium-5" (NIST security level 5).
        #[pallet::call_index(8)]
        #[pallet::weight(<T as Config>::WeightInfo::register_ml_dsa_key(public_key.len() as u32))]
        pub fn register_ml_dsa_key(
            origin: OriginFor<T>,
            algorithm: PqAlgorithm,
            public_key: BoundedVec<u8, ConstU32<2592>>,
        ) -> DispatchResult {
            let account = ensure_signed(origin)?;
            ensure!(
                pq_verify::validate_ml_dsa_pk(&algorithm, public_key.as_slice()),
                Error::<T>::MlDsaKeyInvalid
            );
            ValidatorMlDsaKey::<T>::insert(&account, public_key);
            ValidatorMlDsaAlgo::<T>::insert(&account, algorithm.clone());

            Self::deposit_event(Event::ValidatorMlDsaKeyRegistered { account, algorithm });
            Ok(())
        }

        /// Verify a real ML-DSA (FIPS 204) signature over `message` against the
        /// caller's registered public key, and record the verified attestation
        /// on-chain. This is genuine post-quantum signature verification executed
        /// inside the runtime — not a hash or byte-presence check.
        #[pallet::call_index(9)]
        #[pallet::weight(<T as Config>::WeightInfo::verify_pq_signature(signature.len() as u32))]
        pub fn verify_pq_signature(
            origin: OriginFor<T>,
            message: BoundedVec<u8, ConstU32<65536>>,
            signature: BoundedVec<u8, ConstU32<4627>>,
            ctx: BoundedVec<u8, ConstU32<255>>,
        ) -> DispatchResult {
            let attester = ensure_signed(origin)?;
            let public_key =
                ValidatorMlDsaKey::<T>::get(&attester).ok_or(Error::<T>::MlDsaNotRegistered)?;
            let algorithm =
                ValidatorMlDsaAlgo::<T>::get(&attester).ok_or(Error::<T>::MlDsaNotRegistered)?;

            ensure!(
                pq_verify::verify_ml_dsa(
                    &algorithm,
                    public_key.as_slice(),
                    message.as_slice(),
                    signature.as_slice(),
                    ctx.as_slice(),
                ),
                Error::<T>::MlDsaSignatureInvalid
            );

            // Replay guard: an attester may record each distinct statement only once.
            // Without this, an already-verified attestation could be resubmitted to spam
            // events and refresh its recorded block number while conveying no new info.
            let statement_hash = BlakeTwo256::hash_of(&message);
            ensure!(
                !PqVerifiedAttestations::<T>::contains_key((&attester, statement_hash)),
                Error::<T>::AttestationAlreadyRecorded
            );
            let now = frame_system::Pallet::<T>::block_number();
            PqVerifiedAttestations::<T>::insert((&attester, statement_hash), now);

            Self::deposit_event(Event::PqSignatureVerified {
                attester,
                algorithm,
                statement_hash,
            });
            Ok(())
        }
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        fn on_initialize(n: BlockNumberFor<T>) -> Weight {
            if (n % 10u32.into()).is_zero() {
                Self::check_downtime_slashing();
            }

            Self::check_validation_timeout();

            let period: BlockNumberFor<T> = T::DifficultyAdjustmentPeriod::get().max(1).into();
            if (n % period).is_zero() {
                Self::adjust_difficulty();
            }

            // Bound on-chain cost: downtime slashing scans up to MaxValidators entries
            // and the difficulty retarget touches a few storage items.
            T::DbWeight::get().reads_writes(
                (T::MaxValidators::get() as u64).saturating_add(4),
                (T::MaxValidators::get() as u64).saturating_add(4),
            )
        }

        fn on_finalize(_n: BlockNumberFor<T>) {
            if CurrentPhase::<T>::get() == ConsensusPhase::Finalization {
                if let Some(finalized) = PendingValidationBlock::<T>::get() {
                    // Prune history beyond the retention window to bound state growth.
                    if let Some(old) = finalized.checked_sub(HEADER_RETENTION) {
                        BlockHeaders::<T>::remove(old);
                        BlockMiners::<T>::remove(old);
                        BlockValidators::<T>::remove(old);
                    }
                }
                PendingValidationBlock::<T>::kill();
                PendingValidator::<T>::kill();
                PhaseStartedAt::<T>::kill();
                CurrentPhase::<T>::put(ConsensusPhase::PowMining);
            }
        }
    }
}

impl<T: Config> Pallet<T> {
    fn account_id() -> T::AccountId {
        T::PalletId::get().into_account_truncating()
    }

    /// Reduce a validator's recorded stake by `slash_amount` and burn the
    /// corresponding tokens from the pallet's staking account (the tokens were
    /// moved into the pallet account when the validator staked). Burning reduces
    /// total issuance, so slashed value is destroyed rather than silently locked
    /// in the pallet account forever.
    fn reduce_stake_and_burn(validator: &T::AccountId, slash_amount: BalanceOf<T>) {
        let current = ValidatorStakes::<T>::get(validator).unwrap_or_default();
        let new_stake = current.saturating_sub(slash_amount);
        let burned = current.saturating_sub(new_stake);
        if new_stake.is_zero() {
            ValidatorStakes::<T>::remove(validator);
            ValidatorCount::<T>::mutate(|count| *count = count.saturating_sub(1));
            // Drop the activity record so check_downtime_slashing stops scanning a ghost entry.
            LastActiveBlock::<T>::remove(validator);
        } else {
            ValidatorStakes::<T>::insert(validator, new_stake);
        }
        let _ = <pallet_balances::Pallet<T> as Currency<T::AccountId>>::withdraw(
            &Self::account_id(),
            burned,
            WithdrawReasons::all(),
            ExistenceRequirement::AllowDeath,
        );
    }

    fn all_stakers() -> Vec<ValidatorStake<T::AccountId, BalanceOf<T>>> {
        ValidatorStakes::<T>::iter()
            .map(|(account, stake)| ValidatorStake {
                account,
                stake,
                weight: stake.saturated_into(),
            })
            .collect()
    }

    fn get_miner_for_block(block_number: u32) -> Result<T::AccountId, Error<T>> {
        BlockMiners::<T>::get(block_number).ok_or(Error::<T>::BlockNotFound)
    }

    /// Finalize a freshly submitted PoW block when there are no stakers to run PoS
    /// validation. The miner receives the entire block reward (`distribute_rewards`
    /// pays the whole reward to the miner when the staker set is empty), old history is
    /// pruned exactly as `on_finalize` does for the validated path, and the chain stays
    /// in the PoW mining phase. This stops a block from stalling in PosValidation (and
    /// having to be rescued by the validation timeout) whenever the staker set is empty.
    fn finalize_without_validator(block_number: u32, miner: &T::AccountId) -> DispatchResult {
        let reward = calculate_block_reward::<T>(T::BlockReward::get());
        distribute_rewards::<T>(miner.clone(), Vec::new(), reward.clone())?;

        if let Some(old) = block_number.checked_sub(HEADER_RETENTION) {
            BlockHeaders::<T>::remove(old);
            BlockMiners::<T>::remove(old);
            BlockValidators::<T>::remove(old);
        }

        // The whole reward went to the miner; report it accurately (no staker share).
        Self::deposit_event(Event::RewardsDistributed {
            miner: miner.clone(),
            miner_reward: reward.total,
            stakers_reward: Zero::zero(),
        });
        Ok(())
    }

    pub fn check_downtime_slashing() {
        let current_block = frame_system::Pallet::<T>::block_number().saturated_into::<u32>();
        let max_downtime = T::MaxDowntimeBlocks::get();

        for (validator, last_active) in LastActiveBlock::<T>::iter() {
            if current_block.saturating_sub(last_active) > max_downtime {
                let stake = match ValidatorStakes::<T>::get(&validator) {
                    Some(stake) => stake,
                    None => continue,
                };

                let slash_amount =
                    stake.saturating_mul(T::DowntimeSlashPercentage::get().into()) / 100u32.into();
                let mut records = SlashingRecords::<T>::get();
                if records
                    .try_push((
                        validator.clone(),
                        SlashingReason::Downtime,
                        slash_amount,
                        frame_system::Pallet::<T>::block_number(),
                    ))
                    .is_ok()
                {
                    Self::reduce_stake_and_burn(&validator, slash_amount);
                    SlashingRecords::<T>::put(records);
                    Self::deposit_event(Event::ValidatorSlashed {
                        validator,
                        reason: SlashingReason::Downtime,
                        amount: slash_amount,
                    });
                }
            }
        }
    }

    /// Retarget PoW difficulty toward `TargetBlockTime` using the real elapsed time
    /// (from `pallet_timestamp`) and block count since the last retarget. Difficulty
    /// is clamped to at most a 4x change per retarget to damp oscillation.
    pub fn adjust_difficulty() {
        let now = pallet_timestamp::Pallet::<T>::get().saturated_into::<u64>();
        let current_block = frame_system::Pallet::<T>::block_number().saturated_into::<u32>();
        let last_block = LastRetargetBlock::<T>::get();
        let last_moment = LastRetargetMoment::<T>::get();

        // First observation just establishes the baseline; we cannot measure a rate yet.
        if last_block == 0 && last_moment == 0 {
            LastRetargetBlock::<T>::put(current_block);
            LastRetargetMoment::<T>::put(now);
            return;
        }

        let blocks_elapsed = current_block.saturating_sub(last_block);
        let time_elapsed = now.saturating_sub(last_moment);
        if blocks_elapsed == 0 || time_elapsed == 0 {
            return;
        }

        let actual_block_time = (time_elapsed / blocks_elapsed as u64).max(1);
        let target_block_time = T::TargetBlockTime::get().max(1);
        let current_difficulty = Difficulty::<T>::get();
        let proposed =
            calculate_difficulty_adjustment::<T>(current_difficulty, actual_block_time, target_block_time);

        // Clamp to [current/4, current*4] so a single retarget cannot swing wildly.
        let min_difficulty = (current_difficulty / 4).max(1);
        let max_difficulty = current_difficulty.saturating_mul(4).max(1);
        let new_difficulty = proposed.clamp(min_difficulty, max_difficulty);

        LastRetargetBlock::<T>::put(current_block);
        LastRetargetMoment::<T>::put(now);

        if new_difficulty != current_difficulty {
            Difficulty::<T>::put(new_difficulty);
            Self::deposit_event(Event::DifficultyAdjusted {
                old_difficulty: current_difficulty,
                new_difficulty,
            });
        }
    }

    pub fn check_validation_timeout() {
        if CurrentPhase::<T>::get() != ConsensusPhase::PosValidation {
            return;
        }

        let Some(block_number) = PendingValidationBlock::<T>::get() else {
            PendingValidator::<T>::kill();
            PhaseStartedAt::<T>::kill();
            CurrentPhase::<T>::put(ConsensusPhase::PowMining);
            return;
        };

        let current_block = frame_system::Pallet::<T>::block_number().saturated_into::<u32>();
        if current_block.saturating_sub(PhaseStartedAt::<T>::get()) <= T::MaxValidationBlocks::get()
        {
            return;
        }

        PendingValidationBlock::<T>::kill();
        PendingValidator::<T>::kill();
        PhaseStartedAt::<T>::kill();
        CurrentPhase::<T>::put(ConsensusPhase::PowMining);
        Self::deposit_event(Event::ValidationTimedOut { block_number });
    }
}

pub trait WeightInfo {
    fn submit_block() -> Weight;
    fn stake() -> Weight;
    fn unstake() -> Weight;
    fn validate_block() -> Weight;
    fn report_misbehavior() -> Weight;
    fn register_pq_readiness(metadata_len: u32) -> Weight;
    fn attest_pq_readiness(proof_len: u32) -> Weight;
    fn remove_pq_readiness() -> Weight;
    fn register_ml_dsa_key(key_len: u32) -> Weight;
    fn verify_pq_signature(sig_len: u32) -> Weight;
}

impl WeightInfo for () {
    fn submit_block() -> Weight {
        Weight::from_parts(10_000, 0)
    }

    fn stake() -> Weight {
        Weight::from_parts(10_000, 0)
    }

    fn unstake() -> Weight {
        Weight::from_parts(10_000, 0)
    }

    fn validate_block() -> Weight {
        Weight::from_parts(10_000, 0)
    }

    fn report_misbehavior() -> Weight {
        Weight::from_parts(10_000, 0)
    }

    fn register_pq_readiness(metadata_len: u32) -> Weight {
        Weight::from_parts(50_000, 3_000)
            .saturating_add(Weight::from_parts(1_500, 0).saturating_mul(metadata_len.into()))
    }

    fn attest_pq_readiness(proof_len: u32) -> Weight {
        Weight::from_parts(80_000, 6_000)
            .saturating_add(Weight::from_parts(2_000, 0).saturating_mul(proof_len.into()))
    }

    fn remove_pq_readiness() -> Weight {
        Weight::from_parts(45_000, 4_000)
    }

    fn register_ml_dsa_key(key_len: u32) -> Weight {
        Weight::from_parts(60_000, 3_000)
            .saturating_add(Weight::from_parts(200, 0).saturating_mul(key_len.into()))
    }

    fn verify_pq_signature(sig_len: u32) -> Weight {
        // ML-DSA-87 verification dominates; conservative placeholder until benchmarked.
        Weight::from_parts(8_000_000, 0)
            .saturating_add(Weight::from_parts(300, 0).saturating_mul(sig_len.into()))
    }
}
