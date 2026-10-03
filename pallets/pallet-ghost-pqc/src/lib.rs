//! # Ghost PQC Key Registry Pallet
//!
//! On-chain registry of ML-DSA-87 (FIPS-204, formerly CRYSTALS-Dilithium5) public keys,
//! gated by proof-of-possession. See `docs/ghost-consensus-design.md` section 8.
//!
//! ## Overview
//!
//! - Accounts register their ML-DSA-87 public key by submitting a signature over the
//!   domain-separated message `b"GHOST-PQC-POP" || account_id` made with the matching
//!   secret key. One key per account; `revoke_pqc_key` releases the slot.
//! - Bonded validators (any registered key holder in v1 — see TODO on `pqc_attest`)
//!   can attest block hashes with their registered key via `pqc_attest`. Attestations
//!   are informational and do not gate finality.
//! - `PqcRequired` is a policy flag flipped by root; the consensus pallet reads it
//!   through [`PqcKeyProvider`] to decide whether validators must hold a PQC key.
//! - Signature verification runs inside the runtime and therefore MUST stay no_std.
//!   The verifier is the pure-Rust `ml-dsa` crate with `default-features = false`
//!   (no `alloc`, no `getrandom`). Do not add a std-gated verifier.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

pub mod weights;

use frame_support::weights::Weight;

/// Weight functions needed for `pallet-ghost-pqc`.
pub trait WeightInfo {
    fn register_pqc_key() -> Weight;
    fn revoke_pqc_key() -> Weight;
    fn pqc_attest() -> Weight;
    fn set_pqc_required() -> Weight;
}

use codec::Encode;
use frame_support::{pallet_prelude::ConstU32, BoundedVec};
use ml_dsa::{EncodedSignature, EncodedVerifyingKey, MlDsa87, Signature, Verifier, VerifyingKey};
use sp_runtime::sp_std::prelude::*;

/// Encoded ML-DSA-87 public key size in bytes (FIPS-204).
pub const PQC_PUBLIC_KEY_BYTES: usize = 2592;
/// Encoded ML-DSA-87 signature size in bytes (FIPS-204).
pub const PQC_SIGNATURE_BYTES: usize = 4627;

/// Maximum accepted public key size (`BoundedVec` bound).
pub type MaxPqcKeySize = ConstU32<{ PQC_PUBLIC_KEY_BYTES as u32 }>;
/// Maximum accepted signature size (`BoundedVec` bound).
pub type MaxSigSize = ConstU32<{ PQC_SIGNATURE_BYTES as u32 }>;

/// A registered ML-DSA-87 public key.
pub type PqcPublicKey = BoundedVec<u8, MaxPqcKeySize>;

/// Domain separator for proof-of-possession signatures, per design doc section 8.
/// The signed message is `POP_DOMAIN || account_id.encode()`.
pub const POP_DOMAIN: &[u8] = b"GHOST-PQC-POP";

/// Lookup surface consumed by `pallet-ghost-consensus` (and future pallets) to gate
/// logic on PQC key registration without taking a storage dependency on this pallet.
pub trait PqcKeyProvider<AccountId> {
    /// Returns `true` if `who` has a registered ML-DSA-87 key.
    fn has_pqc_key(who: &AccountId) -> bool;
    /// Returns the registered ML-DSA-87 public key of `who`, if any.
    fn pqc_key(who: &AccountId) -> Option<PqcPublicKey>;
}

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The overarching runtime event type.
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Weight information for extrinsics.
        type WeightInfo: WeightInfo;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// Registry of ML-DSA-87 public keys. One key per account.
    #[pallet::storage]
    #[pallet::getter(fn pqc_key)]
    pub type PqcKeys<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, PqcPublicKey>;

    /// Policy flag: when `true`, downstream consensus logic requires validators to
    /// hold a registered PQC key. Flipped by root only. Default `false`.
    ///
    /// Note: this flag gates *validator eligibility* in other pallets — it never
    /// gates signature verification itself, which is always on.
    #[pallet::storage]
    #[pallet::getter(fn pqc_required)]
    pub type PqcRequired<T: Config> = StorageValue<_, bool, ValueQuery>;

    /// Events that functions in this pallet can emit.
    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// An account registered an ML-DSA-87 public key.
        PqcKeyRegistered { who: T::AccountId },
        /// An account revoked its ML-DSA-87 public key.
        PqcKeyRevoked { who: T::AccountId },
        /// A registered key holder attested a block hash.
        PqcAttested {
            who: T::AccountId,
            block_hash: T::Hash,
        },
        /// The `PqcRequired` policy flag was updated.
        PqcRequiredSet { required: bool },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// The signature failed verification against the submitted/registered key.
        InvalidSignature,
        /// The account already has a registered key (revoke first to rotate).
        KeyAlreadyRegistered,
        /// The account has no registered key.
        KeyNotRegistered,
        /// The submitted public key exceeds `MaxPqcKeySize`.
        KeyTooLarge,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Register an ML-DSA-87 public key for the signed origin.
        ///
        /// `proof_of_possession` must be an ML-DSA-87 signature over
        /// `b"GHOST-PQC-POP" || account_id` produced by the secret key matching
        /// `public_key`. This prevents rogue-key registration / key-spam.
        ///
        /// TODO(deposit): take a storage deposit on registration and return it on
        /// `revoke_pqc_key` (requires `pallet_balances` holds wiring) once the
        /// economics spec lands — design doc does not specify parameters.
        #[pallet::call_index(0)]
        #[pallet::weight(<T as Config>::WeightInfo::register_pqc_key())]
        pub fn register_pqc_key(
            origin: OriginFor<T>,
            public_key: Vec<u8>,
            proof_of_possession: Vec<u8>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            ensure!(
                !PqcKeys::<T>::contains_key(&who),
                Error::<T>::KeyAlreadyRegistered
            );

            let key: PqcPublicKey =
                BoundedVec::try_from(public_key).map_err(|_| Error::<T>::KeyTooLarge)?;

            // PoP message per design doc section 8: b"GHOST-PQC-POP" || account_id.
            let mut message = Vec::with_capacity(POP_DOMAIN.len() + 32);
            message.extend_from_slice(POP_DOMAIN);
            who.encode_to(&mut message);

            ensure!(
                Self::verify_signature(&message, &proof_of_possession, &key),
                Error::<T>::InvalidSignature
            );

            PqcKeys::<T>::insert(&who, key);
            Self::deposit_event(Event::PqcKeyRegistered { who });
            Ok(())
        }

        /// Revoke the signed origin's registered key, freeing the slot for a
        /// fresh registration.
        ///
        /// TODO(deposit): release the registration deposit here once deposits are
        /// wired (see `register_pqc_key`).
        #[pallet::call_index(1)]
        #[pallet::weight(<T as Config>::WeightInfo::revoke_pqc_key())]
        pub fn revoke_pqc_key(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;

            ensure!(
                PqcKeys::<T>::contains_key(&who),
                Error::<T>::KeyNotRegistered
            );

            PqcKeys::<T>::remove(&who);
            Self::deposit_event(Event::PqcKeyRevoked { who });
            Ok(())
        }

        /// Attest a block hash with the signed origin's registered key.
        ///
        /// Per design doc section 8 the signer must be a bonded validator; the
        /// candidate/bond check lives in `pallet-ghost-consensus` and is not wired
        /// yet, so v1 gates on holding a registered key only.
        /// TODO(validator-gate): add a `BondedValidatorProvider` config bound and
        /// enforce bonded-validator status once the consensus pallet exposes it.
        /// Attestations are informational; they do not gate finality.
        #[pallet::call_index(2)]
        #[pallet::weight(<T as Config>::WeightInfo::pqc_attest())]
        pub fn pqc_attest(
            origin: OriginFor<T>,
            block_hash: T::Hash,
            signature: Vec<u8>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            let key = PqcKeys::<T>::get(&who).ok_or(Error::<T>::KeyNotRegistered)?;

            ensure!(
                Self::verify_signature(block_hash.as_ref(), &signature, &key),
                Error::<T>::InvalidSignature
            );

            Self::deposit_event(Event::PqcAttested { who, block_hash });
            Ok(())
        }

        /// Set the `PqcRequired` policy flag (root only). When `true`, consensus
        /// logic is expected to require validators to hold a registered PQC key.
        #[pallet::call_index(3)]
        #[pallet::weight(<T as Config>::WeightInfo::set_pqc_required())]
        pub fn set_pqc_required(origin: OriginFor<T>, required: bool) -> DispatchResult {
            ensure_root(origin)?;
            PqcRequired::<T>::put(required);
            Self::deposit_event(Event::PqcRequiredSet { required });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Verify an ML-DSA-87 signature. Pure no_std path — this is linked into
        /// the Wasm runtime, so no `std`/`alloc`/`getrandom` features may be used.
        ///
        /// `public_key` must decode to exactly `PQC_PUBLIC_KEY_BYTES` and
        /// `signature` to exactly `PQC_SIGNATURE_BYTES`; any deviation fails.
        fn verify_signature(message: &[u8], signature: &[u8], public_key: &PqcPublicKey) -> bool {
            let Some(vk_enc) = encoded_verifying_key(public_key) else {
                return false;
            };
            let Some(sig_enc) = encoded_signature(signature) else {
                return false;
            };
            let Some(sig) = Signature::<MlDsa87>::decode(&sig_enc) else {
                return false;
            };
            let vk = VerifyingKey::<MlDsa87>::decode(&vk_enc);
            vk.verify(message, &sig).is_ok()
        }
    }

    impl<T: Config> PqcKeyProvider<T::AccountId> for Pallet<T> {
        fn has_pqc_key(who: &T::AccountId) -> bool {
            PqcKeys::<T>::contains_key(who)
        }

        fn pqc_key(who: &T::AccountId) -> Option<PqcPublicKey> {
            PqcKeys::<T>::get(who)
        }
    }
}

/// Copy raw bytes into an `EncodedVerifyingKey` iff exactly `PQC_PUBLIC_KEY_BYTES` long.
fn encoded_verifying_key(bytes: &[u8]) -> Option<EncodedVerifyingKey<MlDsa87>> {
    if bytes.len() != PQC_PUBLIC_KEY_BYTES {
        return None;
    }
    let mut enc = EncodedVerifyingKey::<MlDsa87>::default();
    enc.copy_from_slice(bytes);
    Some(enc)
}

/// Copy raw bytes into an `EncodedSignature` iff exactly `PQC_SIGNATURE_BYTES` long.
fn encoded_signature(bytes: &[u8]) -> Option<EncodedSignature<MlDsa87>> {
    if bytes.len() != PQC_SIGNATURE_BYTES {
        return None;
    }
    let mut enc = EncodedSignature::<MlDsa87>::default();
    enc.copy_from_slice(bytes);
    Some(enc)
}
