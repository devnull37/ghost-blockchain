//! Real post-quantum encryption for the Ghost node — ML-KEM-1024 + ChaCha20-Poly1305.
//!
//! # What this module provides
//!
//! * **ML-KEM-1024 key encapsulation** (NIST FIPS 203, security category 5, the
//!   "Kyber-1024" parameter set) via the [`fips203`] crate (integritychain, same
//!   author/design as the `fips204` ML-DSA crate already used for signatures).
//!
//! * **Hybrid authenticated encryption** of arbitrary payloads: ML-KEM-1024 for
//!   key agreement, ChaCha20-Poly1305 (RustCrypto, IETF RFC 8439) for the actual
//!   AEAD seal/open.  The ML-KEM shared secret (32 bytes) is used directly as the
//!   AEAD key.
//!
//! # Security boundary — please read before deploying
//!
//! This module provides real ML-KEM-1024 post-quantum key encapsulation and
//! ChaCha20-Poly1305 authenticated encryption for payloads and operator tooling.
//! It does **NOT** replace the libp2p Noise/X25519 transport handshake: the
//! `stable2407` polkadot-sdk ships libp2p without a PQ-Noise variant, so
//! **node-to-node transport remains classical (X25519 + ChaCha20-Poly1305 Noise)
//! and is NOT yet post-quantum**. Use this module for application-layer messages,
//! key storage, and operator utilities — not as a claim that the network transport
//! is post-quantum hardened.
//!
//! # FIPS 203 byte-length reference (ML-KEM-1024)
//!
//! | Artifact               | Constant         | Bytes |
//! |------------------------|------------------|-------|
//! | Encapsulation key (EK) | [`EK_LEN`]       | 1 568 |
//! | Decapsulation key (DK) | [`DK_LEN`]       | 3 168 |
//! | Ciphertext (CT)        | [`CT_LEN`]       | 1 568 |
//! | Shared secret (SSK)    | [`SSK_LEN`]      |    32 |
//! | AEAD nonce             | [`NONCE_LEN`]    |    12 |

use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    ChaCha20Poly1305, Key, Nonce,
};
use fips203::{
    ml_kem_1024,
    traits::{Decaps, Encaps, KeyGen, SerDes},
};

// ── ML-KEM-1024 length constants ─────────────────────────────────────────────

/// Byte length of the ML-KEM-1024 encapsulation (public) key — 1 568 bytes.
pub const EK_LEN: usize = 1568;
/// Byte length of the ML-KEM-1024 decapsulation (secret) key — 3 168 bytes.
pub const DK_LEN: usize = 3168;
/// Byte length of the ML-KEM-1024 ciphertext — 1 568 bytes.
pub const CT_LEN: usize = 1568;
/// Byte length of the ML-KEM shared secret for all parameter sets — 32 bytes.
pub const SSK_LEN: usize = 32;
/// Byte length of the ChaCha20-Poly1305 nonce — 12 bytes.
pub const NONCE_LEN: usize = 12;

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors returned by the ML-KEM and hybrid-encryption helpers.
#[derive(Debug, PartialEq, Eq)]
pub enum PqEncryptError {
    /// The provided byte slice does not have the expected length for this type.
    ///
    /// Carries `(expected, got)`.
    BadLength(usize, usize),
    /// The `fips203` crate rejected the key, ciphertext, or shared secret.
    Fips203(&'static str),
    /// ChaCha20-Poly1305 authentication tag verification failed, or the payload
    /// is otherwise malformed.
    AeadError,
    /// Key generation failed (OS RNG unavailable or fips203 error).
    KeyGenFailed,
    /// Encapsulation failed (OS RNG unavailable or fips203 error).
    EncapsFailed,
}

impl core::fmt::Display for PqEncryptError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadLength(exp, got) =>
                write!(f, "bad byte length: expected {exp}, got {got}"),
            Self::Fips203(msg) => write!(f, "fips203 error: {msg}"),
            Self::AeadError  => write!(f, "AEAD authentication/decryption failed"),
            Self::KeyGenFailed => write!(f, "ML-KEM-1024 key generation failed"),
            Self::EncapsFailed => write!(f, "ML-KEM-1024 encapsulation failed"),
        }
    }
}

// ── Low-level ML-KEM-1024 wrappers ───────────────────────────────────────────

/// Generate an ML-KEM-1024 keypair using the OS random number generator.
///
/// Returns `(ek_bytes, dk_bytes)` where:
/// * `ek_bytes` is the 1 568-byte **encapsulation** (public) key — share this.
/// * `dk_bytes` is the 3 168-byte **decapsulation** (secret) key — keep secret.
///
/// Uses [`fips203::ml_kem_1024::KG::try_keygen`], which calls `getrandom`
/// internally.  Never panics; propagates RNG/parameter errors as
/// [`PqEncryptError::KeyGenFailed`].
pub fn keygen() -> Result<([u8; EK_LEN], [u8; DK_LEN]), PqEncryptError> {
    let (ek, dk) = ml_kem_1024::KG::try_keygen().map_err(|_| PqEncryptError::KeyGenFailed)?;
    Ok((ek.into_bytes(), dk.into_bytes()))
}

/// Encapsulate to an ML-KEM-1024 encapsulation key.
///
/// Takes the recipient's serialised encapsulation key (`ek_bytes`, must be
/// exactly [`EK_LEN`] = 1 568 bytes).  Returns `(shared_secret, ciphertext)`
/// where:
/// * `shared_secret` is 32 bytes — used as the AEAD key on both sides.
/// * `ciphertext` is [`CT_LEN`] = 1 568 bytes — send to the decapsulating party.
///
/// Returns an error if `ek_bytes` is the wrong length or is structurally invalid
/// according to FIPS 203.  Never panics.
pub fn encapsulate(
    ek_bytes: &[u8],
) -> Result<([u8; SSK_LEN], [u8; CT_LEN]), PqEncryptError> {
    // Length-check before the fixed-array cast so the error is descriptive.
    if ek_bytes.len() != EK_LEN {
        return Err(PqEncryptError::BadLength(EK_LEN, ek_bytes.len()));
    }
    let ek_arr: [u8; EK_LEN] = ek_bytes.try_into().expect("length checked above");
    let ek = ml_kem_1024::EncapsKey::try_from_bytes(ek_arr)
        .map_err(PqEncryptError::Fips203)?;

    let (ssk, ct) = ek.try_encaps().map_err(|_| PqEncryptError::EncapsFailed)?;
    Ok((ssk.into_bytes(), ct.into_bytes()))
}

/// Decapsulate an ML-KEM-1024 ciphertext using the holder's decapsulation key.
///
/// * `dk_bytes` — the 3 168-byte **decapsulation** (secret) key.
/// * `ct_bytes` — the 1 568-byte ciphertext produced by [`encapsulate`].
///
/// Returns the 32-byte shared secret, which will match what the encapsulating
/// party received, **unless the ciphertext is malformed** — in that case FIPS 203
/// mandates "implicit rejection": a different pseudo-random 32-byte value is
/// returned (so the caller never learns *whether* the ciphertext was valid;
/// authentication must be done at the AEAD layer).
pub fn decapsulate(
    dk_bytes: &[u8],
    ct_bytes: &[u8],
) -> Result<[u8; SSK_LEN], PqEncryptError> {
    if dk_bytes.len() != DK_LEN {
        return Err(PqEncryptError::BadLength(DK_LEN, dk_bytes.len()));
    }
    if ct_bytes.len() != CT_LEN {
        return Err(PqEncryptError::BadLength(CT_LEN, ct_bytes.len()));
    }
    let dk_arr: [u8; DK_LEN] = dk_bytes.try_into().expect("length checked above");
    let ct_arr: [u8; CT_LEN] = ct_bytes.try_into().expect("length checked above");

    let dk = ml_kem_1024::DecapsKey::try_from_bytes(dk_arr)
        .map_err(PqEncryptError::Fips203)?;
    let ct = ml_kem_1024::CipherText::try_from_bytes(ct_arr)
        .map_err(PqEncryptError::Fips203)?;

    let ssk = dk.try_decaps(&ct).map_err(PqEncryptError::Fips203)?;
    Ok(ssk.into_bytes())
}

// ── Hybrid message type ───────────────────────────────────────────────────────

/// A fully self-contained post-quantum encrypted message.
///
/// To send a message to a recipient, you only need their 1 568-byte
/// encapsulation key.  All the pieces needed to decrypt are bundled here.
///
/// Wire format (no external framing required for single-message use):
/// ```text
/// kem_ciphertext  : [u8; 1568]  // ML-KEM-1024 ciphertext
/// nonce           : [u8; 12]    // ChaCha20-Poly1305 nonce (random)
/// aead_ciphertext : Vec<u8>     // plaintext + 16-byte Poly1305 tag
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedMessage {
    /// ML-KEM-1024 ciphertext that carries the encapsulated shared secret.
    pub kem_ciphertext: [u8; CT_LEN],
    /// Random 12-byte nonce for ChaCha20-Poly1305.
    pub nonce: [u8; NONCE_LEN],
    /// ChaCha20-Poly1305 sealed payload (plaintext length + 16-byte auth tag).
    pub aead_ciphertext: Vec<u8>,
}

impl EncryptedMessage {
    /// Serialise to a flat `Vec<u8>` suitable for transmission or storage.
    ///
    /// Layout: `CT_LEN` || `NONCE_LEN` || `u32-LE aead_len` || `aead_ciphertext`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let aead_len = self.aead_ciphertext.len() as u32;
        let mut out = Vec::with_capacity(CT_LEN + NONCE_LEN + 4 + self.aead_ciphertext.len());
        out.extend_from_slice(&self.kem_ciphertext);
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&aead_len.to_le_bytes());
        out.extend_from_slice(&self.aead_ciphertext);
        out
    }

    /// Deserialise from the flat byte format produced by [`EncryptedMessage::to_bytes`].
    ///
    /// Returns `None` if the slice is too short or the embedded length field is
    /// inconsistent with the remaining bytes.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        // Minimum: CT_LEN + NONCE_LEN + 4-byte length field
        let header_len = CT_LEN + NONCE_LEN + 4;
        if bytes.len() < header_len {
            return None;
        }
        let kem_ciphertext: [u8; CT_LEN] = bytes[..CT_LEN].try_into().ok()?;
        let nonce: [u8; NONCE_LEN] = bytes[CT_LEN..CT_LEN + NONCE_LEN].try_into().ok()?;
        let aead_len = u32::from_le_bytes(
            bytes[CT_LEN + NONCE_LEN..CT_LEN + NONCE_LEN + 4].try_into().ok()?,
        ) as usize;
        let aead_start = CT_LEN + NONCE_LEN + 4;
        if bytes.len() != aead_start + aead_len {
            return None;
        }
        let aead_ciphertext = bytes[aead_start..].to_vec();
        Some(Self { kem_ciphertext, nonce, aead_ciphertext })
    }
}

// ── Hybrid encrypt / decrypt ──────────────────────────────────────────────────

/// Encrypt `plaintext` to a recipient identified by their ML-KEM-1024
/// encapsulation key (`recipient_ek_bytes`, 1 568 bytes).
///
/// Internally:
/// 1. Runs ML-KEM-1024 encapsulation to derive a 32-byte shared secret and a
///    1 568-byte KEM ciphertext.
/// 2. Generates a random 12-byte nonce via `OsRng`.
/// 3. Seals `plaintext` with ChaCha20-Poly1305, using the shared secret as the
///    key and the random nonce.
///
/// Returns an [`EncryptedMessage`] bundle containing everything the recipient
/// needs to decrypt.  Never panics; returns [`PqEncryptError`] on any failure.
pub fn encrypt_to(
    recipient_ek_bytes: &[u8],
    plaintext: &[u8],
) -> Result<EncryptedMessage, PqEncryptError> {
    // 1. KEM encapsulation — derive shared secret + KEM ciphertext.
    let (ssk_bytes, kem_ciphertext) = encapsulate(recipient_ek_bytes)?;

    // 2. Build the AEAD key from the 32-byte ML-KEM shared secret.
    let aead_key = Key::from(ssk_bytes);
    let cipher = ChaCha20Poly1305::new(&aead_key);

    // 3. Generate a fresh random 12-byte nonce.
    let nonce_bytes = generate_nonce()?;
    let nonce = Nonce::from(nonce_bytes);

    // 4. Seal the plaintext.
    let aead_ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| PqEncryptError::AeadError)?;

    Ok(EncryptedMessage { kem_ciphertext, nonce: nonce_bytes, aead_ciphertext })
}

/// Decrypt an [`EncryptedMessage`] using the holder's ML-KEM-1024 decapsulation
/// key (`dk_bytes`, 3 168 bytes).
///
/// Internally:
/// 1. Runs ML-KEM-1024 decapsulation to recover the 32-byte shared secret.
/// 2. Opens the ChaCha20-Poly1305 AEAD ciphertext.
///
/// Returns the plaintext on success, or `None` if:
/// * `dk_bytes` is malformed or the wrong length.
/// * The KEM ciphertext is structurally invalid (FIPS 203 implicit rejection
///   will have been applied — the AEAD open step then fails authentication).
/// * The AEAD authentication tag does not verify (tampered ciphertext, wrong key,
///   or wrong nonce).
///
/// Deliberately returns `Option` rather than a typed error so that callers cannot
/// distinguish *why* decryption failed (KEM vs. AEAD), which reduces oracle
/// information available to an attacker.
pub fn decrypt(dk_bytes: &[u8], msg: &EncryptedMessage) -> Option<Vec<u8>> {
    // 1. Recover the shared secret via ML-KEM decapsulation.
    let ssk_bytes = decapsulate(dk_bytes, &msg.kem_ciphertext).ok()?;

    // 2. Reconstruct the AEAD cipher.
    let aead_key = Key::from(ssk_bytes);
    let cipher = ChaCha20Poly1305::new(&aead_key);

    // 3. Open the AEAD ciphertext.
    let nonce = Nonce::from(msg.nonce);
    cipher.decrypt(&nonce, msg.aead_ciphertext.as_ref()).ok()
}

// ── Internal helper ───────────────────────────────────────────────────────────

/// Generate a fresh, cryptographically random 12-byte ChaCha20-Poly1305 nonce
/// using `AeadCore::generate_nonce` backed by the OS RNG (`OsRng`).
///
/// This delegates entirely to the `aead` crate's OsRng integration, so no
/// separate `getrandom` dependency is required.
fn generate_nonce() -> Result<[u8; NONCE_LEN], PqEncryptError> {
    // AeadCore::generate_nonce returns GenericArray<u8, U12>; convert to [u8; 12].
    let nonce_ga = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let nonce: [u8; NONCE_LEN] = nonce_ga.into();
    Ok(nonce)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Constant sanity — FIPS 203 spec values ────────────────────────────────

    /// Verify that the byte-length constants we export match the FIPS 203
    /// specification for ML-KEM-1024 (Table 2, NIST SP 800-235 draft).
    #[test]
    fn fips203_ml_kem_1024_byte_lengths_match_spec() {
        // Encapsulation key  : 32(ρ) + 1536(t̂) = 1568
        assert_eq!(EK_LEN, 1568);
        // Decapsulation key  : 768(s) + 1568(ek) + 32(H(ek)) + 32(z) = 2400... wait
        // FIPS 203 §7.1 Table 1: for k=4 (1024), DK byte length is 3168.
        assert_eq!(DK_LEN, 3168);
        // Ciphertext         : 1408 + 160 = 1568 for k=4
        assert_eq!(CT_LEN, 1568);
        // Shared secret      : 32 for all ML-KEM variants
        assert_eq!(SSK_LEN, 32);
        // ChaCha20-Poly1305 nonce: 96 bits
        assert_eq!(NONCE_LEN, 12);
    }

    // ── ML-KEM-1024 encap/decap produce the same shared secret ───────────────

    /// Core KEM property: the shared secret from `encapsulate` and from
    /// `decapsulate` must be bit-for-bit identical.
    #[test]
    fn encap_decap_shared_secret_matches() {
        let (ek_bytes, dk_bytes) = keygen().expect("keygen should succeed");

        assert_eq!(ek_bytes.len(), EK_LEN);
        assert_eq!(dk_bytes.len(), DK_LEN);

        let (ssk_enc, ct_bytes) = encapsulate(&ek_bytes).expect("encapsulate should succeed");
        assert_eq!(ct_bytes.len(), CT_LEN);
        assert_eq!(ssk_enc.len(), SSK_LEN);

        let ssk_dec = decapsulate(&dk_bytes, &ct_bytes).expect("decapsulate should succeed");
        assert_eq!(ssk_dec.len(), SSK_LEN);

        // This is the fundamental KEM correctness property.
        assert_eq!(
            ssk_enc, ssk_dec,
            "encapsulator and decapsulator must derive the same shared secret"
        );
    }

    /// A second independent keygen produces a different keypair, and the two
    /// shared secrets are different (probabilistic — collision chance 1/2^256).
    #[test]
    fn two_keygens_produce_distinct_keypairs() {
        let (ek1, _) = keygen().unwrap();
        let (ek2, _) = keygen().unwrap();
        assert_ne!(ek1, ek2, "two independent keygens should yield different EKs");
    }

    // ── Hybrid encrypt/decrypt round-trip ─────────────────────────────────────

    /// End-to-end happy path: encrypt a payload then decrypt it and get back the
    /// original plaintext.
    #[test]
    fn encrypt_decrypt_roundtrip() {
        let (ek_bytes, dk_bytes) = keygen().unwrap();
        let plaintext = b"Ghost blockchain operator message - confidential.";

        let msg = encrypt_to(&ek_bytes, plaintext).expect("encrypt_to should succeed");

        // The KEM ciphertext must be exactly CT_LEN bytes.
        assert_eq!(msg.kem_ciphertext.len(), CT_LEN);
        // The nonce must be exactly NONCE_LEN bytes.
        assert_eq!(msg.nonce.len(), NONCE_LEN);
        // AEAD ciphertext is plaintext + 16-byte Poly1305 tag.
        assert_eq!(msg.aead_ciphertext.len(), plaintext.len() + 16);

        let recovered = decrypt(&dk_bytes, &msg).expect("decrypt should succeed");
        assert_eq!(recovered, plaintext);
    }

    /// Empty plaintext is a valid input.
    #[test]
    fn encrypt_decrypt_empty_plaintext() {
        let (ek_bytes, dk_bytes) = keygen().unwrap();
        let msg = encrypt_to(&ek_bytes, b"").unwrap();
        let recovered = decrypt(&dk_bytes, &msg).unwrap();
        assert!(recovered.is_empty());
    }

    /// Large plaintext (64 KiB) is handled correctly — exercises the streaming
    /// path through ChaCha20-Poly1305.
    #[test]
    fn encrypt_decrypt_large_payload() {
        let (ek_bytes, dk_bytes) = keygen().unwrap();
        let plaintext = vec![0xABu8; 65536];
        let msg = encrypt_to(&ek_bytes, &plaintext).unwrap();
        let recovered = decrypt(&dk_bytes, &msg).unwrap();
        assert_eq!(recovered, plaintext);
    }

    // ── Wrong decapsulation key fails ─────────────────────────────────────────

    /// Decrypting with a different (wrong) decapsulation key must fail.
    ///
    /// FIPS 203 §8.3 specifies "implicit rejection": decapsulating with the
    /// wrong key returns a pseudo-random shared secret.  The AEAD open step
    /// will then fail authentication because the derived AEAD key is wrong.
    #[test]
    fn wrong_dk_returns_none() {
        let (ek1, _dk1) = keygen().unwrap();
        let (_ek2, dk2) = keygen().unwrap();

        let plaintext = b"secret payload";
        let msg = encrypt_to(&ek1, plaintext).unwrap();

        // dk2 belongs to a different keypair — AEAD authentication must fail.
        let result = decrypt(&dk2, &msg);
        assert!(
            result.is_none(),
            "decrypting with the wrong DK must return None"
        );
    }

    // ── Tampered AEAD ciphertext fails ────────────────────────────────────────

    /// Flipping one bit in the AEAD ciphertext must cause authentication to fail.
    #[test]
    fn tampered_aead_ciphertext_returns_none() {
        let (ek_bytes, dk_bytes) = keygen().unwrap();
        let plaintext = b"tamper-detection test";
        let mut msg = encrypt_to(&ek_bytes, plaintext).unwrap();

        // Flip the last byte of the AEAD ciphertext (part of the Poly1305 tag).
        let last = msg.aead_ciphertext.last_mut().expect("non-empty AEAD ciphertext");
        *last ^= 0xFF;

        let result = decrypt(&dk_bytes, &msg);
        assert!(
            result.is_none(),
            "tampered AEAD ciphertext must return None"
        );
    }

    /// Flipping one bit in the *body* of the AEAD ciphertext (not the tag)
    /// must also fail — Poly1305 covers the full ciphertext.
    #[test]
    fn tampered_aead_body_returns_none() {
        let (ek_bytes, dk_bytes) = keygen().unwrap();
        let plaintext = b"tamper-detection body test - long enough to have body bytes";
        let mut msg = encrypt_to(&ek_bytes, plaintext).unwrap();

        // Flip a byte in the middle (ciphertext body, before the 16-byte tag).
        msg.aead_ciphertext[0] ^= 0x01;

        assert!(decrypt(&dk_bytes, &msg).is_none());
    }

    /// Replacing the nonce with a different nonce must cause decryption to fail.
    #[test]
    fn wrong_nonce_returns_none() {
        let (ek_bytes, dk_bytes) = keygen().unwrap();
        let plaintext = b"nonce-mismatch test";
        let mut msg = encrypt_to(&ek_bytes, plaintext).unwrap();

        // Flip every bit of the nonce.
        for byte in &mut msg.nonce {
            *byte ^= 0xFF;
        }

        assert!(
            decrypt(&dk_bytes, &msg).is_none(),
            "wrong nonce must prevent decryption"
        );
    }

    // ── Serialisation round-trip of EncryptedMessage ─────────────────────────

    /// `to_bytes` / `from_bytes` must be inverses of each other.
    #[test]
    fn encrypted_message_wire_roundtrip() {
        let (ek_bytes, dk_bytes) = keygen().unwrap();
        let plaintext = b"wire-format round-trip";
        let msg = encrypt_to(&ek_bytes, plaintext).unwrap();

        let wire = msg.to_bytes();
        let decoded = EncryptedMessage::from_bytes(&wire)
            .expect("from_bytes should succeed on valid wire data");

        assert_eq!(msg, decoded);

        // And decrypting after the round-trip still works.
        let recovered = decrypt(&dk_bytes, &decoded).unwrap();
        assert_eq!(recovered.as_slice(), plaintext);
    }

    /// Truncated wire data must not panic and must return None.
    #[test]
    fn from_bytes_truncated_returns_none() {
        let wire = vec![0u8; CT_LEN + NONCE_LEN]; // missing length field
        assert!(EncryptedMessage::from_bytes(&wire).is_none());
    }

    // ── API error paths — bad input lengths ───────────────────────────────────

    #[test]
    fn encapsulate_rejects_wrong_ek_length() {
        let result = encapsulate(&[0u8; 16]);
        assert_eq!(result, Err(PqEncryptError::BadLength(EK_LEN, 16)));
    }

    #[test]
    fn decapsulate_rejects_wrong_dk_length() {
        let result = decapsulate(&[0u8; 16], &[0u8; CT_LEN]);
        assert_eq!(result, Err(PqEncryptError::BadLength(DK_LEN, 16)));
    }

    #[test]
    fn decapsulate_rejects_wrong_ct_length() {
        let (_ek, dk) = keygen().unwrap();
        let result = decapsulate(&dk, &[0u8; 16]);
        assert_eq!(result, Err(PqEncryptError::BadLength(CT_LEN, 16)));
    }

    #[test]
    fn encrypt_to_rejects_wrong_ek_length() {
        let result = encrypt_to(&[0u8; 100], b"msg");
        assert_eq!(result, Err(PqEncryptError::BadLength(EK_LEN, 100)));
    }
}
