// This file is part of Substrate.

// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::{AccountId, BalancesConfig, Runtime, SudoConfig};
use alloc::{vec, vec::Vec};
use serde_json::Value;
use sp_consensus_aura::sr25519::AuthorityId as AuraId;
use sp_consensus_grandpa::AuthorityId as GrandpaId;
use sp_core::{crypto::AccountId32, ed25519, sr25519};
use sp_genesis_builder::{self, PresetId};

const ALICE_SR25519: [u8; 32] =
    hex_literal::hex!("d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d");
const BOB_SR25519: [u8; 32] =
    hex_literal::hex!("8eaf04151687736326c9fea17e25fc5287613693c912909cb226aa4794f26a48");
const ALICE_STASH_SR25519: [u8; 32] =
    hex_literal::hex!("be5ddb1579b72e84524fc29e78609e3caf42e85aa118ebfe0b0ad404b5bdd25f");
const BOB_STASH_SR25519: [u8; 32] =
    hex_literal::hex!("fe65717dad0447d715f660a0a58411de509b42e6efb8375f562f58a554d5860e");
const CHARLIE_SR25519: [u8; 32] =
    hex_literal::hex!("90b5ab205c6974c9ea841be688864633dc9ca8a357843eeacf2314649965fe22");
const DAVE_SR25519: [u8; 32] =
    hex_literal::hex!("306721211d5404bd9da88e0204360a1a9ab8b87c66c1bc2fcdd37f3c2222cc20");
const EVE_SR25519: [u8; 32] =
    hex_literal::hex!("e659a7a1628cdd93febc04a4e0646ea20e9f5f0ce097d9a05290d4a9e054df4e");
const FERDIE_SR25519: [u8; 32] =
    hex_literal::hex!("1cbd2d43530a44705ad088af313e18f80b53ef16b36177cd4b77b846f2a5f07c");
const ALICE_ED25519: [u8; 32] =
    hex_literal::hex!("88dc3417d5058ec4b4503e0c12ea1a0a89be200fe98922423d4334014fa6b0ee");
const BOB_ED25519: [u8; 32] =
    hex_literal::hex!("d17c2d7823ebf260fd138f2d7e27d114c0145d968b5ff5006125f2414fadae69");

fn account(public: [u8; 32]) -> AccountId {
    AccountId32::from(public).into()
}

fn aura(public: [u8; 32]) -> AuraId {
    sr25519::Public::from_raw(public).into()
}

fn grandpa(public: [u8; 32]) -> GrandpaId {
    ed25519::Public::from_raw(public).into()
}

// Returns the genesis config presets populated with given parameters.
fn testnet_genesis(
    initial_authorities: Vec<(AuraId, GrandpaId)>,
    endowed_accounts: Vec<AccountId>,
    root: AccountId,
) -> Value {
    serde_json::json!({
        "balances": BalancesConfig {
            balances: endowed_accounts
                .iter()
                .cloned()
                .map(|k| (k, 1u128 << 60))
                .collect::<Vec<_>>(),
        },
        "aura": pallet_aura::GenesisConfig::<Runtime> {
            authorities: initial_authorities.iter().map(|x| x.0.clone()).collect::<Vec<_>>(),
        },
        "grandpa": pallet_grandpa::GenesisConfig::<Runtime> {
            authorities: initial_authorities.iter().map(|x| (x.1.clone(), 1)).collect::<Vec<_>>(),
            _config: Default::default(),
        },
        "sudo": SudoConfig { key: Some(root) },
    })
}

/// Return the development genesis config.
pub fn development_config_genesis() -> Value {
    testnet_genesis(
        vec![(aura(ALICE_SR25519), grandpa(ALICE_ED25519))],
        vec![
            account(ALICE_SR25519),
            account(BOB_SR25519),
            account(ALICE_STASH_SR25519),
            account(BOB_STASH_SR25519),
        ],
        account(ALICE_SR25519),
    )
}

/// Return the local genesis config preset.
pub fn local_config_genesis() -> Value {
    testnet_genesis(
        vec![
            (aura(ALICE_SR25519), grandpa(ALICE_ED25519)),
            (aura(BOB_SR25519), grandpa(BOB_ED25519)),
        ],
        vec![
            account(ALICE_SR25519),
            account(BOB_SR25519),
            account(ALICE_STASH_SR25519),
            account(BOB_STASH_SR25519),
            account(CHARLIE_SR25519),
            account(DAVE_SR25519),
            account(EVE_SR25519),
            account(FERDIE_SR25519),
        ],
        account(ALICE_SR25519),
    )
}

/// Provides the JSON representation of predefined genesis config for given `id`.
pub fn get_preset(id: &PresetId) -> Option<Vec<u8>> {
    let patch = match id.as_ref() {
        b"dev" => development_config_genesis(),
        b"local_testnet" => local_config_genesis(),
        _ => return None,
    };
    Some(
        serde_json::to_string(&patch)
            .expect("serialization to json is expected to work. qed.")
            .into_bytes(),
    )
}

/// List of supported presets.
pub fn preset_names() -> Vec<PresetId> {
    vec![PresetId::from("dev"), PresetId::from("local_testnet")]
}
