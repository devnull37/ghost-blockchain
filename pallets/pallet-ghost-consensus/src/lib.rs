//! # Ghost Consensus Pallet (v2)
//!
//! On-chain state for Ghost consensus per `docs/ghost-consensus-design.md`:
//!
//! * **Staking** — `bond`/`bond_extra`/`unbond`/`withdraw_unbonded` using
//!   `pallet_balances` *holds* (funds stay on the account, unspendable), with a
//!   bounded per-account unbonding-chunk queue.
//! * **Validator set** — implements `pallet_session::SessionManager` (and the
//!   `historical` specialization): each session picks the top `MaxValidators`
//!   candidates by bonded stake, tie-broken by account id.
//! * **Difficulty** — `Difficulty` is a U256 *work factor* (larger = harder;
//!   the client verifies `hash * difficulty <= U256::MAX`). `on_initialize`
//!   retargets it every `RetargetInterval` blocks toward `TargetBlockTimeMs`
//!   using `new = old * clamp(expected_ms / elapsed_ms, 1/4, 4)`.
//! * **Rewards** — per block, the miner is decoded from the seal-bound
//!   `PreRuntime(POW_ENGINE_ID, SCALE(AccountId32))` digest (unspoofable);
//!   `BlockReward` is minted 40% to the author and 60% pro-rata by bonded stake
//!   to the current session's validators (empty set -> pallet reserve account;
//!   rounding remainder -> author).
//! * **Slashing** — `OnOffenceHandler` (wired to `pallet_offences` + GRANDPA
//!   equivocation + im-online unresponsiveness in the runtime) slashes the
//!   reported fraction of bonded stake, burns it, chills the offender, and
//!   records a bounded `SlashRecord`.
//!
//! There are deliberately no block-submission or validation extrinsics: PoW
//! seal verification lives in the client (`sc-consensus-pow`), finality in
//! GRANDPA. This pallet only tracks stake, difficulty, rewards, and slashes.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod migrations;
#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;
pub mod types;
pub mod weights;

use codec::Decode;
use frame_support::{
    pallet_prelude::*,
    traits::{
        fungible::{Inspect, InspectHold, Mutate, MutateHold},
        tokens::{Fortitude, Precision},
    },
};
use frame_system::pallet_prelude::*;
use sp_core::U256;
use sp_runtime::{
    traits::{SaturatedConversion, Saturating, Zero},
    DigestItem, Perbill,
};
use sp_staking::{
    offence::{OffenceDetails, OnOffenceHandler},
    SessionIndex,
};
use sp_std::{vec, vec::Vec};

use crate::types::*;
use ghost_pow_primitives::{compute_next_difficulty, POW_ENGINE_ID};

/// Maximum size of a registered PQC (Dilithium5 / ML-DSA-87) public key.
pub const MAX_PQC_KEY_SIZE: u32 = 2592;

/// Upper bound on retained `SlashRecords` (oldest are dropped when full).
pub const MAX_SLASH_RECORDS: u32 = 1024;

/// Upper bound on `RecentAuthors` (rolling window of block authors).
pub const MAX_RECENT_AUTHORS: u32 = 100;

type BalanceOf<T> =
    <<T as Config>::Currency as Inspect<<T as frame_system::Config>::AccountId>>::Balance;

/// Source of truth for whether an account has registered session keys.
///
/// The runtime wires this to `pallet_session` (`NextKeys`); `()` is permissive
/// and returns `true` for chains that do not gate validation on session keys.
pub trait SessionKeysLookup<AccountId> {
    /// Whether `who` has session keys registered for the next session.
    fn keys_registered(who: &AccountId) -> bool;
}

impl<AccountId> SessionKeysLookup<AccountId> for () {
    fn keys_registered(_who: &AccountId) -> bool {
        true
    }
}

/// Provider of validator PQC public keys (Dilithium5 / ML-DSA-87).
///
/// Implemented by `pallet-ghost-pqc` once wired into the runtime; `()` is the
/// fallback returning `false`/`None` (no PQC keys registered).
pub trait PqcKeyProvider<AccountId> {
    /// Whether `who` has a registered PQC public key.
    fn has_pqc_key(who: &AccountId) -> bool;
    /// The registered PQC public key of `who`, if any.
    fn pqc_key(who: &AccountId) -> Option<BoundedVec<u8, ConstU32<MAX_PQC_KEY_SIZE>>>;
}

impl<AccountId> PqcKeyProvider<AccountId> for () {
    fn has_pqc_key(_who: &AccountId) -> bool {
        false
    }
    fn pqc_key(_who: &AccountId) -> Option<BoundedVec<u8, ConstU32<MAX_PQC_KEY_SIZE>>> {
        None
    }
}

/// `frame_support::traits::FindAuthor` adapter for PoW-sealed chains.
///
/// Decodes the miner `AccountId` from the first decodable
/// `PreRuntime(POW_ENGINE_ID, _)` digest — the same decode `verify` enforces
/// and `block_author` applies for rewards. Wiring it into
/// `pallet_authorship::Config::FindAuthor` lets `pallet-im-online` credit
/// authored blocks toward validator liveness (see
/// docs/security-review/round-1.md); `()` never decodes anything.
pub struct PowFindAuthor<AccountId>(core::marker::PhantomData<AccountId>);

impl<AccountId: Decode> frame_support::traits::FindAuthor<AccountId> for PowFindAuthor<AccountId> {
    fn find_author<'a, I>(digests: I) -> Option<AccountId>
    where
        I: 'a + IntoIterator<Item = (sp_runtime::ConsensusEngineId, &'a [u8])>,
    {
        digests.into_iter().find_map(|(id, data)| {
            (id == POW_ENGINE_ID)
                .then(|| AccountId::decode(&mut &*data).ok())
                .flatten()
        })
    }
}

/// Benchmark-only hook to arrange preconditions that are unreachable from
/// generic pallet code. `validate` requires session keys registered and,
/// when `RequirePqcKey` is set, a registered PQC key — neither can be
/// provisioned generically inside the pallet's own mock or runtime, so the
/// `benchmarks` module defers to this `Config` associated type.
///
/// `()` is the no-op impl for configurations whose `SessionKeysLookup` and
/// `PqcProvider` already accept any account (e.g. unit tests).
#[cfg(feature = "runtime-benchmarks")]
pub trait BenchmarkHelper<T: Config> {
    /// Arrange `validate`'s non-stake preconditions for `who` (session keys,
    /// and a PQC key when `T::RequirePqcKey` is set).
    fn prepare_validate(who: &T::AccountId);
}

/// No-op [`BenchmarkHelper`].
#[cfg(feature = "runtime-benchmarks")]
impl<T: Config> BenchmarkHelper<T> for () {
    fn prepare_validate(_who: &T::AccountId) {}
}

#[frame_support::pallet]
pub mod pallet {
    use super::*;

    /// The pallet's configuration trait.
    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_timestamp::Config {
        /// The overarching runtime event type.
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// The stakable currency (pallet_balances). Holds are taken under
        /// [`HoldReason::Staking`].
        type Currency: Mutate<Self::AccountId>
            + InspectHold<Self::AccountId, Reason = Self::RuntimeHoldReason>
            + MutateHold<Self::AccountId, Reason = Self::RuntimeHoldReason>;

        /// Runtime hold reason aggregate; must accept this pallet's
        /// [`HoldReason`].
        type RuntimeHoldReason: From<HoldReason> + Parameter + Member + Copy;

        /// Weight information for extrinsics, generated by the `benchmarks`
        /// module into `weights.rs` (`weights::SubstrateWeight<T>`).
        type WeightInfo: WeightInfo;

        /// The pallet's own account (receives the validator share of the block
        /// reward when there are no validators, as a reserve).
        #[pallet::constant]
        type PalletId: Get<frame_support::PalletId>;

        /// Amount minted per block and split 40% author / 60% validators.
        #[pallet::constant]
        type BlockReward: Get<BalanceOf<Self>>;

        /// Minimum bonded stake required to bond, or to be a validator
        /// candidate.
        #[pallet::constant]
        type MinStake: Get<BalanceOf<Self>>;

        /// Maximum total bonded stake per account.
        #[pallet::constant]
        type MaxStake: Get<BalanceOf<Self>>;

        /// Maximum number of validators in a session set.
        #[pallet::constant]
        type MaxValidators: Get<u32>;

        /// Maximum number of validator candidates tracked at once.
        #[pallet::constant]
        type MaxValidatorCandidates: Get<u32>;

        /// Number of blocks an unbonded chunk stays locked before it can be
        /// withdrawn.
        #[pallet::constant]
        type UnbondingPeriod: Get<BlockNumberFor<Self>>;

        /// Maximum number of concurrent unbonding chunks per account.
        #[pallet::constant]
        type MaxUnbondingChunks: Get<u32>;

        /// Difficulty retarget cadence, in blocks (design: 100).
        #[pallet::constant]
        type RetargetInterval: Get<u32>;

        /// Target block time in milliseconds (design: 5000).
        #[pallet::constant]
        type TargetBlockTimeMs: Get<u64>;

        /// Absolute floor for the difficulty work factor (>= 1).
        #[pallet::constant]
        type MinDifficulty: Get<U256>;

        /// Lookup for whether an account has registered session keys
        /// (required for `validate`).
        type SessionKeysLookup: SessionKeysLookup<Self::AccountId>;

        /// Provider of validator PQC public keys (`()` until
        /// `pallet-ghost-pqc` is wired into the runtime).
        type PqcProvider: PqcKeyProvider<Self::AccountId>;

        /// When true, `validate` requires a registered PQC key from
        /// `PqcProvider`. Kept false until `pallet-ghost-pqc` exists.
        #[pallet::constant]
        type RequirePqcKey: Get<bool>;

        /// Benchmark-only setup hook for `validate`'s session-keys/PQC
        /// preconditions. `()` suffices when the lookups accept any account.
        #[cfg(feature = "runtime-benchmarks")]
        type BenchmarkHelper: BenchmarkHelper<Self>;
    }

    /// Reason funds are held: bonded Ghost stake.
    #[pallet::composite_enum]
    pub enum HoldReason {
        /// Funds bonded as Ghost validator stake.
        #[codec(index = 0)]
        Staking,
    }

    #[pallet::pallet]
    #[pallet::storage_version(migrations::STORAGE_VERSION)]
    pub struct Pallet<T>(_);

    /// Current PoW difficulty — a *work factor* (larger = harder).
    ///
    /// The client verifies `hash * difficulty <= U256::MAX`; retargeted every
    /// `RetargetInterval` blocks toward `TargetBlockTimeMs`.
    #[pallet::storage]
    pub type Difficulty<T: Config> = StorageValue<_, U256, ValueQuery>;

    /// Timestamp (ms) at which the last difficulty retarget ran.
    #[pallet::storage]
    pub type LastRetargetTime<T: Config> = StorageValue<_, u64, ValueQuery>;

    /// Number of retarget intervals completed. The first boundary only
    /// establishes the baseline timestamp ("skip genesis window").
    #[pallet::storage]
    pub type RetargetsDone<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Currently bonded (held) stake per account.
    #[pallet::storage]
    pub type Bonded<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>, ValueQuery>;

    /// Per-account queue of unbonding chunks, bounded by `MaxUnbondingChunks`.
    /// Funds remain held until `withdraw_unbonded` releases matured chunks.
    #[pallet::storage]
    pub type Unbonding<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        BoundedVec<UnbondingChunk<BalanceOf<T>, BlockNumberFor<T>>, T::MaxUnbondingChunks>,
        ValueQuery,
    >;

    /// Opted-in validator candidates, bounded by `MaxValidatorCandidates`.
    /// Insertion order; re-sorted by stake at every session rotation.
    #[pallet::storage]
    pub type Candidates<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxValidatorCandidates>, ValueQuery>;

    /// Validator set planned for the next session (computed by `new_session`).
    #[pallet::storage]
    pub type PendingValidators<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxValidators>, ValueQuery>;

    /// Validator set active in the current session (promoted at
    /// `start_session`). Used for the 60% reward split.
    #[pallet::storage]
    pub type ActiveValidators<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxValidators>, ValueQuery>;

    /// Genesis-declared fallback validators, used by `new_session_genesis`
    /// when nothing is bonded yet so a dev chain can still start.
    #[pallet::storage]
    pub type InitialValidators<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxValidators>, ValueQuery>;

    /// Bounded rolling window of slashing records (newest at the end; when
    /// full the oldest record is dropped).
    #[pallet::storage]
    pub type SlashRecords<T: Config> = StorageValue<
        _,
        BoundedVec<
            SlashRecord<T::AccountId, BalanceOf<T>, BlockNumberFor<T>>,
            ConstU32<MAX_SLASH_RECORDS>,
        >,
        ValueQuery,
    >;

    /// Bounded rolling window of the last `MAX_RECENT_AUTHORS` block authors
    /// decoded from PoW pre-runtime digests (telemetry/entropy source).
    #[pallet::storage]
    pub type RecentAuthors<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, ConstU32<MAX_RECENT_AUTHORS>>, ValueQuery>;

    /// The pallet's genesis config.
    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        /// Initial PoW difficulty work factor.
        pub difficulty: U256,
        /// Accounts bonded at genesis as `(account, amount)`.
        pub stakers: Vec<(T::AccountId, BalanceOf<T>)>,
        /// Fallback validator set for `new_session_genesis` when no stakes
        /// exist yet (dev chains only).
        pub initial_validators: Vec<T::AccountId>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            Difficulty::<T>::put(self.difficulty.max(T::MinDifficulty::get()));

            let mut initial = BoundedVec::<T::AccountId, T::MaxValidators>::default();
            for v in &self.initial_validators {
                // Bounded to MaxValidators; extras are dropped.
                let _ = initial.try_push(v.clone());
            }
            InitialValidators::<T>::put(initial);

            for (who, amount) in &self.stakers {
                assert!(
                    *amount >= T::MinStake::get(),
                    "genesis staker below MinStake"
                );
                T::Currency::hold(&HoldReason::Staking.into(), who, *amount)
                    .expect("genesis staker must be able to hold");
                Bonded::<T>::insert(who, *amount);
                if !Candidates::<T>::get().contains(who) {
                    let _ = Candidates::<T>::try_mutate(|c| c.try_push(who.clone()));
                }
            }
        }
    }

    /// Events that functions in this pallet can emit.
    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// `who` bonded `added` stake (total bonded now `total`).
        Bonded {
            who: T::AccountId,
            added: BalanceOf<T>,
            total: BalanceOf<T>,
        },
        /// `who` scheduled `amount` to unbond; withdrawable at `unlock_at`.
        Unbonded {
            who: T::AccountId,
            amount: BalanceOf<T>,
            unlock_at: BlockNumberFor<T>,
        },
        /// `who` withdrew `amount` of matured unbonded stake.
        WithdrawnUnbonded {
            who: T::AccountId,
            amount: BalanceOf<T>,
        },
        /// `who` joined the validator candidate set.
        CandidateJoined { who: T::AccountId },
        /// `who` left the validator candidate set.
        Chilled { who: T::AccountId },
        /// `who` was slashed `amount` for an offence committed in `session`.
        Slashed {
            who: T::AccountId,
            amount: BalanceOf<T>,
            session_index: SessionIndex,
        },
        /// Block reward paid: `author` got `author_reward`, validators split
        /// `validators_reward` (or the reserve when the set was empty).
        BlockRewarded {
            author: T::AccountId,
            author_reward: BalanceOf<T>,
            validators_reward: BalanceOf<T>,
        },
        /// No `PreRuntime(POW_ENGINE_ID, ..)` digest, or it failed to decode:
        /// the block reward was skipped. Never panics.
        BlockRewardSkipped { block_number: BlockNumberFor<T> },
        /// Difficulty retargeted from `old` to `new`.
        DifficultyRetargeted { old: U256, new: U256 },
    }

    /// Errors that can be returned by this pallet.
    #[pallet::error]
    pub enum Error<T> {
        /// Bond amount is below `MinStake` (or a `validate`/`unbond` caller
        /// would end up below it where that is disallowed).
        BondBelowMinimum,
        /// Bond would push the account's total bonded stake above `MaxStake`.
        BondAboveMaximum,
        /// The account has no bonded stake.
        NotBonded,
        /// Unbond amount exceeds the account's bonded stake.
        InsufficientBond,
        /// `MaxUnbondingChunks` reached; wait for a chunk to mature and call
        /// `withdraw_unbonded` first.
        TooManyUnbondingChunks,
        /// `MaxValidatorCandidates` reached.
        TooManyCandidates,
        /// The account is already a validator candidate.
        AlreadyCandidate,
        /// The account is not a validator candidate.
        NotCandidate,
        /// `validate` requires session keys registered via `set_keys`.
        KeysNotRegistered,
        /// `validate` requires a registered PQC key (when `RequirePqcKey`).
        PqcKeyRequired,
        /// Nothing matured to withdraw.
        NothingToWithdraw,
    }

    /// The pallet's dispatchable functions.
    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Bond `amount` of free balance as stake (pallet_balances hold).
        ///
        /// `amount` must be >= `MinStake` on first bond; the resulting total
        /// must not exceed `MaxStake`.
        #[pallet::call_index(0)]
        #[pallet::weight(<T as Config>::WeightInfo::bond())]
        pub fn bond(origin: OriginFor<T>, amount: BalanceOf<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let bonded = Self::bonded(&who);
            ensure!(
                bonded.is_zero() && amount >= T::MinStake::get() || !bonded.is_zero(),
                Error::<T>::BondBelowMinimum
            );
            Self::do_bond(&who, amount)
        }

        /// Add `amount` to an existing bond. No minimum applies to the added
        /// amount; the resulting total must still not exceed `MaxStake`.
        #[pallet::call_index(1)]
        #[pallet::weight(<T as Config>::WeightInfo::bond_extra())]
        pub fn bond_extra(origin: OriginFor<T>, amount: BalanceOf<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(!Self::bonded(&who).is_zero(), Error::<T>::NotBonded);
            Self::do_bond(&who, amount)
        }

        /// Move `amount` of bonded stake into the unbonding queue. Funds stay
        /// held until `UnbondingPeriod` elapses and `withdraw_unbonded` runs.
        #[pallet::call_index(2)]
        #[pallet::weight(<T as Config>::WeightInfo::unbond(
            T::MaxUnbondingChunks::get().saturating_sub(1),
        ))]
        pub fn unbond(origin: OriginFor<T>, amount: BalanceOf<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let bonded = Self::bonded(&who);
            ensure!(!bonded.is_zero(), Error::<T>::NotBonded);
            ensure!(bonded >= amount, Error::<T>::InsufficientBond);
            ensure!(!amount.is_zero(), Error::<T>::InsufficientBond);

            let unlock_at =
                frame_system::Pallet::<T>::block_number().saturating_add(T::UnbondingPeriod::get());

            Unbonding::<T>::try_mutate(&who, |chunks| -> DispatchResult {
                chunks
                    .try_push(UnbondingChunk { amount, unlock_at })
                    .map_err(|_| Error::<T>::TooManyUnbondingChunks)?;
                Ok(())
            })?;

            let remaining = bonded.saturating_sub(amount);
            if remaining.is_zero() {
                Bonded::<T>::remove(&who);
            } else {
                Bonded::<T>::insert(&who, remaining);
            }

            Self::deposit_event(Event::Unbonded {
                who,
                amount,
                unlock_at,
            });
            Ok(())
        }

        /// Release all matured unbonding chunks back to the free balance.
        #[pallet::call_index(3)]
        #[pallet::weight(<T as Config>::WeightInfo::withdraw_unbonded(
            T::MaxUnbondingChunks::get(),
        ))]
        pub fn withdraw_unbonded(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let now = frame_system::Pallet::<T>::block_number();

            let mut matured = BalanceOf::<T>::zero();
            Unbonding::<T>::mutate(&who, |chunks| {
                chunks.retain(|chunk| {
                    if chunk.unlock_at <= now {
                        matured = matured.saturating_add(chunk.amount);
                        false
                    } else {
                        true
                    }
                });
            });

            ensure!(!matured.is_zero(), Error::<T>::NothingToWithdraw);

            let released = T::Currency::release(
                &HoldReason::Staking.into(),
                &who,
                matured,
                Precision::BestEffort,
            )?;

            Self::deposit_event(Event::WithdrawnUnbonded {
                who: who.clone(),
                amount: released,
            });
            Ok(())
        }

        /// Opt a bonded staker into the validator candidate set.
        ///
        /// Requires bonded stake >= `MinStake`, session keys registered
        /// (`pallet_session::set_keys`), and — when `RequirePqcKey` is set — a
        /// registered PQC key from `PqcProvider`.
        #[pallet::call_index(4)]
        #[pallet::weight(<T as Config>::WeightInfo::validate(
            T::MaxValidatorCandidates::get().saturating_sub(1),
        ))]
        pub fn validate(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(
                Self::bonded(&who) >= T::MinStake::get(),
                Error::<T>::BondBelowMinimum
            );
            ensure!(
                T::SessionKeysLookup::keys_registered(&who),
                Error::<T>::KeysNotRegistered
            );
            if T::RequirePqcKey::get() {
                ensure!(
                    T::PqcProvider::has_pqc_key(&who),
                    Error::<T>::PqcKeyRequired
                );
            }
            ensure!(
                !Candidates::<T>::get().contains(&who),
                Error::<T>::AlreadyCandidate
            );
            Candidates::<T>::try_mutate(|c| {
                c.try_push(who.clone())
                    .map_err(|_| Error::<T>::TooManyCandidates)
            })?;
            Self::deposit_event(Event::CandidateJoined { who });
            Ok(())
        }

        /// Leave the validator candidate set (stays bonded).
        #[pallet::call_index(5)]
        #[pallet::weight(<T as Config>::WeightInfo::chill(
            T::MaxValidatorCandidates::get().saturating_sub(1),
            T::MaxValidators::get().saturating_sub(1),
        ))]
        pub fn chill(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(
                Candidates::<T>::get().contains(&who),
                Error::<T>::NotCandidate
            );
            Self::remove_candidate(&who);
            Self::deposit_event(Event::Chilled { who });
            Ok(())
        }
    }

    /// Helper functions.
    impl<T: Config> Pallet<T> {
        /// The pallet's own account (reward reserve sink).
        pub fn account_id() -> T::AccountId {
            use sp_runtime::traits::AccountIdConversion;
            T::PalletId::get().into_account_truncating()
        }

        /// Bonded stake of `who` (zero when not bonded).
        pub fn bonded(who: &T::AccountId) -> BalanceOf<T> {
            Bonded::<T>::get(who)
        }

        /// The difficulty work factor the next block must satisfy.
        ///
        /// Called by the runtime's `GhostPowApi::next_difficulty` impl; used by
        /// `sc-consensus-pow` on every import.
        pub fn next_difficulty() -> U256 {
            Difficulty::<T>::get()
        }

        /// Remove `who` from the candidate set.
        ///
        /// Deliberately does NOT touch `ActiveValidators`: that mirrors the
        /// committee actually seated by `pallet_session`, which keeps the
        /// account voting until the next session boundary regardless. Chilling
        /// takes effect through `select_validators` at the next `new_session`;
        /// mutating the active set mid-session would desync the pallet's
        /// record from the live GRANDPA committee (and would defeat the
        /// empty-committee fallback in `new_session`).
        fn remove_candidate(who: &T::AccountId) {
            Candidates::<T>::mutate(|c| {
                c.retain(|v| v != who);
            });
        }

        /// Shared bond/bond_extra implementation.
        fn do_bond(who: &T::AccountId, amount: BalanceOf<T>) -> DispatchResult {
            let total = Self::bonded(who).saturating_add(amount);
            ensure!(total <= T::MaxStake::get(), Error::<T>::BondAboveMaximum);
            T::Currency::hold(&HoldReason::Staking.into(), who, amount)?;
            Bonded::<T>::insert(who, total);
            Self::deposit_event(Event::Bonded {
                who: who.clone(),
                added: amount,
                total,
            });
            Ok(())
        }

        /// Validator selection for session planning: top `MaxValidators`
        /// candidates by bonded stake, tie-broken by account id (ascending) —
        /// fully deterministic.
        fn select_validators() -> Vec<T::AccountId> {
            let mut scored: Vec<(T::AccountId, BalanceOf<T>)> = Candidates::<T>::get()
                .into_iter()
                .map(|who| (who.clone(), Self::bonded(&who)))
                .filter(|(_, bonded)| *bonded >= T::MinStake::get())
                .filter(|(who, _)| {
                    // Re-checked at selection: `revoke_pqc_key` must not let a
                    // seated validator keep their seat after shedding the key.
                    !T::RequirePqcKey::get() || T::PqcProvider::has_pqc_key(who)
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            scored.truncate(T::MaxValidators::get() as usize);
            scored.into_iter().map(|(who, _)| who).collect()
        }

        /// Decode the block author from this block's `PreRuntime(POW_ENGINE_ID,
        /// SCALE(AccountId32))` digest item. Returns `None` when absent or
        /// malformed — never panics.
        fn block_author() -> Option<T::AccountId> {
            frame_system::Pallet::<T>::digest()
                .logs
                .iter()
                .find_map(|item| match item {
                    DigestItem::PreRuntime(id, data) if *id == POW_ENGINE_ID => {
                        T::AccountId::decode(&mut &data[..]).ok()
                    }
                    _ => None,
                })
        }

        /// Mint and split the block reward: 40% author / 60% pro-rata by
        /// bonded stake over `ActiveValidators` (pallet account when empty or
        /// all-zero). Rounding remainder goes to the author.
        fn distribute_block_reward(author: &T::AccountId) {
            let total = T::BlockReward::get();
            if total.is_zero() {
                return;
            }

            RecentAuthors::<T>::mutate(|recent| {
                if recent.is_full() {
                    recent.remove(0);
                }
                let _ = recent.try_push(author.clone());
            });

            let author_reward = Perbill::from_percent(40).mul_floor(total);
            let validators_pot = total.saturating_sub(author_reward);

            let stakers: Vec<(T::AccountId, BalanceOf<T>)> = ActiveValidators::<T>::get()
                .into_iter()
                .map(|who| (who.clone(), Self::bonded(&who)))
                .filter(|(_, bonded)| !bonded.is_zero())
                .collect();
            let total_stake: BalanceOf<T> = stakers
                .iter()
                .fold(Zero::zero(), |acc, (_, s)| acc.saturating_add(*s));

            let mut validators_paid = BalanceOf::<T>::zero();
            if total_stake.is_zero() {
                // No (staked) validators: the validator share goes to the
                // pallet account as a reserve.
                let _ = T::Currency::mint_into(&Self::account_id(), validators_pot);
                validators_paid = validators_pot;
            } else {
                for (who, stake) in stakers {
                    // U256 intermediates: pot * stake fits u128^2, no overflow.
                    let share: BalanceOf<T> = (U256::from(validators_pot.saturated_into::<u128>())
                        .saturating_mul(U256::from(stake.saturated_into::<u128>()))
                        / U256::from(total_stake.saturated_into::<u128>()))
                    .saturated_into::<u128>()
                    .saturated_into();
                    if !share.is_zero() {
                        let _ = T::Currency::mint_into(&who, share);
                        validators_paid = validators_paid.saturating_add(share);
                    }
                }
            }

            // Rounding remainder goes to the author (no dust accumulation).
            let remainder = total
                .saturating_sub(author_reward)
                .saturating_sub(validators_paid);
            let author_total = author_reward.saturating_add(remainder);
            let _ = T::Currency::mint_into(author, author_total);

            Self::deposit_event(Event::BlockRewarded {
                author: author.clone(),
                author_reward: author_total,
                validators_reward: validators_paid,
            });
        }

        /// Retarget the difficulty work factor.
        ///
        /// `new = old * clamp(expected_ms / elapsed_ms, 1/4, 4)`:
        /// blocks arriving too fast (elapsed < expected) increase the work
        /// factor; too slow decreases it. `elapsed == 0` is treated as a
        /// max-up retarget. Floored at `MinDifficulty`, saturating at
        /// `U256::MAX`. The math itself is shared with the node in
        /// `ghost_pow_primitives::compute_next_difficulty` — do not reimplement
        /// it here or the on-chain and off-chain copies can drift.
        fn retarget() {
            let now: u64 = pallet_timestamp::Pallet::<T>::get().saturated_into();
            let expected_ms =
                u64::from(T::RetargetInterval::get()).saturating_mul(T::TargetBlockTimeMs::get());
            let old = Difficulty::<T>::get();

            // First boundary only establishes the baseline timestamp.
            if RetargetsDone::<T>::get().is_zero() {
                RetargetsDone::<T>::put(1u32);
                LastRetargetTime::<T>::put(now);
                return;
            }

            let elapsed = now.saturating_sub(LastRetargetTime::<T>::get());
            let new = compute_next_difficulty(old, elapsed, expected_ms, T::MinDifficulty::get());
            if new != old {
                Difficulty::<T>::put(new);
                Self::deposit_event(Event::DifficultyRetargeted { old, new });
            }
            RetargetsDone::<T>::mutate(|done| *done = done.saturating_add(1));
            LastRetargetTime::<T>::put(now);
        }
    }

    /// Hooks for automatic behavior.
    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// Retarget the difficulty work factor every `RetargetInterval` blocks.
        fn on_initialize(n: BlockNumberFor<T>) -> Weight {
            let mut weight = T::DbWeight::get().reads(2);
            if !n.is_zero() && (n % T::RetargetInterval::get().into()).is_zero() {
                Self::retarget();
                weight += T::DbWeight::get().reads_writes(3, 3);
            }
            weight
        }

        /// Attribute and split the block reward from the PoW author digest.
        fn on_finalize(n: BlockNumberFor<T>) {
            match Self::block_author() {
                Some(author) => Self::distribute_block_reward(&author),
                None => Self::deposit_event(Event::BlockRewardSkipped { block_number: n }),
            }
        }
    }

    /// `pallet_session::SessionManager`: select the validator set per session.
    ///
    /// Returns `Some(set)` only when the computed set differs from the active
    /// one, so GRANDPA authority set changes are not issued gratuitously.
    impl<T: Config> pallet_session::SessionManager<T::AccountId> for Pallet<T> {
        fn new_session(_new_index: SessionIndex) -> Option<Vec<T::AccountId>> {
            let mut set = Self::select_validators();
            if set.is_empty() {
                // An empty committee would stall GRANDPA finality forever
                // (review M-4): keep the active committee instead. If there
                // isn't one either, fall back to the genesis-declared set.
                set = ActiveValidators::<T>::get().into_inner();
                if set.is_empty() {
                    set = InitialValidators::<T>::get().into_inner();
                }
            }
            let pending = BoundedVec::<T::AccountId, T::MaxValidators>::truncate_from(set.clone());
            PendingValidators::<T>::put(pending.clone());
            if set == ActiveValidators::<T>::get().into_inner() {
                None
            } else {
                Some(set)
            }
        }

        fn new_session_genesis(_new_index: SessionIndex) -> Option<Vec<T::AccountId>> {
            let mut set = Self::select_validators();
            if set.is_empty() {
                // No stakes yet: fall back to the genesis-declared validators
                // so a dev chain can still start.
                set = InitialValidators::<T>::get().into_inner();
            }
            let pending = BoundedVec::<T::AccountId, T::MaxValidators>::truncate_from(set.clone());
            PendingValidators::<T>::put(pending);
            Some(set)
        }

        fn start_session(_start_index: SessionIndex) {
            ActiveValidators::<T>::put(PendingValidators::<T>::get());
        }

        fn end_session(_end_index: SessionIndex) {
            // Downtime detection is handled by pallet-im-online offences; no
            // per-session bookkeeping needed here.
        }
    }

    /// `pallet_session::historical::SessionManager`: same selection, carrying
    /// each validator's bonded stake as the full identification so offences
    /// (GRANDPA equivocation, im-online unresponsiveness) can be attributed.
    impl<T: Config> pallet_session::historical::SessionManager<T::AccountId, BalanceOf<T>>
        for Pallet<T>
    {
        fn new_session(new_index: SessionIndex) -> Option<Vec<(T::AccountId, BalanceOf<T>)>> {
            <Self as pallet_session::SessionManager<T::AccountId>>::new_session(new_index).map(
                |set| {
                    set.into_iter()
                        .map(|who| {
                            let bonded = Self::bonded(&who);
                            (who, bonded)
                        })
                        .collect()
                },
            )
        }

        fn new_session_genesis(
            new_index: SessionIndex,
        ) -> Option<Vec<(T::AccountId, BalanceOf<T>)>> {
            <Self as pallet_session::SessionManager<T::AccountId>>::new_session_genesis(new_index)
                .map(|set| {
                    set.into_iter()
                        .map(|who| {
                            let bonded = Self::bonded(&who);
                            (who, bonded)
                        })
                        .collect()
                })
        }

        fn start_session(start_index: SessionIndex) {
            <Self as pallet_session::SessionManager<T::AccountId>>::start_session(start_index)
        }

        fn end_session(end_index: SessionIndex) {
            <Self as pallet_session::SessionManager<T::AccountId>>::end_session(end_index)
        }
    }

    /// `OnOffenceHandler`: slash `slash_fraction` of each offender's bonded
    /// stake (burned), chill them, and append a bounded `SlashRecord`.
    ///
    /// Wired in the runtime as `pallet_offences::Config::OnOffenceHandler`;
    /// offences arrive from GRANDPA equivocation reports and im-online
    /// unresponsiveness reports. `Offender` is
    /// `pallet_session::historical::IdentificationTuple` = `(AccountId, stake)`.
    impl<T: Config> OnOffenceHandler<T::AccountId, (T::AccountId, BalanceOf<T>), Weight> for Pallet<T> {
        fn on_offence(
            offenders: &[OffenceDetails<T::AccountId, (T::AccountId, BalanceOf<T>)>],
            slash_fraction: &[Perbill],
            slash_session: SessionIndex,
        ) -> Weight {
            let mut consumed = Weight::zero();
            let mut add_db_reads_writes = |reads, writes| {
                consumed += T::DbWeight::get().reads_writes(reads, writes);
            };

            for (details, fraction) in offenders.iter().zip(slash_fraction.iter()) {
                let who = &details.offender.0;
                let bonded = Bonded::<T>::get(who);
                add_db_reads_writes(2, 1);

                // Slash everything still held under the staking reason: the
                // bonded balance AND every unbonding chunk. Chunk funds stay
                // held until `withdraw_unbonded`, so they remain slashable —
                // otherwise `unbond` would grant immunity to a reported
                // offence (see docs/security-review/round-1.md).
                let mut unbonding_cut = BalanceOf::<T>::zero();
                Unbonding::<T>::mutate(who, |chunks| {
                    for chunk in chunks.iter_mut() {
                        let cut = fraction.mul_floor(chunk.amount);
                        unbonding_cut = unbonding_cut.saturating_add(cut);
                        chunk.amount = chunk.amount.saturating_sub(cut);
                    }
                    chunks.retain(|chunk| !chunk.amount.is_zero());
                });
                let slash_bonded = fraction.mul_floor(bonded);
                let amount = slash_bonded.saturating_add(unbonding_cut);

                // Genuinely stakeless offenders (no bonded, no unbonding
                // chunks) are skipped entirely — there is nothing to burn or
                // record. Offenders who merely finished `unbond` still fall
                // through to the chill and record below.
                if bonded.is_zero() && amount.is_zero() {
                    continue;
                }

                if !amount.is_zero() {
                    let _ = T::Currency::burn_held(
                        &HoldReason::Staking.into(),
                        who,
                        amount,
                        Precision::BestEffort,
                        Fortitude::Force,
                    );
                    let remaining = bonded.saturating_sub(slash_bonded);
                    if remaining.is_zero() {
                        Bonded::<T>::remove(who);
                    } else {
                        Bonded::<T>::insert(who, remaining);
                    }
                }

                // Slash and chill: equivocating/unresponsive validators leave
                // the candidate and active sets.
                Self::remove_candidate(who);
                SlashRecords::<T>::mutate(|records| {
                    if records.is_full() {
                        records.remove(0);
                    }
                    let _ = records.try_push(SlashRecord {
                        who: who.clone(),
                        amount,
                        session_index: slash_session,
                        block_number: frame_system::Pallet::<T>::block_number(),
                    });
                });

                Self::deposit_event(Event::Slashed {
                    who: who.clone(),
                    amount,
                    session_index: slash_session,
                });
                add_db_reads_writes(2, 6);
            }
            consumed
        }
    }
}

/// Weight info for the pallet's extrinsics.
///
/// Parameters are the benchmarked linear components of `weights.rs`; the
/// `#[pallet::weight]` annotations pass each component's worst-case value.
pub trait WeightInfo {
    /// Weight of `bond`.
    fn bond() -> Weight;
    /// Weight of `bond_extra`.
    fn bond_extra() -> Weight;
    /// Weight of `unbond`; `u` = unbonding chunks already queued.
    fn unbond(u: u32) -> Weight;
    /// Weight of `withdraw_unbonded`; `u` = unbonding chunks drained.
    fn withdraw_unbonded(u: u32) -> Weight;
    /// Weight of `validate`; `c` = candidates already opted in.
    fn validate(c: u32) -> Weight;
    /// Weight of `chill`; `c` = other candidates retained over,
    /// `v` = other active validators retained over.
    fn chill(c: u32, v: u32) -> Weight;
}

/// Test-only `WeightInfo` used by unit-test mocks.
impl WeightInfo for () {
    fn bond() -> Weight {
        Weight::from_parts(10_000, 0)
    }
    fn bond_extra() -> Weight {
        Weight::from_parts(10_000, 0)
    }
    fn unbond(_u: u32) -> Weight {
        Weight::from_parts(10_000, 0)
    }
    fn withdraw_unbonded(_u: u32) -> Weight {
        Weight::from_parts(10_000, 0)
    }
    fn validate(_c: u32) -> Weight {
        Weight::from_parts(10_000, 0)
    }
    fn chill(_c: u32, _v: u32) -> Weight {
        Weight::from_parts(10_000, 0)
    }
}
