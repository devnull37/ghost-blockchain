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
use frame_support::traits::{Currency, ReservableCurrency, ExistenceRequirement};

use crate::types::*;
use crate::functions::*;

type BalanceOf<T> = <<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

#[frame_support::pallet]
pub mod pallet {
	use super::*;

	/// The pallet's configuration trait.
	#[pallet::config]
	pub trait Config: frame_system::Config {
		/// The overarching runtime event type.
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// Weight information for extrinsics.
		type WeightInfo: WeightInfo;

		/// The currency trait.
		type Currency: Currency<Self::AccountId> + ReservableCurrency<Self::AccountId>;

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

	/// Miners of each block
	#[pallet::storage]
	pub type BlockMiners<T: Config> = StorageMap<_, Blake2_128Concat, u32, T::AccountId>;

	/// Validator stakes
	#[pallet::storage]
	pub type ValidatorStakes<T> = StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>>;

	/// Total stake in the system
	#[pallet::storage]
	pub type TotalStake<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

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
	}

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

			// Store the miner for this block
			BlockMiners::<T>::insert(block_header.number, miner.clone());

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
			T::Currency::transfer(&staker, &Self::account_id(), amount, ExistenceRequirement::KeepAlive)?;

			// Update stake
			let current_stake = ValidatorStakes::<T>::get(&staker).unwrap_or_default();
			ValidatorStakes::<T>::insert(&staker, current_stake + amount);
			TotalStake::<T>::mutate(|total| *total += amount);

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
			TotalStake::<T>::mutate(|total| *total -= amount);

			// Transfer tokens back
			T::Currency::transfer(&Self::account_id(), &staker, amount, ExistenceRequirement::KeepAlive)?;

			Ok(())
		}

		/// Select and validate PoS validator
		#[pallet::call_index(3)]
		#[pallet::weight(T::WeightInfo::validate_block())]
		pub fn validate_block(
			origin: OriginFor<T>,
			block_number: u32,
		) -> DispatchResult {
			let validator = ensure_signed(origin)?;

			// Check if validator has stake
			ensure!(ValidatorStakes::<T>::contains_key(&validator), Error::<T>::NotAValidator);

			let block_header = BlockHeaders::<T>::get(block_number)
				.ok_or(Error::<T>::BlockNotFound)?;

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

			// Sign the block
			let signature = BlakeTwo256::hash_of(&(validator.clone(), block_number));
			let mut signed_header = block_header;
			signed_header.validator_signature = Some(signature);

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

			apply_slashing::<T>(&validator, reason)?;

			Ok(())
		}
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(n: BlockNumberFor<T>) -> Weight {
			let current_block_number: u32 = n.saturated_into();
			let mut weight = Weight::from_parts(0, 0);

			// Check for validator downtime every 100 blocks
			if current_block_number % 100 == 0 {
				for (validator, _) in ValidatorStakes::<T>::iter() {
					weight = weight.saturating_add(T::DbWeight::get().reads(1));
					if let Some(reason) = check_slashing_conditions::<T>(&validator, current_block_number) {
						let _ = apply_slashing::<T>(&validator, reason);
						weight = weight.saturating_add(T::DbWeight::get().writes(1));
					}
				}
			}

			weight
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
			BlockMiners::<T>::get(block_number).ok_or(Error::<T>::BlockNotFound)
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
}

/// Default implementation of WeightInfo
impl WeightInfo for () {
	fn submit_block() -> Weight { Weight::from_parts(10_000, 0) }
	fn stake() -> Weight { Weight::from_parts(10_000, 0) }
	fn unstake() -> Weight { Weight::from_parts(10_000, 0) }
	fn validate_block() -> Weight { Weight::from_parts(10_000, 0) }
	fn report_misbehavior() -> Weight { Weight::from_parts(10_000, 0) }
}
