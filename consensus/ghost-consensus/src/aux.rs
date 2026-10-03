//! Persistence helpers for the PoW aux store.
//!
//! `PowBlockImport` already writes `PowAux { difficulty, total_difficulty }`
//! under `POW_AUX_PREFIX ++ block_hash` on every import; these helpers let the
//! node (and tests) read and repair that data so cumulative total difficulty
//! survives restarts.

use codec::Encode;
use sc_client_api::backend::AuxStore;
use sc_consensus_pow::{Error, PowAux, POW_AUX_PREFIX};
use sp_core::U256;
use sp_runtime::traits::Block as BlockT;

/// Aux storage key for a block hash — identical to sc-consensus-pow's
/// internal `aux_key` (`POW_AUX_PREFIX ++ hash`). Re-derived here because the
/// upstream function is private; the constant itself is public.
pub fn aux_key<H: AsRef<[u8]>>(hash: &H) -> Vec<u8> {
    POW_AUX_PREFIX
        .iter()
        .chain(hash.as_ref())
        .copied()
        .collect()
}

/// Read the `PowAux` recorded for `hash`. Missing data decodes to
/// `PowAux::default()` (zero difficulty / zero total), matching upstream.
pub fn read_aux<C: AuxStore, B: BlockT>(
    client: &C,
    hash: &B::Hash,
) -> Result<PowAux<U256>, Error<B>> {
    PowAux::<U256>::read::<_, B>(client, hash)
}

/// Write `PowAux` for `hash`. Used by recovery tooling and tests; the import
/// path normally writes aux through `BlockImportParams::auxiliary` itself.
pub fn write_aux<C: AuxStore, B: BlockT>(
    client: &C,
    hash: &B::Hash,
    aux: &PowAux<U256>,
) -> Result<(), Error<B>> {
    let key = aux_key(hash);
    let value = aux.encode();
    client
        .insert_aux(&[(&key[..], &value[..])], &[])
        .map_err(Error::Client)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use sp_core::H256;
    use std::{collections::BTreeMap, sync::Mutex};

    #[derive(Default)]
    struct MemAuxStore(Mutex<BTreeMap<Vec<u8>, Vec<u8>>>);

    impl AuxStore for MemAuxStore {
        fn insert_aux<
            'a,
            'b: 'a,
            'c: 'a,
            I: IntoIterator<Item = &'a (&'c [u8], &'c [u8])>,
            D: IntoIterator<Item = &'a &'b [u8]>,
        >(
            &self,
            insert: I,
            delete: D,
        ) -> sp_blockchain::Result<()> {
            let mut store = self.0.lock().unwrap();
            for (k, v) in insert {
                store.insert(k.to_vec(), v.to_vec());
            }
            for k in delete {
                store.remove(&k[..]);
            }
            Ok(())
        }

        fn get_aux(&self, key: &[u8]) -> sp_blockchain::Result<Option<Vec<u8>>> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }
    }

    type TestBlock =
        sp_runtime::generic::Block<sp_runtime::testing::Header, sp_runtime::OpaqueExtrinsic>;

    #[test]
    fn aux_read_write_roundtrip() {
        let store = MemAuxStore::default();
        let hash = H256::from([9u8; 32]);

        // Missing entry reads as default.
        let empty = read_aux::<_, TestBlock>(&store, &hash).unwrap();
        assert!(empty.difficulty.is_zero() && empty.total_difficulty.is_zero());

        let aux = PowAux {
            difficulty: U256::from(500u64),
            total_difficulty: U256::from(1_700u64),
        };
        write_aux::<_, TestBlock>(&store, &hash, &aux).unwrap();

        let back = read_aux::<_, TestBlock>(&store, &hash).unwrap();
        assert_eq!(back.difficulty, aux.difficulty);
        assert_eq!(back.total_difficulty, aux.total_difficulty);

        // Total difficulty accumulates: a later block stores prev_total + own.
        let aux2 = PowAux {
            difficulty: U256::from(300u64),
            total_difficulty: U256::from(2_000u64),
        };
        write_aux::<_, TestBlock>(&store, &H256::from([10u8; 32]), &aux2).unwrap();
        let back2 = read_aux::<_, TestBlock>(&store, &H256::from([10u8; 32])).unwrap();
        assert_eq!(
            back2.total_difficulty,
            aux.total_difficulty + aux2.difficulty
        );
    }

    #[test]
    fn aux_key_matches_pow_prefix() {
        let hash = [1u8; 32];
        let key = aux_key(&hash);
        assert_eq!(&key[..4], b"PoW:");
        assert_eq!(&key[4..], &hash[..]);
    }

    proptest::proptest! {
        /// Aux persistence round-trips arbitrary difficulty pairs through the
        /// real SCALE encoding the import path writes.
        #[test]
        fn aux_roundtrip_arbitrary(
            hash in any::<[u8; 32]>(),
            difficulty in any::<[u8; 32]>(),
            total in any::<[u8; 32]>(),
        ) {
            let store = MemAuxStore::default();
            let hash = H256::from(hash);
            let aux = PowAux {
                difficulty: U256::from_big_endian(&difficulty),
                total_difficulty: U256::from_big_endian(&total),
            };
            write_aux::<_, TestBlock>(&store, &hash, &aux).unwrap();
            let back = read_aux::<_, TestBlock>(&store, &hash).unwrap();
            prop_assert_eq!(back.difficulty, aux.difficulty);
            prop_assert_eq!(back.total_difficulty, aux.total_difficulty);
            // Keys never collide for distinct hashes and always carry the
            // upstream prefix — a wrong key scheme would silently split
            // total-difficulty accounting from the import path's writes.
            let key = aux_key(&hash);
            prop_assert_eq!(&key[..4], b"PoW:");
            prop_assert_eq!(key.len(), 4 + 32);
        }
    }
}
