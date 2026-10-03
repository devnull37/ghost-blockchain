//! Tests for `pallet-ghost-pqc`. Uses real ML-DSA-87 keys and signatures —
//! `SigningKey::from_seed` gives deterministic keygen, so no RNG is needed.

use crate::{mock::*, pallet::*, PqcKeyProvider, POP_DOMAIN};
use codec::Encode;
use frame_support::{assert_noop, assert_ok};
use ml_dsa::{Keypair, MlDsa87, Seed, Signer, SigningKey};
use sp_core::{crypto::AccountId32, H256};
use sp_runtime::DispatchError::BadOrigin;

fn alice() -> AccountId32 {
    AccountId32::new([1u8; 32])
}

fn bob() -> AccountId32 {
    AccountId32::new([2u8; 32])
}

fn keypair(seed: u8) -> SigningKey<MlDsa87> {
    let mut s = Seed::default();
    s.iter_mut().for_each(|b| *b = seed);
    SigningKey::from_seed(&s)
}

/// Encoded ML-DSA-87 public key for `sk`.
fn pk_of(sk: &SigningKey<MlDsa87>) -> Vec<u8> {
    sk.verifying_key().encode().as_slice().to_vec()
}

/// Proof-of-possession: ML-DSA-87 signature over `b"GHOST-PQC-POP" || who`.
fn pop_for(sk: &SigningKey<MlDsa87>, who: &AccountId32) -> Vec<u8> {
    let mut msg = POP_DOMAIN.to_vec();
    msg.extend_from_slice(&who.encode());
    sk.sign(&msg).encode().as_slice().to_vec()
}

#[test]
fn register_pqc_key_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let sk = keypair(7);
        let who = alice();

        assert!(!<GhostPqc as PqcKeyProvider<_>>::has_pqc_key(&who));
        assert_ok!(GhostPqc::register_pqc_key(
            RuntimeOrigin::signed(who.clone()),
            pk_of(&sk),
            pop_for(&sk, &who),
        ));

        assert!(<GhostPqc as PqcKeyProvider<_>>::has_pqc_key(&who));
        assert_eq!(
            <GhostPqc as PqcKeyProvider<_>>::pqc_key(&who)
                .expect("key stored")
                .as_slice(),
            pk_of(&sk).as_slice()
        );
        System::assert_last_event(Event::PqcKeyRegistered { who }.into());
    });
}

#[test]
fn register_rejects_tampered_signature() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let sk = keypair(7);
        let who = alice();
        let mut sig = pop_for(&sk, &who);
        let mid = sig.len() / 2;
        sig[mid] ^= 0x01;

        assert_noop!(
            GhostPqc::register_pqc_key(RuntimeOrigin::signed(who), pk_of(&sk), sig,),
            Error::<Test>::InvalidSignature
        );
    });
}

#[test]
fn register_rejects_pop_for_wrong_account() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let sk = keypair(7);
        // Signature binds to bob's account but is submitted by alice — the
        // runtime builds the PoP message from the signer, so it must not verify.
        let sig = pop_for(&sk, &bob());
        assert_noop!(
            GhostPqc::register_pqc_key(RuntimeOrigin::signed(alice()), pk_of(&sk), sig,),
            Error::<Test>::InvalidSignature
        );
    });
}

#[test]
fn register_rejects_signature_under_other_key() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let who = alice();
        let real = keypair(7);
        let other = keypair(9);
        // PoP produced by `other`'s secret key, submitted with `real`'s pk.
        assert_noop!(
            GhostPqc::register_pqc_key(
                RuntimeOrigin::signed(who.clone()),
                pk_of(&real),
                pop_for(&other, &who),
            ),
            Error::<Test>::InvalidSignature
        );
    });
}

#[test]
fn register_rejects_oversized_key() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let sk = keypair(7);
        let who = alice();
        let mut big_key = pk_of(&sk);
        big_key.push(0);
        assert_noop!(
            GhostPqc::register_pqc_key(
                RuntimeOrigin::signed(who.clone()),
                big_key,
                pop_for(&sk, &who),
            ),
            Error::<Test>::KeyTooLarge
        );
    });
}

#[test]
fn register_rejects_undersized_key() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let sk = keypair(7);
        let who = alice();
        let small_key = pk_of(&sk)[..100].to_vec();
        assert_noop!(
            GhostPqc::register_pqc_key(
                RuntimeOrigin::signed(who.clone()),
                small_key,
                pop_for(&sk, &who),
            ),
            Error::<Test>::InvalidSignature
        );
    });
}

#[test]
fn register_rejects_double_registration() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let sk = keypair(7);
        let who = alice();
        let pk = pk_of(&sk);
        let sig = pop_for(&sk, &who);
        assert_ok!(GhostPqc::register_pqc_key(
            RuntimeOrigin::signed(who.clone()),
            pk.clone(),
            sig.clone(),
        ));
        assert_noop!(
            GhostPqc::register_pqc_key(RuntimeOrigin::signed(who), pk, sig,),
            Error::<Test>::KeyAlreadyRegistered
        );
    });
}

#[test]
fn revoke_releases_key_and_allows_reregister() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let sk = keypair(7);
        let who = alice();
        assert_ok!(GhostPqc::register_pqc_key(
            RuntimeOrigin::signed(who.clone()),
            pk_of(&sk),
            pop_for(&sk, &who),
        ));

        assert_ok!(GhostPqc::revoke_pqc_key(RuntimeOrigin::signed(who.clone())));
        assert!(!<GhostPqc as PqcKeyProvider<_>>::has_pqc_key(&who));
        System::assert_last_event(Event::PqcKeyRevoked { who: who.clone() }.into());

        // A different key can be registered afterwards (key rotation).
        let sk2 = keypair(8);
        assert_ok!(GhostPqc::register_pqc_key(
            RuntimeOrigin::signed(who.clone()),
            pk_of(&sk2),
            pop_for(&sk2, &who),
        ));
    });
}

#[test]
fn revoke_without_key_fails() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            GhostPqc::revoke_pqc_key(RuntimeOrigin::signed(alice())),
            Error::<Test>::KeyNotRegistered
        );
    });
}

#[test]
fn attest_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let sk = keypair(7);
        let who = alice();
        assert_ok!(GhostPqc::register_pqc_key(
            RuntimeOrigin::signed(who.clone()),
            pk_of(&sk),
            pop_for(&sk, &who),
        ));

        let block_hash = H256::repeat_byte(0xaa);
        let sig = sk.sign(block_hash.as_bytes()).encode().as_slice().to_vec();
        assert_ok!(GhostPqc::pqc_attest(
            RuntimeOrigin::signed(who.clone()),
            block_hash,
            sig,
        ));
        System::assert_last_event(Event::PqcAttested { who, block_hash }.into());
    });
}

#[test]
fn attest_requires_registered_key() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let sk = keypair(7);
        let block_hash = H256::repeat_byte(0xaa);
        let sig = sk.sign(block_hash.as_bytes()).encode().as_slice().to_vec();
        assert_noop!(
            GhostPqc::pqc_attest(RuntimeOrigin::signed(alice()), block_hash, sig),
            Error::<Test>::KeyNotRegistered
        );
    });
}

#[test]
fn attest_rejects_invalid_signature() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let sk = keypair(7);
        let who = alice();
        assert_ok!(GhostPqc::register_pqc_key(
            RuntimeOrigin::signed(who.clone()),
            pk_of(&sk),
            pop_for(&sk, &who),
        ));

        // Signature over a *different* block hash.
        let signed_hash = H256::repeat_byte(0xbb);
        let sig = sk.sign(signed_hash.as_bytes()).encode().as_slice().to_vec();
        assert_noop!(
            GhostPqc::pqc_attest(RuntimeOrigin::signed(who), H256::repeat_byte(0xaa), sig,),
            Error::<Test>::InvalidSignature
        );
    });
}

#[test]
fn set_pqc_required_is_root_only() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert!(!GhostPqc::pqc_required());
        assert_noop!(
            GhostPqc::set_pqc_required(RuntimeOrigin::signed(alice()), true),
            BadOrigin
        );

        assert_ok!(GhostPqc::set_pqc_required(RuntimeOrigin::root(), true));
        assert!(GhostPqc::pqc_required());
        System::assert_last_event(Event::PqcRequiredSet { required: true }.into());
    });
}
