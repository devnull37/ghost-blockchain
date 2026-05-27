//! Real post-quantum signature verification (ML-DSA / NIST FIPS 204).
//!
//! "Dilithium-5" is standardized as **ML-DSA-87** (FIPS 204, security category 5).
//! This module exposes deterministic, panic-free, `no_std`-compatible verification
//! that runs directly inside the Substrate Wasm runtime.
//!
//! Key generation and signing require randomness and therefore live only behind
//! `#[cfg(test)]` here (and in off-chain node tooling). The runtime itself only ever
//! *verifies* — so the production code path pulls no RNG/`getrandom` dependency.
//!
//! This module is the **only** place that touches the `fips204` crate API directly;
//! the rest of the pallet calls [`verify_ml_dsa`] / [`validate_ml_dsa_pk`], which keeps
//! the external API surface isolated behind a stable wrapper.

use crate::types::PqAlgorithm;
use fips204::traits::{SerDes, Verifier};

/// Public-key byte length for ML-DSA-44 (Dilithium-2, NIST level 2).
pub const ML_DSA_44_PK_LEN: usize = fips204::ml_dsa_44::PK_LEN;
/// Public-key byte length for ML-DSA-65 (Dilithium-3, NIST level 3).
pub const ML_DSA_65_PK_LEN: usize = fips204::ml_dsa_65::PK_LEN;
/// Public-key byte length for ML-DSA-87 (Dilithium-5, NIST level 5).
pub const ML_DSA_87_PK_LEN: usize = fips204::ml_dsa_87::PK_LEN;

/// Signature byte length for ML-DSA-44.
pub const ML_DSA_44_SIG_LEN: usize = fips204::ml_dsa_44::SIG_LEN;
/// Signature byte length for ML-DSA-65.
pub const ML_DSA_65_SIG_LEN: usize = fips204::ml_dsa_65::SIG_LEN;
/// Signature byte length for ML-DSA-87.
pub const ML_DSA_87_SIG_LEN: usize = fips204::ml_dsa_87::SIG_LEN;

/// Largest ML-DSA public key (ML-DSA-87). Bounds on-chain key storage.
pub const MAX_ML_DSA_PK_LEN: u32 = ML_DSA_87_PK_LEN as u32;
/// Largest ML-DSA signature (ML-DSA-87). Bounds on-chain signature inputs.
pub const MAX_ML_DSA_SIG_LEN: u32 = ML_DSA_87_SIG_LEN as u32;

/// `true` if `algorithm` is one of the ML-DSA parameter sets this module verifies.
pub fn is_ml_dsa(algorithm: &PqAlgorithm) -> bool {
    matches!(
        algorithm,
        PqAlgorithm::MlDsa44 | PqAlgorithm::MlDsa65 | PqAlgorithm::MlDsa87
    )
}

/// Expected public-key length for `algorithm`, or `None` if it is not ML-DSA.
pub fn ml_dsa_pk_len(algorithm: &PqAlgorithm) -> Option<usize> {
    match algorithm {
        PqAlgorithm::MlDsa44 => Some(ML_DSA_44_PK_LEN),
        PqAlgorithm::MlDsa65 => Some(ML_DSA_65_PK_LEN),
        PqAlgorithm::MlDsa87 => Some(ML_DSA_87_PK_LEN),
        _ => None,
    }
}

/// Expected signature length for `algorithm`, or `None` if it is not ML-DSA.
pub fn ml_dsa_sig_len(algorithm: &PqAlgorithm) -> Option<usize> {
    match algorithm {
        PqAlgorithm::MlDsa44 => Some(ML_DSA_44_SIG_LEN),
        PqAlgorithm::MlDsa65 => Some(ML_DSA_65_SIG_LEN),
        PqAlgorithm::MlDsa87 => Some(ML_DSA_87_SIG_LEN),
        _ => None,
    }
}

/// Verify an ML-DSA (FIPS 204) signature.
///
/// Deterministic, allocation-free, and never panics: any length mismatch, malformed
/// key/signature, oversized context, or non-ML-DSA algorithm returns `false`. Returns
/// `true` only for a cryptographically valid signature of `message` under `public_key`
/// for the given `ctx` (FIPS 204 context string, max 255 bytes).
pub fn verify_ml_dsa(
    algorithm: &PqAlgorithm,
    public_key: &[u8],
    message: &[u8],
    signature: &[u8],
    ctx: &[u8],
) -> bool {
    if ctx.len() > 255 {
        return false;
    }
    match algorithm {
        PqAlgorithm::MlDsa44 => verify_44(public_key, message, signature, ctx),
        PqAlgorithm::MlDsa65 => verify_65(public_key, message, signature, ctx),
        PqAlgorithm::MlDsa87 => verify_87(public_key, message, signature, ctx),
        _ => false,
    }
}

/// Structurally validate an ML-DSA public key: correct length for the algorithm and a
/// decodable FIPS 204 encoding. Returns `false` for non-ML-DSA algorithms.
pub fn validate_ml_dsa_pk(algorithm: &PqAlgorithm, public_key: &[u8]) -> bool {
    match algorithm {
        PqAlgorithm::MlDsa44 => decode_pk_44(public_key).is_some(),
        PqAlgorithm::MlDsa65 => decode_pk_65(public_key).is_some(),
        PqAlgorithm::MlDsa87 => decode_pk_87(public_key).is_some(),
        _ => false,
    }
}

// Generate a decoder + verifier pair for each parameter set. Keeping this in a macro
// guarantees the three sets stay byte-for-byte identical in logic.
macro_rules! impl_param_set {
    ($verify:ident, $decode_pk:ident, $m:ident, $pk_len:expr, $sig_len:expr) => {
        fn $decode_pk(pk: &[u8]) -> Option<fips204::$m::PublicKey> {
            let arr: [u8; $pk_len] = pk.try_into().ok()?;
            fips204::$m::PublicKey::try_from_bytes(arr).ok()
        }

        fn $verify(pk: &[u8], msg: &[u8], sig: &[u8], ctx: &[u8]) -> bool {
            let public = match $decode_pk(pk) {
                Some(p) => p,
                None => return false,
            };
            let sig_arr: [u8; $sig_len] = match sig.try_into() {
                Ok(a) => a,
                Err(_) => return false,
            };
            public.verify(msg, &sig_arr, ctx)
        }
    };
}

impl_param_set!(verify_44, decode_pk_44, ml_dsa_44, ML_DSA_44_PK_LEN, ML_DSA_44_SIG_LEN);
impl_param_set!(verify_65, decode_pk_65, ml_dsa_65, ML_DSA_65_PK_LEN, ML_DSA_65_SIG_LEN);
impl_param_set!(verify_87, decode_pk_87, ml_dsa_87, ML_DSA_87_PK_LEN, ML_DSA_87_SIG_LEN);

#[cfg(test)]
mod tests {
    use super::*;
    use fips204::traits::Signer;

    // Dilithium-5 (ML-DSA-87) is the chain default; exercise the full path.
    #[test]
    fn ml_dsa_87_dilithium5_roundtrip_and_tamper_detection() {
        let (pk, sk) = fips204::ml_dsa_87::try_keygen().expect("ml-dsa-87 keygen");
        let msg = b"ghost canonical block header bytes";
        let ctx = b"ghost-validator-v1";
        let sig = sk.try_sign(msg, ctx).expect("ml-dsa-87 sign");

        let pk_bytes = pk.into_bytes();
        assert_eq!(pk_bytes.len(), ML_DSA_87_PK_LEN);
        assert_eq!(sig.len(), ML_DSA_87_SIG_LEN);

        // Valid signature verifies.
        assert!(verify_ml_dsa(&PqAlgorithm::MlDsa87, &pk_bytes, msg, &sig, ctx));

        // Tampered message must fail.
        assert!(!verify_ml_dsa(&PqAlgorithm::MlDsa87, &pk_bytes, b"different message", &sig, ctx));

        // Wrong context must fail.
        assert!(!verify_ml_dsa(&PqAlgorithm::MlDsa87, &pk_bytes, msg, &sig, b"wrong-ctx"));

        // Bit-flipped signature must fail.
        let mut bad_sig = sig;
        bad_sig[100] ^= 0xFF;
        assert!(!verify_ml_dsa(&PqAlgorithm::MlDsa87, &pk_bytes, msg, &bad_sig, ctx));
    }

    #[test]
    fn ml_dsa_44_and_65_roundtrip() {
        let (pk, sk) = fips204::ml_dsa_44::try_keygen().unwrap();
        let sig = sk.try_sign(b"msg-44", b"").unwrap();
        assert!(verify_ml_dsa(&PqAlgorithm::MlDsa44, &pk.into_bytes(), b"msg-44", &sig, b""));

        let (pk, sk) = fips204::ml_dsa_65::try_keygen().unwrap();
        let sig = sk.try_sign(b"msg-65", b"").unwrap();
        assert!(verify_ml_dsa(&PqAlgorithm::MlDsa65, &pk.into_bytes(), b"msg-65", &sig, b""));
    }

    #[test]
    fn wrong_key_fails() {
        let (_pk1, sk1) = fips204::ml_dsa_87::try_keygen().unwrap();
        let (pk2, _sk2) = fips204::ml_dsa_87::try_keygen().unwrap();
        let sig = sk1.try_sign(b"m", b"").unwrap();
        // Signature from key 1 must not verify under key 2.
        assert!(!verify_ml_dsa(&PqAlgorithm::MlDsa87, &pk2.into_bytes(), b"m", &sig, b""));
    }

    #[test]
    fn rejects_bad_lengths_and_non_ml_dsa() {
        // Empty public key.
        assert!(!verify_ml_dsa(&PqAlgorithm::MlDsa87, &[], b"m", &[0u8; ML_DSA_87_SIG_LEN], b""));
        // Non-ML-DSA algorithm.
        assert!(!verify_ml_dsa(
            &PqAlgorithm::Unknown,
            &[0u8; ML_DSA_87_PK_LEN],
            b"m",
            &[0u8; ML_DSA_87_SIG_LEN],
            b"",
        ));
        // Oversized context.
        let (pk, _sk) = fips204::ml_dsa_87::try_keygen().unwrap();
        assert!(!verify_ml_dsa(
            &PqAlgorithm::MlDsa87,
            &pk.into_bytes(),
            b"m",
            &[0u8; ML_DSA_87_SIG_LEN],
            &[0u8; 256],
        ));
    }

    #[test]
    fn validate_pk_accepts_real_key_rejects_wrong_length() {
        let (pk, _sk) = fips204::ml_dsa_87::try_keygen().unwrap();
        let pk_bytes = pk.into_bytes();
        assert!(validate_ml_dsa_pk(&PqAlgorithm::MlDsa87, &pk_bytes));
        assert!(!validate_ml_dsa_pk(&PqAlgorithm::MlDsa87, &[0u8; 16]));
        assert!(!validate_ml_dsa_pk(&PqAlgorithm::Falcon512, &pk_bytes));
    }
}
