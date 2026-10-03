#![no_main]

//! Fuzz the digest parsing the import path performs on every header:
//! `Digest`/`DigestItem` SCALE decode, the `pow_` pre-runtime author lookup
//! (`author_from_header`), and `author_from_pre_digest` on arbitrary payloads.
//! A crash or author mis-attribution here is remote-reachable.

use ghost_consensus::{
    author_from_header, author_from_pre_digest, miner_digest_item, POW_ENGINE_ID,
};
use libfuzzer_sys::fuzz_target;
use parity_scale_codec::{Decode, Encode};
use sp_core::{crypto::AccountId32, H256};
use sp_runtime::{generic::Digest, traits::BlakeTwo256, traits::Header as _, DigestItem};

type Header = sp_runtime::generic::Header<u32, BlakeTwo256>;

fuzz_target!(|data: &[u8]| {
    // 1. Arbitrary bytes -> Digest: decode must never panic, and whatever it
    //    accepts must re-encode to a prefix of the input.
    if let Ok(digest) = Digest::decode(&mut &data[..]) {
        let consumed = digest.encode().len();
        assert!(consumed <= data.len());
        assert_eq!(digest.encode().as_slice(), &data[..consumed]);

        // 2. The author lookup must be panic-free and agree with an
        //    independent scan: the first `pow_` pre-runtime item whose
        //    payload decodes to an AccountId32 wins; a malformed `pow_`
        //    payload falls through to the next item.
        let header = Header::new(0, H256::zero(), H256::zero(), H256::zero(), digest.clone());
        let author = author_from_header(&header);
        let expected = digest.logs().iter().find_map(|log| match log {
            DigestItem::PreRuntime(id, bytes) if *id == POW_ENGINE_ID => {
                author_from_pre_digest(bytes).ok()
            }
            _ => None,
        });
        assert_eq!(author, expected);
    }

    // 3. The payload decoder alone: AccountId32 is a fixed 32-byte decode
    //    that ignores trailing bytes — pin that contract.
    match author_from_pre_digest(data) {
        Ok(account) => {
            assert!(data.len() >= 32);
            assert_eq!(account.encode().as_slice(), &data[..32]);
        }
        Err(_) => assert!(data.len() < 32),
    }

    // 4. Round-trip the real encoder: a digest built by miner_digest_item
    //    must always yield its author back.
    let mut author_bytes = [0u8; 32];
    let n = data.len().min(32);
    author_bytes[..n].copy_from_slice(&data[..n]);
    let author = AccountId32::from(author_bytes);
    let mut digest = Digest::default();
    digest.push(miner_digest_item(&author));
    let header = Header::new(0, H256::zero(), H256::zero(), H256::zero(), digest);
    assert_eq!(author_from_header(&header), Some(author));
});
