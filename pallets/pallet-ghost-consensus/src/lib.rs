//! # Ghost Consensus Pallet
//!
//! This pallet implements a hybrid Proof-of-Work (PoW) and Proof-of-Stake (PoS) consensus mechanism
//! for the Ghost blockchain. It combines the security of PoW with the efficiency of PoS.
//!
//! ## Overview
//!
//! The Ghost consensus mechanism works as follows:
//! 1. Miners perform PoW to find a valid nonce
//! 2. Validators are selected based on their stake weight
//! 3. Selected validators sign the PoW-mined blocks
//! 4. Rewards are distributed: 40% to miner, 60% to stakers
//! 5. Slashing for misbehavior (double-signing, invalid blocks, downtime)

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

pub mod types;
pub mod functions;
pub mod consensus;
pub mod rpc;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;
#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

use frame_support::pallet_prelude::*;
use frame_system::pallet_prelude::*;
use sp_core::H256;
use sp_runtime::traits::{BlakeTwo256, Hash, SaturatedConversion};

use crate::types::*;
use crate::functions::*;

#[frame_support::pallet]
pub mod pallet {
	use super::*;

	/// The pallet's configuration trait.
	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_balances::Config {
		/// The overarching runtime event type.
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// Weight information for extrinsics.
		type WeightInfo: WeightInfo;

		/// Block reward amount
		#[pallet::constant]
		type BlockReward: Get<BalanceOf<Self>>;

		/// Minimum stake required to participate in PoS
		#[pallet::constant]
		type MinStake: Get<BalanceOf<Self>>;

		/// Maximum downtime blocks before slashing
		#[pallet::constant]
		type MaxDowntimeBlocks: Get<u32>;

		/// Double sign slash percentage
		#[pallet::constant]
		type DoubleSignSlashPercentage: Get<u8>;

		/// Invalid block slash percentage
		#[pallet::constant]
		type InvalidBlockSlashPercentage: Get<u8>;

		/// Downtime slash percentage
		#[pallet::constant]
		type DowntimeSlashPercentage: Get<u8>;

		/// The pallet ID
		#[pallet::constant]
		type PalletId: Get<frame_support::PalletId>;
	}

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	/// Current mining difficulty
	#[pallet::storage]
	pub type Difficulty<T> = StorageValue<_, u64, ValueQuery>;

	/// Current consensus phase
	#[pallet::storage]
	pub type CurrentPhase<T> = StorageValue<_, ConsensusPhase, ValueQuery>;

	/// Block headers storage
	#[pallet::storage]
	pub type BlockHeaders<T> = StorageMap<_, Blake2_128Concat, u32, GhostBlockHeader>;

	/// Validator stakes
	#[pallet::storage]
	pub type ValidatorStakes<T> = StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>>;

	/// Last active block for validators
	#[pallet::storage]
	pub type LastActiveBlock<T> = StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

	/// Double sign reports
	#[pallet::storage]
	pub type DoubleSignReports<T> = StorageMap<_, Blake2_128Concat, T::AccountId, bool, ValueQuery>;

	/// Invalid block reports
	#[pallet::storage]
	pub type InvalidBlockReports<T> = StorageMap<_, Blake2_128Concat, T::AccountId, bool, ValueQuery>;

	/// Slashing records
	#[pallet::storage]
	pub type SlashingRecords<T> = StorageValue<_, Vec<(T::AccountId, SlashingReason, BalanceOf<T>, BlockNumberFor<T>)>, ValueQuery>;

	/// Events that functions in this pallet can emit.
	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// A new block has been mined
		BlockMined {
			block_number: u32,
			miner: T::AccountId,
			nonce: u64,
		},
		/// A validator has been selected for PoS
		ValidatorSelected {
			validator: T::AccountId,
			weight: u64,
		},
		/// Block rewards have been distributed
		RewardsDistributed {
			miner: T::AccountId,
			miner_reward: BalanceOf<T>,
			stakers_reward: BalanceOf<T>,
		},
		/// A validator has been slashed
		ValidatorSlashed {
			validator: T::AccountId,
			reason: SlashingReason,
			amount: BalanceOf<T>,
		},
		/// Difficulty has been adjusted
		DifficultyAdjusted {
			old_difficulty: u64,
			new_difficulty: u64,
		},
	}

	/// Errors that can be returned by this pallet.
	#[pallet::error]
	pub enum Error<T> {
		/// Invalid block number
		InvalidBlockNumber,
		/// Invalid parent hash
		InvalidParentHash,
		/// Invalid PoW
		InvalidPow,
		/// Difficulty too low
		DifficultyTooLow,
		/// Difficulty too high
		DifficultyTooHigh,
		/// Insufficient stake
		InsufficientStake,
		/// Not a validator
		NotAValidator,
		/// Block not found
		BlockNotFound,
		/// Invalid phase transition
		InvalidPhaseTransition,
		/// Invalid PQC signature
		InvalidPqcSignature,
		/// PQC public key not found
		PqcPublicKeyNotFound,
	}

	/// PQC Public Keys for validators
	#[pallet::storage]
	pub type ValidatorPqcPublicKeys<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, [u8; 2592]>;

	/// Rolling window of the last 100 block producers
	#[pallet::storage]
	pub type RecentBlockProducers<T: Config> = StorageValue<_, BoundedVec<T::AccountId, ConstU32<100>>, ValueQuery>;

	/// Current entropy value (scaled by 10^6)
	#[pallet::storage]
	pub type CurrentEntropy<T> = StorageValue<_, u64, ValueQuery>;

	/// The pallet's dispatchable functions.
	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Submit a mined block with PoW solution
		#[pallet::call_index(0)]
		#[pallet::weight(T::WeightInfo::submit_block())]
		pub fn submit_block(
			origin: OriginFor<T>,
			block_header: GhostBlockHeader,
		) -> DispatchResult {
			let miner = ensure_signed(origin)?;

			// Validate the block header
			let parent_header = BlockHeaders::<T>::get(block_header.number - 1)
				.ok_or(Error::<T>::BlockNotFound)?;

			validate_block_header::<T>(&block_header, &parent_header)?;

			// Store the block header
			BlockHeaders::<T>::insert(block_header.number, block_header.clone());

			// Update recent block producers
			let mut recent = RecentBlockProducers::<T>::get();
			if recent.len() >= 100 {
				recent.remove(0);
			}
			let _ = recent.try_push(miner.clone());
			RecentBlockProducers::<T>::put(recent);

			// Update last active block for miner
			LastActiveBlock::<T>::insert(&miner, block_header.number);

			// Transition to PoS validation phase
			CurrentPhase::<T>::put(ConsensusPhase::PosValidation);

			Self::deposit_event(Event::BlockMined {
				block_number: block_header.number,
				miner: miner.clone(),
				nonce: block_header.nonce,
			});

			Ok(())
		}

		/// Stake tokens to participate in PoS validation
		#[pallet::call_index(1)]
		#[pallet::weight(T::WeightInfo::stake())]
		pub fn stake(
			origin: OriginFor<T>,
			amount: BalanceOf<T>,
		) -> DispatchResult {
			let staker = ensure_signed(origin)?;

			ensure!(amount >= T::MinStake::get(), Error::<T>::InsufficientStake);

			// Transfer tokens to stake
			pallet_balances::Pallet::<T>::transfer(
				RawOrigin::Signed(staker.clone()).into(),
				staker.clone(),
				Self::account_id(),
				amount,
			)?;

			// Update stake
			let current_stake = ValidatorStakes::<T>::get(&staker).unwrap_or_default();
			ValidatorStakes::<T>::insert(&staker, current_stake + amount);

			Ok(())
		}

		/// Unstake tokens
		#[pallet::call_index(2)]
		#[pallet::weight(T::WeightInfo::unstake())]
		pub fn unstake(
			origin: OriginFor<T>,
			amount: BalanceOf<T>,
		) -> DispatchResult {
			let staker = ensure_signed(origin)?;

			let current_stake = ValidatorStakes::<T>::get(&staker)
				.ok_or(Error::<T>::NotAValidator)?;

			ensure!(current_stake >= amount, Error::<T>::InsufficientStake);

			// Update stake
			ValidatorStakes::<T>::insert(&staker, current_stake - amount);

			// Transfer tokens back
			pallet_balances::Pallet::<T>::transfer(
				RawOrigin::Signed(Self::account_id()).into(),
				Self::account_id(),
				staker,
				amount,
			)?;

			Ok(())
		}

		/// Select and validate PoS validator
		#[pallet::call_index(3)]
		#[pallet::weight(T::WeightInfo::validate_block())]
		pub fn validate_block(
			origin: OriginFor<T>,
			block_number: u32,
			pqc_signature: PqcSignature,
		) -> DispatchResult {
			let validator = ensure_signed(origin)?;

			// Check if validator has stake
			ensure!(ValidatorStakes::<T>::contains_key(&validator), Error::<T>::NotAValidator);

			let block_header = BlockHeaders::<T>::get(block_number)
				.ok_or(Error::<T>::BlockNotFound)?;

			// Ensure we are in PoS validation phase
			ensure!(CurrentPhase::<T>::get() == ConsensusPhase::PosValidation, Error::<T>::InvalidPhaseTransition);

			// Select validator based on stake
			let stakers = ValidatorStakes::<T>::iter()
				.map(|(account, stake)| ValidatorStake {
					account,
					stake,
					weight: stake.saturated_into(),
				})
				.collect::<Vec<_>>();

			let seed = BlakeTwo256::hash_of(&(block_number, frame_system::Pallet::<T>::block_number()));
			let selection = select_pos_validator::<T>(stakers, seed)
				.ok_or(Error::<T>::NotAValidator)?;

			ensure!(selection.validator == validator, Error::<T>::NotAValidator);

			// Verify PQC Signature
			let pqc_public_key = ValidatorPqcPublicKeys::<T>::get(&validator)
				.ok_or(Error::<T>::PqcPublicKeyNotFound)?;

			let message = BlakeTwo256::hash_of(&(validator.clone(), block_number));
			ensure!(
				verify_pqc_signature(message.as_bytes(), &pqc_signature.0, &pqc_public_key),
				Error::<T>::InvalidPqcSignature
			);

			// Sign the block
			let signature = BlakeTwo256::hash_of(&(validator.clone(), block_number));
			let mut signed_header = block_header;
			signed_header.validator_signature = Some(signature);
			signed_header.pqc_signature = Some(pqc_signature);

			BlockHeaders::<T>::insert(block_number, signed_header);

			// Distribute rewards
			let reward = calculate_block_reward::<T>(T::BlockReward::get());
			let miner = Self::get_miner_for_block(block_number)?;
			let stakers_list = ValidatorStakes::<T>::iter()
				.map(|(account, stake)| ValidatorStake {
					account,
					stake,
					weight: stake.saturated_into(),
				})
				.collect();

			distribute_rewards::<T>(miner.clone(), stakers_list, reward.clone())?;

			// Update last active block
			LastActiveBlock::<T>::insert(&validator, block_number);

			// Transition to finalization phase
			CurrentPhase::<T>::put(ConsensusPhase::Finalization);

			Self::deposit_event(Event::ValidatorSelected {
				validator,
				weight: selection.weight,
			});

			Self::deposit_event(Event::RewardsDistributed {
				miner,
				miner_reward: reward.miner_reward,
				stakers_reward: reward.stakers_reward,
			});

			Ok(())
		}

		/// Register PQC public key
		#[pallet::call_index(5)]
		#[pallet::weight(T::WeightInfo::register_pqc_key())]
		pub fn register_pqc_key(
			origin: OriginFor<T>,
			public_key: [u8; 2592],
		) -> DispatchResult {
			let validator = ensure_signed(origin)?;
			ValidatorPqcPublicKeys::<T>::insert(&validator, public_key);
			Ok(())
		}

		/// Report validator misbehavior
		#[pallet::call_index(4)]
		#[pallet::weight(T::WeightInfo::report_misbehavior())]
		pub fn report_misbehavior(
			origin: OriginFor<T>,
			validator: T::AccountId,
			reason: SlashingReason,
		) -> DispatchResult {
			let _reporter = ensure_signed(origin)?;

			match reason {
				SlashingReason::DoubleSigning => {
					DoubleSignReports::<T>::insert(&validator, true);
				},
				SlashingReason::InvalidBlock => {
					InvalidBlockReports::<T>::insert(&validator, true);
				},
				_ => {},
			}

			// Apply slashing - simplified for now
			let slash_percentage = match reason {
				SlashingReason::DoubleSigning => T::DoubleSignSlashPercentage::get(),
				SlashingReason::InvalidBlock => T::InvalidBlockSlashPercentage::get(),
				SlashingReason::Downtime => T::DowntimeSlashPercentage::get(),
				SlashingReason::Other => 10,
			};

			let validator_balance = pallet_balances::Pallet::<T>::get(&validator);
			let slash_amount = (validator_balance * slash_percentage.into()) / 100u32.into();

			pallet_balances::Pallet::<T>::mutate(&validator, |balance| {
				*balance -= slash_amount;
			});

			Self::deposit_event(Event::ValidatorSlashed {
				validator,
				reason,
				amount: slash_amount,
			});

			Ok(())
		}
	}

	/// Helper functions
	impl<T: Config> Pallet<T> {
		/// Get the pallet's account ID
		fn account_id() -> T::AccountId {
			T::PalletId::get().into_account_truncating()
		}

		/// Get miner for a block
		fn get_miner_for_block(block_number: u32) -> Result<T::AccountId, Error<T>> {
			// This would need to be implemented based on how miners are tracked
			// For now, return a placeholder
			Err(Error::<T>::BlockNotFound)
		}

		/// Distribute block rewards to miner and stakers
		pub fn distribute_block_rewards(
			miner: T::AccountId,
			block_number: u32,
		) -> DispatchResult {
			let total_reward = T::BlockReward::get();
			let reward = calculate_block_reward::<T>(total_reward);

			// Reward the miner (40%)
			let _ = pallet_balances::Pallet::<T>::deposit_creating(&miner, reward.miner_reward);

			// Collect all stakers
			let stakers: Vec<ValidatorStake<T::AccountId, BalanceOf<T>>> = ValidatorStakes::<T>::iter()
				.map(|(account, stake)| ValidatorStake {
					account,
					stake,
					weight: stake.saturated_into(),
				})
				.collect();

			// Distribute to stakers proportionally (60%)
			if !stakers.is_empty() {
				let total_stake: BalanceOf<T> = stakers.iter().map(|s| s.stake).sum();
				if !total_stake.is_zero() {
					for staker in stakers {
						let staker_reward = (reward.stakers_reward * staker.stake) / total_stake;
						let _ = pallet_balances::Pallet::<T>::deposit_creating(&staker.account, staker_reward);
					}
				}
			}

			Self::deposit_event(Event::RewardsDistributed {
				miner,
				miner_reward: reward.miner_reward,
				stakers_reward: reward.stakers_reward,
			});

			Ok(())
		}

		/// Check and apply slashing for downtime
		pub fn check_downtime_slashing() {
			let current_block = frame_system::Pallet::<T>::block_number().saturated_into::<u32>();
			let max_downtime = T::MaxDowntimeBlocks::get();

			for (validator, last_active) in LastActiveBlock::<T>::iter() {
				if current_block.saturating_sub(last_active) > max_downtime {
					// Apply downtime slashing
					let slash_percentage = T::DowntimeSlashPercentage::get();
					if let Some(stake) = ValidatorStakes::<T>::get(&validator) {
						let slash_amount = (stake * slash_percentage.into()) / 100u32.into();
						let new_stake = stake.saturating_sub(slash_amount);
						ValidatorStakes::<T>::insert(&validator, new_stake);

						// Record slashing
						let mut records = SlashingRecords::<T>::get();
						records.push((
							validator.clone(),
							SlashingReason::Downtime,
							slash_amount,
							frame_system::Pallet::<T>::block_number(),
						));
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

		/// Adjust difficulty based on block time and entropy
		pub fn adjust_difficulty() {
			let current_difficulty = Difficulty::<T>::get();
			let target_block_time = 5u64; // 5 seconds

			// In a real implementation, calculate actual block time from recent blocks
			// For now, use a simple adjustment
			let actual_block_time = 5u64; // Placeholder
			
			// Calculate entropy
			let producers = RecentBlockProducers::<T>::get().to_vec();
			let entropy = calculate_entropy::<T>(producers);
			CurrentEntropy::<T>::put(entropy);

			let new_difficulty = calculate_difficulty_adjustment::<T>(
				current_difficulty,
				actual_block_time,
				target_block_time,
				entropy,
			);

			if new_difficulty != current_difficulty {
				Difficulty::<T>::put(new_difficulty);
				Self::deposit_event(Event::DifficultyAdjusted {
					old_difficulty: current_difficulty,
					new_difficulty,
				});
			}
		}
	}

	/// Hooks for automatic behavior
	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		/// Called at the beginning of each block
		fn on_initialize(n: BlockNumberFor<T>) -> Weight {
			// Check for downtime slashing every 10 blocks
			if (n % 10u32.into()).is_zero() {
				Self::check_downtime_slashing();
			}

			// Adjust difficulty every 100 blocks
			if (n % 100u32.into()).is_zero() {
				Self::adjust_difficulty();
			}

			Weight::from_parts(10_000, 0)
		}

		/// Called at the end of each block
		fn on_finalize(_n: BlockNumberFor<T>) {
			// Transition back to PoW mining phase for next block
			if CurrentPhase::<T>::get() == ConsensusPhase::Finalization {
				CurrentPhase::<T>::put(ConsensusPhase::PowMining);
			}
		}
	}
}

/// Default weight info for the pallet
pub trait WeightInfo {
	fn submit_block() -> Weight;
	fn stake() -> Weight;
	fn unstake() -> Weight;
	fn validate_block() -> Weight;
	fn report_misbehavior() -> Weight;
	fn register_pqc_key() -> Weight;
}

/// Default implementation of WeightInfo
impl WeightInfo for () {
	fn submit_block() -> Weight { Weight::from_parts(10_000, 0) }
	fn stake() -> Weight { Weight::from_parts(10_000, 0) }
	fn unstake() -> Weight { Weight::from_parts(10_000, 0) }
	fn validate_block() -> Weight { Weight::from_parts(10_000, 0) }
	fn report_misbehavior() -> Weight { Weight::from_parts(10_000, 0) }
	fn register_pqc_key() -> Weight { Weight::from_parts(10_000, 0) }
}
