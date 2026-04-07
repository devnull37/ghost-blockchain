//! Experimental Ghost consensus pallet.
//!
//! This pallet models a two-phase flow:
//! 1. A miner submits a PoW-style block header.
//! 2. A stake-weighted validator finalizes that header and rewards are paid out.
//!
//! The node still authors blocks with Aura and finalizes them with GRANDPA. This pallet is
//! runtime logic that can be exercised through tests and extrinsics; it is not a custom
//! block-production engine.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

pub mod functions;
pub mod types;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

use frame_support::{
	pallet_prelude::*,
	traits::{Currency, ExistenceRequirement},
};
use frame_system::pallet_prelude::*;
use sp_runtime::traits::{
	AccountIdConversion, BlakeTwo256, Hash, SaturatedConversion, Saturating, Zero,
};

use crate::functions::{
	calculate_block_reward, calculate_difficulty_adjustment, distribute_rewards,
	select_pos_validator, validate_block_header,
};
use crate::types::{ConsensusPhase, GhostBlockHeader, SlashingReason, ValidatorStake};

pub type BalanceOf<T> = <T as pallet_balances::Config>::Balance;

#[frame_support::pallet]
pub mod pallet {
	use super::*;

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_balances::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		type WeightInfo: WeightInfo;

		#[pallet::constant]
		type BlockReward: Get<BalanceOf<Self>>;
		#[pallet::constant]
		type MinStake: Get<BalanceOf<Self>>;
		#[pallet::constant]
		type MaxDowntimeBlocks: Get<u32>;
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
		1_000_000_000_000
	}

	#[pallet::type_value]
	pub fn DefaultPhase<T: Config>() -> ConsensusPhase {
		ConsensusPhase::PowMining
	}

	#[pallet::storage]
	pub type Difficulty<T: Config> = StorageValue<_, u64, ValueQuery, DefaultDifficulty<T>>;

	#[pallet::storage]
	pub type CurrentPhase<T: Config> =
		StorageValue<_, ConsensusPhase, ValueQuery, DefaultPhase<T>>;

	#[pallet::storage]
	pub type BlockHeaders<T: Config> = StorageMap<_, Blake2_128Concat, u32, GhostBlockHeader>;

	#[pallet::storage]
	pub type BlockMiners<T: Config> = StorageMap<_, Blake2_128Concat, u32, T::AccountId>;

	#[pallet::storage]
	pub type ValidatorStakes<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>>;

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
	#[pallet::unbounded]
	pub type SlashingRecords<T: Config> = StorageValue<
		_,
		Vec<(T::AccountId, SlashingReason, BalanceOf<T>, BlockNumberFor<T>)>,
		ValueQuery,
	>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		BlockMined { block_number: u32, miner: T::AccountId, nonce: u64 },
		ValidatorSelected { validator: T::AccountId, weight: u64 },
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
		DifficultyAdjusted { old_difficulty: u64, new_difficulty: u64 },
	}

	#[pallet::error]
	pub enum Error<T> {
		InvalidBlockNumber,
		InvalidParentHash,
		InvalidPow,
		DifficultyTooLow,
		DifficultyTooHigh,
		InsufficientStake,
		NotAValidator,
		BlockNotFound,
		InvalidPhaseTransition,
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
			CurrentPhase::<T>::put(ConsensusPhase::PosValidation);

			Self::deposit_event(Event::BlockMined {
				block_number: block_header.number,
				miner,
				nonce: block_header.nonce,
			});

			Ok(())
		}

		#[pallet::call_index(1)]
		#[pallet::weight(<T as Config>::WeightInfo::stake())]
		pub fn stake(origin: OriginFor<T>, amount: BalanceOf<T>) -> DispatchResult {
			let staker = ensure_signed(origin)?;
			ensure!(amount >= T::MinStake::get(), Error::<T>::InsufficientStake);

			<pallet_balances::Pallet<T> as Currency<T::AccountId>>::transfer(
				&staker,
				&Self::account_id(),
				amount,
				ExistenceRequirement::AllowDeath,
			)?;

			let current_stake = ValidatorStakes::<T>::get(&staker).unwrap_or_default();
			ValidatorStakes::<T>::insert(&staker, current_stake.saturating_add(amount));

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
			if new_stake.is_zero() {
				ValidatorStakes::<T>::remove(&staker);
			} else {
				ValidatorStakes::<T>::insert(&staker, new_stake);
			}

			<pallet_balances::Pallet<T> as Currency<T::AccountId>>::transfer(
				&Self::account_id(),
				&staker,
				amount,
				ExistenceRequirement::AllowDeath,
			)?;

			Ok(())
		}

		#[pallet::call_index(3)]
		#[pallet::weight(<T as Config>::WeightInfo::validate_block())]
		pub fn validate_block(origin: OriginFor<T>, block_number: u32) -> DispatchResult {
			let validator = ensure_signed(origin)?;
			ensure!(
				CurrentPhase::<T>::get() == ConsensusPhase::PosValidation,
				Error::<T>::InvalidPhaseTransition
			);
			ensure!(ValidatorStakes::<T>::contains_key(&validator), Error::<T>::NotAValidator);

			let block_header = BlockHeaders::<T>::get(block_number).ok_or(Error::<T>::BlockNotFound)?;
			let stakers = Self::all_stakers();
			let seed =
				BlakeTwo256::hash_of(&(block_number, frame_system::Pallet::<T>::block_number()));
			let selection =
				select_pos_validator::<T>(stakers.clone(), seed).ok_or(Error::<T>::NotAValidator)?;
			ensure!(selection.validator == validator, Error::<T>::NotAValidator);

			let mut signed_header = block_header;
			signed_header.validator_signature = Some(BlakeTwo256::hash_of(&(validator.clone(), block_number)));
			BlockHeaders::<T>::insert(block_number, signed_header);

			let reward = calculate_block_reward::<T>(T::BlockReward::get());
			let miner = Self::get_miner_for_block(block_number)?;
			distribute_rewards::<T>(miner.clone(), stakers, reward.clone())?;

			LastActiveBlock::<T>::insert(&validator, block_number);
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

		#[pallet::call_index(4)]
		#[pallet::weight(<T as Config>::WeightInfo::report_misbehavior())]
		pub fn report_misbehavior(
			origin: OriginFor<T>,
			validator: T::AccountId,
			reason: SlashingReason,
		) -> DispatchResult {
			let _reporter = ensure_signed(origin)?;
			let current_stake =
				ValidatorStakes::<T>::get(&validator).ok_or(Error::<T>::NotAValidator)?;

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

			let slash_amount = (current_stake * slash_percentage.into()) / 100u32.into();
			let new_stake = current_stake.saturating_sub(slash_amount);
			if new_stake.is_zero() {
				ValidatorStakes::<T>::remove(&validator);
			} else {
				ValidatorStakes::<T>::insert(&validator, new_stake);
			}

			let mut records = SlashingRecords::<T>::get();
			records.push((
				validator.clone(),
				reason.clone(),
				slash_amount,
				frame_system::Pallet::<T>::block_number(),
			));
			SlashingRecords::<T>::put(records);

			Self::deposit_event(Event::ValidatorSlashed {
				validator,
				reason,
				amount: slash_amount,
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

			if (n % 100u32.into()).is_zero() {
				Self::adjust_difficulty();
			}

			Weight::from_parts(10_000, 0)
		}

		fn on_finalize(_n: BlockNumberFor<T>) {
			if CurrentPhase::<T>::get() == ConsensusPhase::Finalization {
				CurrentPhase::<T>::put(ConsensusPhase::PowMining);
			}
		}
	}
}

impl<T: Config> Pallet<T> {
	fn account_id() -> T::AccountId {
		T::PalletId::get().into_account_truncating()
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

	pub fn check_downtime_slashing() {
		let current_block = frame_system::Pallet::<T>::block_number().saturated_into::<u32>();
		let max_downtime = T::MaxDowntimeBlocks::get();

		for (validator, last_active) in LastActiveBlock::<T>::iter() {
			if current_block.saturating_sub(last_active) > max_downtime {
				let stake = match ValidatorStakes::<T>::get(&validator) {
					Some(stake) => stake,
					None => continue,
				};

				let slash_amount = (stake * T::DowntimeSlashPercentage::get().into()) / 100u32.into();
				let new_stake = stake.saturating_sub(slash_amount);
				if new_stake.is_zero() {
					ValidatorStakes::<T>::remove(&validator);
				} else {
					ValidatorStakes::<T>::insert(&validator, new_stake);
				}

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

	pub fn adjust_difficulty() {
		let current_difficulty = Difficulty::<T>::get();
		let new_difficulty =
			calculate_difficulty_adjustment::<T>(current_difficulty, 5u64, 5u64);

		if new_difficulty != current_difficulty {
			Difficulty::<T>::put(new_difficulty);
			Self::deposit_event(Event::DifficultyAdjusted {
				old_difficulty: current_difficulty,
				new_difficulty,
			});
		}
	}
}

pub trait WeightInfo {
	fn submit_block() -> Weight;
	fn stake() -> Weight;
	fn unstake() -> Weight;
	fn validate_block() -> Weight;
	fn report_misbehavior() -> Weight;
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
}
