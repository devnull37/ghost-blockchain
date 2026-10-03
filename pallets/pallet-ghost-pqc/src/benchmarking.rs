//! Benchmarks for `pallet-ghost-pqc`, plus a native measurement harness.
//!
//! Benchmarks compile only with `runtime-benchmarks` — nothing in the runtime
//! tree links `ml_dsa::SigningKey`/`sign_deterministic` in production.
//!
//! Worst-case states: all four extrinsics have fixed-size inputs (the ML-DSA-87
//! key/signature encodings are exact-size or rejected), so no linear component
//! applies — `register_pqc_key` and `pqc_attest` each run one full ML-DSA-87
//! signature verification, which dominates ref_time by orders of magnitude.
//!
//! # Measurement harness (`mod measure`)
//!
//! The node's `benchmark pallet` subcommand is not wired yet, so weights are
//! produced natively: [`run_once`] executes the real generated
//! `run_benchmark` (setups, `commit_db`, `wipe_db`, whitelist handling,
//! `verify` blocks, timing) inside `sp_io::externalities` backed by a
//! [`TrackingBackend`] — an `InMemoryBackend` wrapper implementing the
//! read/write counters and proof-size accounting that `InMemoryBackend` itself
//! leaves `unimplemented!()` (they exist only on the client-DB backends used by
//! the CLI). The component grid and `min_squares_iqr` analysis are identical to
//! `frame-benchmarking-cli`.
//!
//! Honest deviations from the CLI (documented in the generated `weights.rs`
//! header):
//! * times are *native* (debug or `--release`, depending on how the test was
//!   run) — on the Wasm interpreter the same extrinsics are slower; regenerate
//!   through the node `benchmark pallet` subcommand before any public-network
//!   weight claim;
//! * `reads`/`writes` count *unique storage keys* touched — the Externalities
//!   API never sees a key that stays in the overlay — this is exactly what
//!   `sc-client-db`'s `BenchmarkState` reports to `run_benchmark` too;
//! * `proof_size` is `key + value` bytes of touched state (data bytes) — a
//!   lower bound on the true trie proof, which also carries branch nodes.

use super::*;
use frame_benchmarking::v2::*;
use frame_system::RawOrigin;
use ml_dsa::{Keypair, MlDsa87, Seed, Signer, SigningKey};

/// A deterministic ML-DSA-87 keypair (fixed seed — keygen only shapes setup
/// state, never the measured dispatch).
fn keypair(seed: u8) -> SigningKey<MlDsa87> {
    let mut s = Seed::default();
    s.iter_mut().for_each(|b| *b = seed);
    SigningKey::from_seed(&s)
}

/// Encoded verifying key bytes for `sk`.
fn pk_bytes(sk: &SigningKey<MlDsa87>) -> Vec<u8> {
    sk.verifying_key().encode().as_slice().to_vec()
}

/// Deterministic ML-DSA-87 signature bytes over `msg` (empty context).
fn sig_bytes(sk: &SigningKey<MlDsa87>, msg: &[u8]) -> Vec<u8> {
    sk.sign(msg).encode().as_slice().to_vec()
}

/// Proof-of-possession signature over `POP_DOMAIN || who` — exactly what
/// `register_pqc_key` verifies.
fn pop_bytes<T: Config>(sk: &SigningKey<MlDsa87>, who: &T::AccountId) -> Vec<u8> {
    let mut msg = POP_DOMAIN.to_vec();
    who.encode_to(&mut msg);
    sig_bytes(sk, &msg)
}

#[benchmarks]
mod benchmarks {
    use super::*;

    /// Register a full-size (2592 B) ML-DSA-87 key with a real proof-of-
    /// possession signature — the extrinsic's only cost driver is the single
    /// signature verification, so the fixed input is already the worst case.
    #[benchmark]
    fn register_pqc_key() -> Result<(), BenchmarkError> {
        let who: T::AccountId = account("pqc", 0, 0);
        let sk = keypair(1);
        let public_key = pk_bytes(&sk);
        assert_eq!(public_key.len(), PQC_PUBLIC_KEY_BYTES);
        let proof_of_possession = pop_bytes::<T>(&sk, &who);
        assert_eq!(proof_of_possession.len(), PQC_SIGNATURE_BYTES);

        #[extrinsic_call]
        _(RawOrigin::Signed(who.clone()), public_key, proof_of_possession);

        assert!(PqcKeys::<T>::contains_key(&who));
        Ok(())
    }

    /// Revoke a registered key: `PqcKeys` read + remove.
    #[benchmark]
    fn revoke_pqc_key() -> Result<(), BenchmarkError> {
        let who: T::AccountId = account("pqc", 0, 0);
        let sk = keypair(1);
        let key: PqcPublicKey = pk_bytes(&sk)
            .try_into()
            .map_err(|_| BenchmarkError::Stop("key fits bound"))?;
        PqcKeys::<T>::insert(&who, key);

        #[extrinsic_call]
        _(RawOrigin::Signed(who.clone()));

        assert!(!PqcKeys::<T>::contains_key(&who));
        Ok(())
    }

    /// Attest a block hash with a real signature under the caller's registered
    /// key: `PqcKeys` read (2592 B value) + one ML-DSA-87 verify + event.
    #[benchmark]
    fn pqc_attest() -> Result<(), BenchmarkError> {
        let who: T::AccountId = account("pqc", 0, 0);
        let sk = keypair(1);
        let key: PqcPublicKey = pk_bytes(&sk)
            .try_into()
            .map_err(|_| BenchmarkError::Stop("key fits bound"))?;
        PqcKeys::<T>::insert(&who, key);

        let block_hash = <T::Hash as codec::Decode>::decode(
            &mut &sp_core::H256::repeat_byte(42)[..],
        )
        .expect("hash decodes");
        let signature = sig_bytes(&sk, block_hash.as_ref());
        assert_eq!(signature.len(), PQC_SIGNATURE_BYTES);

        #[extrinsic_call]
        _(RawOrigin::Signed(who.clone()), block_hash, signature);

        // Attestations are informational; the observable post-state is the
        // deposited event.
        assert_eq!(frame_system::Pallet::<T>::events().len(), 1);
        Ok(())
    }

    /// Root flips `PqcRequired`: one storage write.
    #[benchmark]
    fn set_pqc_required() -> Result<(), BenchmarkError> {
        let required = true;
        assert!(!PqcRequired::<T>::get());

        #[extrinsic_call]
        _(RawOrigin::Root, required);

        assert!(PqcRequired::<T>::get());
        Ok(())
    }

    impl_benchmark_test_suite!(Pallet, crate::mock::new_test_ext(), crate::mock::Test);
}

/// Native measurement harness — see module docs for method and caveats.
///
/// Regenerate `weights.rs` (release build — debug timings are not
/// representative):
/// ```sh
/// GENERATE_WEIGHTS=1 rtk cargo test -p pallet-ghost-pqc \
///     --features runtime-benchmarks --release -- --nocapture generate_weights
/// ```
#[cfg(all(test, feature = "runtime-benchmarks"))]
mod measure {
    use crate::mock::{new_test_ext, Test};
    use crate::Pallet as GhostPqc;
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
            let rec = keys
                .entry(key.to_vec())
                .or_insert_with(|| {
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

        fn storage_hash(
            &self,
            key: &[u8],
        ) -> Result<Option<sp_core::H256>, Self::Error> {
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
            // No Ghost-PQC path iterates storage; if one appears, wire the
            // iterator honestly rather than faking it.
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
            unreachable!("raw_iter never constructed")
        }
    }

    /// Run `run_benchmark` once for `name` under `c` with real state and
    /// tracking externalities; `verify` runs the benchmark's post-state
    /// assertions.
    fn run_once(name: &[u8], c: &[(BenchmarkParameter, u32)], verify: bool) -> BenchmarkResult {
        let mut test_ext = new_test_ext();
        let backend = TrackingBackend {
            inner: RefCell::new(test_ext.as_backend()),
            ..Default::default()
        };
        let mut overlay = OverlayedChanges::<H>::default();
        let mut ext = Ext::new(&mut overlay, &backend, None);
        let results = sp_externalities::set_and_run_with_externalities(&mut ext, || {
            <GhostPqc<Test> as Benchmarking>::run_benchmark(name, c, &[], verify, 1)
                .unwrap_or_else(|e| {
                    panic!("benchmark {:?} components {:?} failed: {:?}", name, c, e)
                })
        });
        results
            .into_iter()
            .next()
            .expect("internal_repeats=1 yields one result")
    }

    /// Replicates the CLI's component sweep: for each declared component take
    /// `STEPS` points in `low..=high` with all others pinned at `high`.
    fn all_components(meta: &BenchmarkMetadata) -> Vec<Vec<(BenchmarkParameter, u32)>> {
        let mut all: Vec<Vec<(BenchmarkParameter, u32)>> = Vec::new();
        for (i, (param, low, high)) in meta.components.iter().enumerate() {
            let steps = (STEPS.min((*high - *low).max(1)) + 1) as u64;
            for s in 0..steps {
                let v = low + (((*high - *low) as u64 * s) / (steps - 1).max(1)) as u32;
                let mut c: Vec<(BenchmarkParameter, u32)> = meta
                    .components
                    .iter()
                    .map(|(p, _, h)| (*p, *h))
                    .collect();
                c[i] = (*param, v);
                all.push(c);
            }
        }
        if all.is_empty() {
            all.push(Vec::new());
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

    fn fmt_num(n: u128) -> String {
        n.to_string()
            .as_bytes()
            .rchunks(3)
            .rev()
            .map(|c| core::str::from_utf8(c).unwrap().to_string())
            .collect::<Vec<_>>()
            .join("_")
    }

    /// Render one `weights.rs` function from `frame-benchmarking` `Analysis`
    /// results — same output shape as `frame-benchmarking-cli`'s `template.hbs`.
    /// `db` is the DB-weight accessor — `T::DbWeight::get()` inside
    /// `SubstrateWeight<T>` and `RocksDbWeight::get()` inside `()`.
    fn emit_fn(
        name: &str,
        params: &[(BenchmarkParameter, u32, u32)],
        time: &Analysis,
        reads: &Analysis,
        writes: &Analysis,
        proof: Option<&Analysis>,
        min_time_ns: u128,
        db: &str,
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
            .zip(is_used.iter())
            .map(|((p, _, _), used)| {
                format!("{}{}: u32", if *used { "" } else { "_" }, format!("{:?}", p))
            })
            .collect();

        let mut f = String::new();
        f.push_str(&format!("    fn {}({}) -> Weight {{\n", name, param_decls.join(", ")));
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
                "            // Standard Error: {}\n            .saturating_add(Weight::from_parts({}, 0).saturating_mul(({:?}).into()))\n",
                fmt_num(
                    time.errors
                        .as_ref()
                        .and_then(|e| e.get(i))
                        .copied()
                        .unwrap_or(0),
                ),
                fmt_num(slope),
                p
            ));
        }
        for (i, (p, _, _)) in params.iter().enumerate() {
            let slope = reads.slopes.get(i).copied().unwrap_or(0);
            if slope == 0 {
                continue;
            }
            f.push_str(&format!(
                "            .saturating_add({db}.reads({}_u64).saturating_mul(({:?}).into()))\n",
                slope, p, db = db
            ));
        }
        for (i, (p, _, _)) in params.iter().enumerate() {
            let slope = writes.slopes.get(i).copied().unwrap_or(0);
            if slope == 0 {
                continue;
            }
            f.push_str(&format!(
                "            .saturating_add({db}.writes({}_u64).saturating_mul(({:?}).into()))\n",
                slope, p, db = db
            ));
        }
        if reads.base != 0 {
            f.push_str(&format!(
                "            .saturating_add({db}.reads({}))\n",
                reads.base,
                db = db
            ));
        }
        if writes.base != 0 {
            f.push_str(&format!(
                "            .saturating_add({db}.writes({}))\n",
                writes.base,
                db = db
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

    #[test]
    fn generate_weights() {
        let metas = <GhostPqc<Test> as Benchmarking>::benchmarks(false);
        let mut rendered: Vec<(String, String)> = Vec::new();
        let mut table = String::from(
            "| extrinsic | base ref_time (ps) | reads | writes | pov(B) |\n|---|---:|---:|---:|---:|\n",
        );

        for meta in metas.iter() {
            let name = String::from_utf8(meta.name.clone()).expect("benchmark names are utf-8");
            let mut results = Vec::new();
            for c in all_components(meta) {
                // One verification pass (result discarded) per component set,
                // then timed passes.
                let _ = run_once(name.as_bytes(), &c, true);
                for _ in 0..EXTERNAL_REPEAT {
                    results.push(run_once(name.as_bytes(), &c, false));
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

            rendered.push((
                emit_fn(
                    &name,
                    &meta.components,
                    &time,
                    &reads,
                    &writes,
                    proof.as_ref(),
                    min_time_ns,
                    "T::DbWeight::get()",
                ),
                emit_fn(
                    &name,
                    &meta.components,
                    &time,
                    &reads,
                    &writes,
                    proof.as_ref(),
                    min_time_ns,
                    "RocksDbWeight::get()",
                ),
            ));
            table.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                name, fmt_num(time.base), reads.base, writes.base, proof_base
            ));
        }

        eprintln!("\n=== measured weights ===\n{}", table);

        if std::env::var("GENERATE_WEIGHTS").is_err() {
            return;
        }

        let mut out = String::from(HEADER);
        out.push_str(
            &rendered
                .iter()
                .map(|(t, _)| t.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        out.push_str("}\n\nimpl crate::WeightInfo for () {\n");
        out.push_str(
            &rendered
                .iter()
                .map(|(_, u)| u.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        out.push_str("}\n");
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/weights.rs"
        );
        std::fs::write(path, out).expect("write weights.rs");
        eprintln!("wrote {}", path);
    }

    const HEADER: &str = r#"//! Autogenerated weights for `pallet_ghost_pqc`
//!
//! THIS FILE WAS GENERATED BY AN IN-PALLET MEASUREMENT HARNESS — NOT by
//! `frame-benchmarking-cli` (the node `benchmark pallet` subcommand is not wired
//! yet; see `src/benchmarking.rs` → `mod measure`). Regenerate with:
//!
//! ```sh
//! GENERATE_WEIGHTS=1 rtk cargo test -p pallet-ghost-pqc \
//!     --features runtime-benchmarks --release -- --nocapture generate_weights
//! ```
//!
//! Caveats vs the canonical CLI flow — BEFORE any public-network weight claim,
//! re-run through the node `benchmark pallet` subcommand:
//!
//! * `ref_time` is native execution time (release build) — the Wasm interpreter
//!   is slower;
//! * `reads`/`writes` are unique storage keys touched, measured at the
//!   `Externalities` boundary — identical to what `sc-client-db`'s
//!   `BenchmarkState` reports;
//! * `proof_size` is a lower bound (data bytes of touched keys only — no trie
//!   branch nodes);
//! * `DbWeight` is read through `T::DbWeight` — the emitted file uses the
//!   runtime's configured DB weight at compile time of the consumer.

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]
#![allow(missing_docs)]

use frame_support::{traits::Get, weights::{constants::RocksDbWeight, Weight}};
use core::marker::PhantomData;

/// Weight functions for `pallet_ghost_pqc`.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> crate::WeightInfo for SubstrateWeight<T> {
"#;
}
