//! Benchmarks for `pallet-ghost-pqc`.
//!
//! Benchmarks compile only with `runtime-benchmarks` — nothing in the runtime
//! tree links `ml_dsa::SigningKey`/`sign_deterministic` in production.
//!
//! Worst-case states: all four extrinsics have fixed-size inputs (the ML-DSA-87
//! key/signature encodings are exact-size or rejected), so no linear component
//! applies — `register_pqc_key` and `pqc_attest` each run one full ML-DSA-87
//! signature verification, which dominates ref_time by orders of magnitude.
//! `pqc_attest`'s bonded-validator check is seeded by `T::BenchmarkHelper`.
//!
//! Weights come from the node's real benchmark pipeline (Wasm-measured):
//! ```sh
//! rtk cargo build --release --bin ghost-node --features runtime-benchmarks
//! ./target/release/ghost-node benchmark pallet --chain dev \
//!     --pallet pallet_ghost_pqc --extrinsic "*" --steps 50 --repeat 20
//! ```

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
        _(
            RawOrigin::Signed(who.clone()),
            public_key,
            proof_of_possession,
        );

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
        // `pqc_attest` is gated to bonded validators in the real runtime —
        // the helper seeds whatever `T::BondedValidators` reads (a no-op in
        // the mock, where everyone except NOT_BONDED is bonded).
        T::BenchmarkHelper::seed_bonded_validator(&who);

        let block_hash =
            <T::Hash as codec::Decode>::decode(&mut &sp_core::H256::repeat_byte(42)[..])
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
