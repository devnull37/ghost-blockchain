// This is free and unencumbered software released into the public domain.
// See <http://unlicense.org>

use crate::{
    AccountId, Balance, BalancesConfig, GhostConsensusConfig, RuntimeGenesisConfig, SudoConfig,
    UNIT,
};
use alloc::{vec, vec::Vec};
use pallet_ghost_consensus::types::GenesisHeaderInit;
use serde_json::Value;
use sp_core::H256;
use sp_genesis_builder::PresetId;
use sp_keyring::Sr25519Keyring;
use sp_runtime::traits::{BlakeTwo256, Hash};

const ENDOWMENT: Balance = 1u128 << 60;
const INITIAL_GHOST_STAKE: Balance = 2 * UNIT;

fn ghost_genesis_header() -> GenesisHeaderInit {
    (
        0,
        H256::zero(),
        BlakeTwo256::hash_of(&(0u32, "ghost-genesis-state")),
        BlakeTwo256::hash_of(&(0u32, "ghost-genesis-extrinsics")),
        0,
        1_000_000_000_000,
        None,
    )
}

/// Build a genesis config preset. PoW authoring needs no consensus authorities, so only
/// balances, sudo, and the Ghost consensus (PoS) staking state are seeded.
fn testnet_genesis(
    endowed_accounts: Vec<AccountId>,
    root: AccountId,
    ghost_validators: Vec<AccountId>,
) -> Value {
    let ghost_validator_stakes = ghost_validators
        .into_iter()
        .map(|account| (account, INITIAL_GHOST_STAKE))
        .collect::<Vec<_>>();

    // stable2407 has no `build_struct_json_patch!`; build the full genesis config and
    // serialize it. Unset pallet configs fall back to their defaults.
    let config = RuntimeGenesisConfig {
        balances: BalancesConfig {
            balances: endowed_accounts
                .iter()
                .cloned()
                .map(|k| (k, ENDOWMENT))
                .collect::<Vec<_>>(),
        },
        sudo: SudoConfig { key: Some(root) },
        ghost_consensus: GhostConsensusConfig {
            genesis_header: Some(ghost_genesis_header()),
            validator_stakes: ghost_validator_stakes,
        },
        ..Default::default()
    };

    serde_json::to_value(&config).expect("Genesis config must be serializable to JSON. qed.")
}

/// Return the development genesis config.
pub fn development_config_genesis() -> Value {
    let alice = Sr25519Keyring::Alice.to_account_id();
    let bob = Sr25519Keyring::Bob.to_account_id();

    testnet_genesis(
        vec![
            alice.clone(),
            bob.clone(),
            Sr25519Keyring::AliceStash.to_account_id(),
            Sr25519Keyring::BobStash.to_account_id(),
        ],
        alice.clone(),
        vec![alice, bob],
    )
}

/// Return the local genesis config preset.
pub fn local_config_genesis() -> Value {
    testnet_genesis(
        Sr25519Keyring::iter()
            .filter(|v| v != &Sr25519Keyring::One && v != &Sr25519Keyring::Two)
            .map(|v| v.to_account_id())
            .collect::<Vec<_>>(),
        Sr25519Keyring::Alice.to_account_id(),
        vec![
            Sr25519Keyring::Alice.to_account_id(),
            Sr25519Keyring::Bob.to_account_id(),
            Sr25519Keyring::Charlie.to_account_id(),
        ],
    )
}

/// Preset identifiers. In stable2407 `PresetId` is a plain runtime string, so we define
/// the names locally instead of importing constants that do not exist in this SDK version.
pub const DEV_PRESET: &str = "development";
pub const LOCAL_PRESET: &str = "local_testnet";

/// Provides the JSON representation of predefined genesis config for given `id`.
pub fn get_preset(id: &PresetId) -> Option<Vec<u8>> {
    let patch = if id == &PresetId::from(DEV_PRESET) {
        development_config_genesis()
    } else if id == &PresetId::from(LOCAL_PRESET) {
        local_config_genesis()
    } else {
        return None;
    };
    Some(
        serde_json::to_string(&patch)
            .expect("serialization to json is expected to work. qed.")
            .into_bytes(),
    )
}

/// List of supported presets.
pub fn preset_names() -> Vec<PresetId> {
    vec![PresetId::from(DEV_PRESET), PresetId::from(LOCAL_PRESET)]
}
