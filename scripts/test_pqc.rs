// Test script for Dilithium-5 Signature Verification
// This mimics the logic in pallets/pallet-ghost-consensus/src/functions.rs

use pqcrypto_dilithium::dilithium5;
use pqcrypto_traits::sign::{DetachedSignature, PublicKey, SecretKey};

fn main() {
    println!("--- Ghost PQC Verification Test ---");

    // 1. Generate Keypair
    let (pk, sk) = dilithium5::keypair();
    println!("Generated Dilithium-5 Keypair.");
    println!("Public Key Size: {} bytes", pk.as_bytes().len());
    println!("Signature Size: {} bytes", dilithium5::detached_sign(b"test", &sk).as_bytes().len());

    // 2. Sign Message
    let message = b"Ghost Block #12345 Finalization";
    let sig = dilithium5::detached_sign(message, &sk);
    println!("Message signed.");

    // 3. Verify
    match dilithium5::verify_detached_signature(&sig, message, &pk) {
        Ok(_) => println!("✅ PQC Verification Successful!"),
        Err(_) => println!("❌ PQC Verification Failed!"),
    }
}
