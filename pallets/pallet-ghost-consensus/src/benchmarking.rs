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
//! * `chill(c, v)`: caller last in `Candidates` (c others) and
//!   `ActiveValidators` (v others); `remove_candidate` retains over both —
//!   O(c) + O(v).

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

    /// `chill` with `c` other candidates + `v` other active validators;
    /// `remove_candidate` retains over both lists — caller placed last so the
    /// retain scans the full vecs.
    #[benchmark]
    fn chill(
        c: Linear<0, { T::MaxValidatorCandidates::get().saturating_sub(1) }>,
        v: Linear<0, { T::MaxValidators::get().saturating_sub(1) }>,
    ) -> Result<(), BenchmarkError> {
        let who: T::AccountId = account::<T::AccountId>("chilled", 0, SEED);

        let mut others = BoundedVec::<T::AccountId, T::MaxValidatorCandidates>::default();
        for a in candidate_accounts::<T>(c) {
            others.try_push(a).expect("candidates bounded below caller");
        }
        others.try_push(who.clone()).expect("caller fits at max");
        Candidates::<T>::put(others);

        let mut active = BoundedVec::<T::AccountId, T::MaxValidators>::default();
        for i in 0..v {
            active
                .try_push(account::<T::AccountId>("active", i, SEED))
                .expect("validators bounded below caller");
        }
        active.try_push(who.clone()).expect("caller fits at max");
        ActiveValidators::<T>::put(active);

        #[extrinsic_call]
        _(RawOrigin::Signed(who.clone()));

        assert!(!Candidates::<T>::get().contains(&who));
        assert!(!ActiveValidators::<T>::get().contains(&who));
        Ok(())
    }

    impl_benchmark_test_suite!(Pallet, crate::mock::new_test_ext(), crate::mock::Test);
}

/// Deterministic native measurement harness.
///
/// `run_benchmark` (the code `frame-benchmarking-cli` invokes through a Wasm
/// executor) relies on backend-side benchmarking hooks —
/// `read_write_count`, `proof_size`, `commit` — that the stock
/// `InMemoryBackend` leaves `unimplemented!()`. [`TrackingBackend`] supplies
/// them, so the *real* generated `run_benchmark` executes natively:
///
/// * `extrinsic_time` — wall-clock nanoseconds of the dispatch (native `std`
///   execution, not Wasm).
/// * `reads`/`writes`/`keys` — Externalities-level storage-key accesses
///   counted by the backend wrapper, honouring the benchmark whitelist.
/// * `proof_size` — cumulative bytes of every touched key+value; `diff_pov`
///   yields the data bytes the timed call touched (a measured-data lower
///   bound; real trie proofs add hash-node overhead on top).
///
/// Weight formulas use `Analysis::min_squares_iqr` — the CLI default — so
/// `weights.rs` matches the CLI's emitted shape end-to-end.
///
/// Regenerate:
/// ```sh
/// GENERATE_WEIGHTS=1 rtk cargo test -p pallet-ghost-consensus \
///     --features runtime-benchmarks --release -- --nocapture generate_weights
/// ```
#[cfg(all(test, feature = "runtime-benchmarks"))]
mod measure {
    use crate::mock::{new_test_ext, Test};
    use crate::Pallet as GhostConsensus;
    use core::cell::{Cell, RefCell};
    use frame_benchmarking::{
        Analysis, BenchmarkMetadata, BenchmarkParameter, BenchmarkResult, BenchmarkSelector,
        Benchmarking,
    };
    use sp_core::storage::{ChildInfo, StateVersion, TrackedStorageKey};
    use sp_state_machine::{
        Backend, BackendTransaction, ChildStorageCollection, DefaultError, Ext, InMemoryBackend,
        IterArgs, OverlayedChanges, StateMachineStats, StorageCollection, StorageIterator,
        StorageKey, StorageValue, UsageInfo,
    };
    use sp_trie::{MerkleValue, PrefixedMemoryDB};
    use std::collections::BTreeMap;

    /// Samples per component value (matches the CLI's `--steps` semantics).
    const STEPS: u32 = 50;
    /// External repeats per component value. Each runs on a fresh backend —
    /// the in-memory `wipe` cannot un-commit, so repeats must not share state.
    const EXTERNAL_REPEAT: u32 = 5;

    /// Per-key access record backing `get_read_and_written_keys`.
    #[derive(Debug, Default)]
    struct KeyRecord {
        reads: u32,
        writes: u32,
    }

    /// `sp_io::TestExternalities`' hasher (`Blake2Hasher`).
    type H = sp_core::Blake2Hasher;

    /// `Backend` wrapper around [`InMemoryBackend`] that implements the
    /// benchmarking counters (`read_write_count`, `proof_size`,
    /// `get_read_and_written_keys`, `commit`) whose `Backend` defaults are
    /// `unimplemented!()` — everything else delegates.
    ///
    /// Counting model, matching `sc-client-db`'s `BenchmarkState`
    /// (`substrate/client/db/src/bench.rs`):
    /// * one tracker per touched key holding independent read/write counts;
    /// * `reads` = keys with `reads > 0`, `repeat_reads` = `reads - 1` per key
    ///   summed — so a key that is read and then written at `commit` counts as
    ///   one read AND one write;
    /// * `proof_size` accumulates `key + value` bytes of every newly recorded
    ///   non-whitelisted key (a data-bytes lower bound of the proof; see file
    ///   docs).
    #[derive(Debug, Default)]
    struct TrackingBackend {
        inner: RefCell<InMemoryBackend<H>>,
        whitelist: RefCell<Vec<TrackedStorageKey>>,
        keys: RefCell<BTreeMap<StorageKey, KeyRecord>>,
        pov_bytes: Cell<u64>,
    }

    impl TrackingBackend {
        fn is_whitelisted(&self, key: &[u8]) -> bool {
            self.whitelist
                .borrow()
                .iter()
                .any(|k| k.whitelisted && k.key.as_slice() == key)
        }

        fn record(&self, key: &[u8], value: &Option<StorageValue>, write: bool) {
            if self.is_whitelisted(key) {
                return;
            }
            let mut keys = self.keys.borrow_mut();
            let rec = keys.entry(key.to_vec()).or_insert_with(|| {
                let size = key.len() + value.as_ref().map_or(0, |v| v.len());
                self.pov_bytes.set(self.pov_bytes.get() + size as u64);
                KeyRecord::default()
            });
            if write {
                rec.writes += 1;
            } else {
                rec.reads += 1;
            }
        }
    }

    impl Backend<H> for TrackingBackend {
        type Error = DefaultError;
        type TrieBackendStorage = PrefixedMemoryDB<H>;
        type RawIter = NeverIter;

        fn storage(&self, key: &[u8]) -> Result<Option<StorageValue>, Self::Error> {
            let value = self.inner.borrow().storage(key)?;
            self.record(key, &value, false);
            Ok(value)
        }

        fn storage_hash(&self, key: &[u8]) -> Result<Option<sp_core::H256>, Self::Error> {
            let hash = self.inner.borrow().storage_hash(key)?;
            self.record(key, &None, false);
            Ok(hash)
        }

        fn closest_merkle_value(
            &self,
            key: &[u8],
        ) -> Result<Option<MerkleValue<sp_core::H256>>, Self::Error> {
            let value = self.inner.borrow().closest_merkle_value(key)?;
            self.record(key, &None, false);
            Ok(value)
        }

        fn child_closest_merkle_value(
            &self,
            child_info: &ChildInfo,
            key: &[u8],
        ) -> Result<Option<MerkleValue<sp_core::H256>>, Self::Error> {
            let value = self
                .inner
                .borrow()
                .child_closest_merkle_value(child_info, key)?;
            let mut full_key = child_info.prefixed_storage_key().to_vec();
            full_key.extend_from_slice(key);
            self.record(&full_key, &None, false);
            Ok(value)
        }

        fn child_storage(
            &self,
            child_info: &ChildInfo,
            key: &[u8],
        ) -> Result<Option<StorageValue>, Self::Error> {
            let value = self.inner.borrow().child_storage(child_info, key)?;
            let mut full_key = child_info.prefixed_storage_key().to_vec();
            full_key.extend_from_slice(key);
            self.record(&full_key, &value, false);
            Ok(value)
        }

        fn child_storage_hash(
            &self,
            child_info: &ChildInfo,
            key: &[u8],
        ) -> Result<Option<sp_core::H256>, Self::Error> {
            let hash = self.inner.borrow().child_storage_hash(child_info, key)?;
            let mut full_key = child_info.prefixed_storage_key().to_vec();
            full_key.extend_from_slice(key);
            self.record(&full_key, &None, false);
            Ok(hash)
        }

        fn next_storage_key(&self, key: &[u8]) -> Result<Option<StorageKey>, Self::Error> {
            let next = self.inner.borrow().next_storage_key(key)?;
            if let Some(next) = next.as_ref() {
                self.record(next, &None, false);
            }
            Ok(next)
        }

        fn next_child_storage_key(
            &self,
            child_info: &ChildInfo,
            key: &[u8],
        ) -> Result<Option<StorageKey>, Self::Error> {
            let next = self
                .inner
                .borrow()
                .next_child_storage_key(child_info, key)?;
            if let Some(next) = next.as_ref() {
                let mut full_key = child_info.prefixed_storage_key().to_vec();
                full_key.extend_from_slice(next);
                self.record(&full_key, &None, false);
            }
            Ok(next)
        }

        fn storage_root<'a>(
            &self,
            delta: impl Iterator<Item = (&'a [u8], Option<&'a [u8]>)>,
            state_version: StateVersion,
        ) -> (sp_core::H256, BackendTransaction<H>)
        where
            sp_core::H256: Ord + codec::Codec,
        {
            self.inner.borrow().storage_root(delta, state_version)
        }

        fn child_storage_root<'a>(
            &self,
            child_info: &ChildInfo,
            delta: impl Iterator<Item = (&'a [u8], Option<&'a [u8]>)>,
            state_version: StateVersion,
        ) -> (sp_core::H256, bool, BackendTransaction<H>)
        where
            sp_core::H256: Ord + codec::Codec,
        {
            self.inner
                .borrow()
                .child_storage_root(child_info, delta, state_version)
        }

        fn raw_iter(&self, _args: IterArgs) -> Result<Self::RawIter, Self::Error> {
            // No Ghost-consensus path iterates storage; if one appears, wire
            // the iterator honestly rather than faking it.
            Err("raw_iter not supported by TrackingBackend".into())
        }

        fn register_overlay_stats(&self, _stats: &StateMachineStats) {}

        fn usage_info(&self) -> UsageInfo {
            UsageInfo::empty()
        }

        fn wipe(&self) -> Result<(), Self::Error> {
            self.inner.borrow().wipe()
        }

        fn commit(
            &self,
            root: sp_core::H256,
            tx: BackendTransaction<H>,
            main: StorageCollection,
            child: ChildStorageCollection,
        ) -> Result<(), Self::Error> {
            for (key, value) in main.iter() {
                self.record(key, value, true);
            }
            for (child_key, changes) in child.iter() {
                for (key, value) in changes.iter() {
                    let mut full_key = child_key.clone();
                    full_key.extend_from_slice(key);
                    self.record(&full_key, value, true);
                }
            }
            self.inner.borrow_mut().apply_transaction(root, tx);
            Ok(())
        }

        /// `(reads, repeat_reads, writes, repeat_writes)` — one per key
        /// touched, repeats are accesses beyond the first per key (same as
        /// `BenchmarkState::read_write_count`).
        fn read_write_count(&self) -> (u32, u32, u32, u32) {
            let (mut reads, mut repeat_reads, mut writes, mut repeat_writes) =
                (0u32, 0u32, 0u32, 0u32);
            for rec in self.keys.borrow().values() {
                if rec.reads > 0 {
                    reads += 1;
                    repeat_reads += rec.reads - 1;
                }
                if rec.writes > 0 {
                    writes += 1;
                    repeat_writes += rec.writes - 1;
                }
            }
            (reads, repeat_reads, writes, repeat_writes)
        }

        fn reset_read_write_count(&self) {
            self.keys.borrow_mut().clear();
            self.pov_bytes.set(0);
        }

        fn get_whitelist(&self) -> Vec<TrackedStorageKey> {
            self.whitelist.borrow().clone()
        }

        fn set_whitelist(&self, new: Vec<TrackedStorageKey>) {
            *self.whitelist.borrow_mut() = new;
        }

        fn proof_size(&self) -> Option<u32> {
            Some(self.pov_bytes.get() as u32)
        }

        /// Per-key counts capped at 1 and aggregated by 32-byte key prefix,
        /// matching `BenchmarkState::get_read_and_written_keys`.
        fn get_read_and_written_keys(&self) -> Vec<(Vec<u8>, u32, u32, bool)> {
            let mut prefixes = BTreeMap::<Vec<u8>, (u32, u32)>::new();
            for (key, rec) in self.keys.borrow().iter() {
                let prefix_len = key.len().min(32);
                let entry = prefixes.entry(key[..prefix_len].to_vec()).or_default();
                entry.0 += rec.reads.min(1);
                entry.1 += rec.writes.min(1);
            }
            prefixes
                .into_iter()
                .map(|(key, (reads, writes))| (key, reads, writes, false))
                .collect()
        }
    }

    /// Unused `raw_iter` type — `Err` is returned before construction.
    #[derive(Debug)]
    struct NeverIter;
    impl StorageIterator<H> for NeverIter {
        type Backend = TrackingBackend;
        type Error = DefaultError;

        fn next_key(
            &mut self,
            _backend: &Self::Backend,
        ) -> Option<Result<StorageKey, Self::Error>> {
            unreachable!("raw_iter never constructed")
        }

        fn next_pair(
            &mut self,
            _backend: &Self::Backend,
        ) -> Option<Result<(StorageKey, StorageValue), Self::Error>> {
            unreachable!("raw_iter never constructed")
        }

        fn was_complete(&self) -> bool {
            true
        }
    }

    /// One `run_benchmark` invocation on a fresh tracking externalities;
    /// returns its single `BenchmarkResult` (internal_repeats = 1).
    fn run_once(
        extrinsic: &str,
        components: &[(BenchmarkParameter, u32)],
        verify: bool,
    ) -> BenchmarkResult {
        let mut test_ext = new_test_ext();
        let backend = TrackingBackend {
            inner: RefCell::new(test_ext.as_backend()),
            ..Default::default()
        };
        let mut overlay = OverlayedChanges::<H>::default();
        let mut ext = Ext::new(&mut overlay, &backend, None);
        let results = sp_externalities::set_and_run_with_externalities(&mut ext, || {
            <GhostConsensus<Test> as Benchmarking>::run_benchmark(
                extrinsic.as_bytes(),
                components,
                &[],
                verify,
                1,
            )
        })
        .expect("run_benchmark succeeds");
        assert_eq!(results.len(), 1);
        results.into_iter().next().expect("one result")
    }

    /// Expand the component grid like `benchmark pallet`: sweep each declared
    /// component `low..=high` at `STEPS` points with all other components
    /// pinned at `high`, then dedupe equal value sets.
    fn all_components(meta: &BenchmarkMetadata) -> Vec<Vec<(BenchmarkParameter, u32)>> {
        if meta.components.is_empty() {
            return vec![Default::default()];
        }
        let mut all = Vec::new();
        for (name, low, high) in meta.components.iter() {
            let step_size = ((*high - *low) as f32 / (STEPS - 1) as f32).max(0.0);
            for s in 0..STEPS {
                let v = ((*low as f32 + step_size * s as f32) as u32).clamp(*low, *high);
                let c: Vec<(BenchmarkParameter, u32)> = meta
                    .components
                    .iter()
                    .map(|(n, _, h)| (*n, if n == name { v } else { *h }))
                    .collect();
                all.push(c);
            }
        }
        // `BenchmarkParameter` is `PartialEq` only — dedupe by membership.
        let mut dedup: Vec<Vec<(BenchmarkParameter, u32)>> = Vec::new();
        for c in all {
            if !dedup.contains(&c) {
                dedup.push(c);
            }
        }
        dedup
    }

    /// `template.hbs`-style `_`-separated number rendering.
    fn fmt_num(n: u128) -> String {
        let s = n.to_string();
        let mut out = String::new();
        for (i, c) in s.chars().rev().enumerate() {
            if i % 3 == 0 && i > 0 {
                out.push('_');
            }
            out.push(c);
        }
        out.chars().rev().collect()
    }

    /// Emit the `impl` body for one extrinsic, mirroring the CLI template.
    fn emit_fn(
        name: &str,
        params: &[(BenchmarkParameter, u32, u32)],
        time: &Analysis,
        reads: &Analysis,
        writes: &Analysis,
        proof: Option<&Analysis>,
        min_time_ns: u128,
    ) -> String {
        let proof_base = proof.map_or(0, |a| a.base);
        let proof_slope = |i: usize| proof.map_or(0, |a| a.slopes.get(i).copied().unwrap_or(0));
        // A component is "used" when any analysis gives it a nonzero slope.
        let is_used: Vec<bool> = params
            .iter()
            .enumerate()
            .map(|(i, _)| {
                time.slopes.get(i).copied().unwrap_or(0) != 0
                    || reads.slopes.get(i).copied().unwrap_or(0) != 0
                    || writes.slopes.get(i).copied().unwrap_or(0) != 0
                    || proof_slope(i) != 0
            })
            .collect();
        let param_decls: Vec<String> = params
            .iter()
            .enumerate()
            .map(|(i, (p, _, _))| {
                let n = format!("{:?}", p);
                if is_used[i] {
                    format!("{}: u32", n)
                } else {
                    format!("_{}: u32", n)
                }
            })
            .collect();

        let mut f = String::new();
        for (p, lo, hi) in params {
            f.push_str(&format!(
                "    /// The range of component `{:?}` is `[{}, {}]`.\n",
                p, lo, hi
            ));
        }
        f.push_str(&format!(
            "    fn {}({}) -> Weight {{\n",
            name,
            param_decls.join(", ")
        ));
        f.push_str("        // Proof Size summary in bytes:\n");
        f.push_str(&format!(
            "        //  Measured:  `{}`\n        //  Estimated: `{}`\n",
            proof_base, proof_base
        ));
        f.push_str(&format!(
            "        // Minimum execution time: {}_000 picoseconds.\n",
            fmt_num(min_time_ns)
        ));
        f.push_str(&format!(
            "        Weight::from_parts({}, 0)\n            .saturating_add(Weight::from_parts(0, {}))\n",
            fmt_num(time.base),
            proof_base
        ));
        for (i, (p, _, _)) in params.iter().enumerate() {
            let slope = time.slopes.get(i).copied().unwrap_or(0);
            if slope == 0 {
                continue;
            }
            f.push_str(&format!(
                "            .saturating_add(Weight::from_parts({}, 0).saturating_mul(({:?}).into()))\n",
                fmt_num(slope),
                p
            ));
        }
        if reads.base != 0 {
            f.push_str(&format!(
                "            .saturating_add(T::DbWeight::get().reads({}_u64))\n",
                reads.base
            ));
        }
        for (i, (p, _, _)) in params.iter().enumerate() {
            let slope = reads.slopes.get(i).copied().unwrap_or(0);
            if slope == 0 {
                continue;
            }
            f.push_str(&format!(
                "            .saturating_add(T::DbWeight::get().reads(({}_u64).saturating_mul(({:?}).into())))\n",
                slope, p
            ));
        }
        if writes.base != 0 {
            f.push_str(&format!(
                "            .saturating_add(T::DbWeight::get().writes({}_u64))\n",
                writes.base
            ));
        }
        for (i, (p, _, _)) in params.iter().enumerate() {
            let slope = writes.slopes.get(i).copied().unwrap_or(0);
            if slope == 0 {
                continue;
            }
            f.push_str(&format!(
                "            .saturating_add(T::DbWeight::get().writes(({}_u64).saturating_mul(({:?}).into())))\n",
                slope, p
            ));
        }
        for (i, (p, _, _)) in params.iter().enumerate() {
            let slope = proof_slope(i);
            if slope == 0 {
                continue;
            }
            f.push_str(&format!(
                "            .saturating_add(Weight::from_parts(0, {}).saturating_mul(({:?}).into()))\n",
                slope, p
            ));
        }
        f.push_str("    }\n");
        f
    }

    /// Measure every declared benchmark, print a summary table, and — with
    /// `GENERATE_WEIGHTS=1` — write `src/weights.rs`.
    #[test]
    fn generate_weights() {
        let metas = <GhostConsensus<Test> as Benchmarking>::benchmarks(false);
        let mut rendered = Vec::new();
        let mut table = String::from(
            "| extrinsic | base ref_time (ps) | reads | writes | pov(B) |\n|---|---:|---:|---:|---:|\n",
        );

        for meta in metas.iter() {
            let name = String::from_utf8(meta.name.clone()).expect("benchmark names are utf-8");
            let mut results = Vec::new();
            for c in all_components(meta) {
                // One verification pass (result discarded) per component set,
                // then timed passes.
                let _ = run_once(&name, &c, true);
                for _ in 0..EXTERNAL_REPEAT {
                    results.push(run_once(&name, &c, false));
                }
            }
            let time = Analysis::min_squares_iqr(&results, BenchmarkSelector::ExtrinsicTime)
                .expect("extrinsic time analysis");
            let reads = Analysis::min_squares_iqr(&results, BenchmarkSelector::Reads)
                .expect("reads analysis");
            let writes = Analysis::min_squares_iqr(&results, BenchmarkSelector::Writes)
                .expect("writes analysis");
            let proof = Analysis::min_squares_iqr(&results, BenchmarkSelector::ProofSize);
            let min_time_ns = time.minimum;
            let proof_base = proof.as_ref().map_or(0, |a| a.base);

            rendered.push(emit_fn(
                &name,
                &meta.components,
                &time,
                &reads,
                &writes,
                proof.as_ref(),
                min_time_ns,
            ));
            table.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                name,
                fmt_num(time.base),
                reads.base,
                writes.base,
                proof_base
            ));
        }

        println!("\n=== measured weights ===\n{}", table);

        if std::env::var("GENERATE_WEIGHTS").is_err() {
            println!("(set GENERATE_WEIGHTS=1 to write src/weights.rs)");
            return;
        }

        let header = concat!(
            "//! Autogenerated weights for `pallet_ghost_consensus`\n",
            "//!\n",
            "//! Generated NATIVELY by the in-pallet measurement harness\n",
            "//! (`mod measure` in `src/benchmarking.rs`) executing the real\n",
            "//! generated `run_benchmark` over an instrumented in-memory state\n",
            "//! backend — the `frame-benchmarking-cli` node subcommand is not\n",
            "//! wired on this branch yet (see PR description for the follow-up).\n",
            "//!\n",
            "//! Command:\n",
            "//!   GENERATE_WEIGHTS=1 rtk cargo test -p pallet-ghost-consensus \\\n",
            "//!     --features runtime-benchmarks --release -- --nocapture generate_weights\n",
            "//!\n",
            "//! Caveats vs a `frame-benchmarking-cli` run:\n",
            "//!   * `ref_time` is native `std` execution on the generating host\n",
            "//!     (see PR body for CPU/host); Wasm execution timing differs —\n",
            "//!     regenerate via the CLI before any public-network claim.\n",
            "//!   * DB read/write counts are Externalities-level key accesses.\n",
            "//!   * `proof_size` = bytes of touched key+value data (measured\n",
            "//!     lower bound; ignores trie-node hash overhead).\n",
            "\n",
        );
        let mut file = String::from(header);
        file.push_str(concat!(
            "\n#![cfg_attr(rustfmt, rustfmt_skip)]\n",
            "#![allow(unused_parens)]\n",
            "#![allow(unused_imports)]\n",
            "#![allow(missing_docs)]\n",
            "\n",
            "use frame_support::{traits::Get, weights::Weight};\n",
            "use core::marker::PhantomData;\n",
            "\n",
            "/// Weight functions for `pallet_ghost_consensus`.\n",
            "pub struct SubstrateWeight<T>(PhantomData<T>);\n",
            "impl<T: frame_system::Config> crate::WeightInfo for SubstrateWeight<T> {\n",
        ));
        for f in rendered {
            file.push_str(&f);
            file.push('\n');
        }
        file.push_str("}\n");
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/weights.rs");
        std::fs::write(path, file).expect("write weights.rs");
        println!("wrote {}", path);
    }
}
