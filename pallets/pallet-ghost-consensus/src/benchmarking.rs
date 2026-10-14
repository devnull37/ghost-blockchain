//! Benchmarks for `pallet-ghost-consensus` extrinsics.
//!
//! Worst-case setup per extrinsic:
//! * `bond` / `bond_extra`: caller funded to `MaxStake + ED`; `bond` measures
//!   the first-time path (`bonded == 0` branch), `bond_extra` the existing-bond
//!   path topping up to `MaxStake`.
//! * `unbond(u)`: `u` unbonding chunks already queued, so the extrinsic's
//!   `try_push` reaches `MaxUnbondingChunks` at the top of the range; the
//!   `Unbonding` map get/set decodes+re-encodes the queue — O(u).
//! * `withdraw_unbonded(u)`: `u` matured chunks queued (the whole bounded vec
//!   is retained over, then dropped) — O(u).
//! * `validate(c)`: `c` other candidates in `Candidates`; the extrinsic's
//!   `contains` + `try_push` scan/decode the whole vec — O(c). Session keys +
//!   (when enabled) a PQC key are arranged by `T::BenchmarkHelper`.
//! * `chill(c)`: caller last in `Candidates` (c others); `remove_candidate`
//!   retains over it — O(c). `ActiveValidators` is untouched by design (the
//!   seat drops at the next session boundary).

use super::*;
use frame_benchmarking::v2::*;
use frame_system::RawOrigin;

const SEED: u32 = 0;

/// A fresh funded account: `MaxStake + ED` free balance so any bond amount
/// succeeds.
fn funded<T: Config>(index: u32) -> T::AccountId {
    let who = account::<T::AccountId>("benchmark", index, SEED);
    let amount = T::MaxStake::get().saturating_add(T::Currency::minimum_balance());
    T::Currency::mint_into(&who, amount).expect("funded account mint works");
    who
}

/// Establish `Bonded` state exactly as `do_bond` leaves it (hold + map entry),
/// without running the extrinsic — setup only.
fn bond_in_storage<T: Config>(who: &T::AccountId, amount: BalanceOf<T>) {
    T::Currency::hold(&HoldReason::Staking.into(), who, amount).expect("benchmark hold succeeds");
    Bonded::<T>::insert(who, amount);
}

/// `count` distinct accounts, disjoint from benchmark callers.
fn candidate_accounts<T: Config>(count: u32) -> Vec<T::AccountId> {
    (0..count)
        .map(|i| account::<T::AccountId>("candidate", i, SEED))
        .collect()
}

#[benchmarks]
mod benchmarks {
    use super::*;

    /// First-time bond at `MaxStake`: `Bonded` miss + `Currency::hold` +
    /// `Bonded` insert + event.
    #[benchmark]
    fn bond() -> Result<(), BenchmarkError> {
        let who = funded::<T>(0);
        let amount = T::MaxStake::get();

        #[extrinsic_call]
        _(RawOrigin::Signed(who.clone()), amount);

        assert_eq!(Bonded::<T>::get(&who), amount);
        Ok(())
    }

    /// Top up an existing `MinStake` bond to `MaxStake`.
    #[benchmark]
    fn bond_extra() -> Result<(), BenchmarkError> {
        let who = funded::<T>(0);
        bond_in_storage::<T>(&who, T::MinStake::get());
        let amount = T::MaxStake::get().saturating_sub(T::MinStake::get());

        #[extrinsic_call]
        _(RawOrigin::Signed(who.clone()), amount);

        assert_eq!(Bonded::<T>::get(&who), T::MaxStake::get());
        Ok(())
    }

    /// Queue one more unbonding chunk on top of `u` existing chunks; the
    /// `Unbonding` map decode/encode is O(u).
    #[benchmark]
    fn unbond(
        u: Linear<0, { T::MaxUnbondingChunks::get().saturating_sub(1) }>,
    ) -> Result<(), BenchmarkError> {
        let who = funded::<T>(0);
        let bonded = T::MaxStake::get();
        bond_in_storage::<T>(&who, bonded);

        let chunk = UnbondingChunk {
            amount: T::MinStake::get(),
            unlock_at: frame_system::Pallet::<T>::block_number(),
        };
        Unbonding::<T>::mutate(&who, |chunks| {
            for _ in 0..u {
                chunks.try_push(chunk.clone()).expect("queue has capacity");
            }
        });
        assert_eq!(Unbonding::<T>::get(&who).len() as u32, u);

        #[extrinsic_call]
        _(RawOrigin::Signed(who.clone()), T::MinStake::get());

        assert_eq!(Unbonding::<T>::get(&who).len() as u32, u + 1);
        assert_eq!(
            Bonded::<T>::get(&who),
            bonded.saturating_sub(T::MinStake::get())
        );
        Ok(())
    }

    /// Drain `u` matured chunks: bounded-vec `retain` is O(u) and each chunk's
    /// `unlock_at` is compared — worst case is a full queue all matured.
    #[benchmark]
    fn withdraw_unbonded(
        u: Linear<1, { T::MaxUnbondingChunks::get() }>,
    ) -> Result<(), BenchmarkError> {
        let who = funded::<T>(0);
        let per_chunk = T::MinStake::get();
        let total: BalanceOf<T> = per_chunk.saturating_mul(u.saturated_into());
        T::Currency::hold(&HoldReason::Staking.into(), &who, total)
            .expect("benchmark hold succeeds");

        Unbonding::<T>::mutate(&who, |chunks| {
            for _ in 0..u {
                chunks
                    .try_push(UnbondingChunk {
                        amount: per_chunk,
                        // block 0 matured before block 1.
                        unlock_at: Zero::zero(),
                    })
                    .expect("queue has capacity");
            }
        });

        #[extrinsic_call]
        _(RawOrigin::Signed(who.clone()));

        assert!(Unbonding::<T>::get(&who).is_empty());
        Ok(())
    }

    /// Opt into the candidate set with `c` candidates already present; the
    /// `contains` + `try_push` over `Candidates` is O(c). Session keys and the
    /// (optional) PQC key are provisioned by `T::BenchmarkHelper` so the worst
    /// case runs every gate.
    #[benchmark]
    fn validate(
        c: Linear<0, { T::MaxValidatorCandidates::get().saturating_sub(1) }>,
    ) -> Result<(), BenchmarkError> {
        let who = funded::<T>(0);
        bond_in_storage::<T>(&who, T::MinStake::get());
        T::BenchmarkHelper::prepare_validate(&who);

        let mut others = BoundedVec::<T::AccountId, T::MaxValidatorCandidates>::default();
        for a in candidate_accounts::<T>(c) {
            others.try_push(a).expect("candidates bounded below caller");
        }
        Candidates::<T>::put(others);

        #[extrinsic_call]
        _(RawOrigin::Signed(who.clone()));

        assert!(Candidates::<T>::get().contains(&who));
        Ok(())
    }

    /// `chill` with `c` other candidates; `remove_candidate` retains over
    /// `Candidates` — caller placed last so the retain scans the full vec.
    /// `ActiveValidators` is deliberately NOT mutated (a chilled validator
    /// keeps its seat until the next session boundary), so no `v` param.
    #[benchmark]
    fn chill(
        c: Linear<0, { T::MaxValidatorCandidates::get().saturating_sub(1) }>,
    ) -> Result<(), BenchmarkError> {
        let who: T::AccountId = account::<T::AccountId>("chilled", 0, SEED);

        let mut others = BoundedVec::<T::AccountId, T::MaxValidatorCandidates>::default();
        for a in candidate_accounts::<T>(c) {
            others.try_push(a).expect("candidates bounded below caller");
        }
        others.try_push(who.clone()).expect("caller fits at max");
        Candidates::<T>::put(others);

        #[extrinsic_call]
        _(RawOrigin::Signed(who.clone()));

        assert!(!Candidates::<T>::get().contains(&who));
        Ok(())
    }

    impl_benchmark_test_suite!(Pallet, crate::mock::new_test_ext(), crate::mock::Test);
}
