use sc_service::ChainType;
use solochain_template_runtime::WASM_BINARY;

/// Specialized `ChainSpec`. This is a specialization of the general Substrate ChainSpec type.
pub type ChainSpec = sc_service::GenericChainSpec;

/// Fetch a runtime genesis preset with the `grandpa` and `imOnline` sections
/// removed.
///
/// The runtime presets register each authority's GRANDPA and im-online keys
/// via `pallet_session`, and `SessionHandler::on_genesis_session` seeds
/// `pallet_grandpa::Authorities` / `pallet_im_online::Keys` itself. Each
/// pallet's own genesis build calls the same initialize routine, which
/// asserts the storage is still empty, so leaving `grandpa.authorities` or
/// `imOnline.keys` in the patch panics with "already initialized!" at
/// genesis. Session keys are the single source of truth for the genesis
/// committee.
fn genesis_preset(preset_name: &'static str) -> serde_json::Value {
    let mut patch: serde_json::Value = serde_json::from_slice(
        &solochain_template_runtime::genesis_config_presets::get_preset(
            &sp_genesis_builder::PresetId::from(preset_name),
        )
        .unwrap_or_else(|| panic!("unknown genesis preset '{preset_name}'")),
    )
    .expect("runtime genesis preset is valid JSON; qed");
    let map = patch
        .as_object_mut()
        .expect("runtime genesis preset is a JSON object; qed");
    map.remove("grandpa");
    map.remove("imOnline");
    patch
}

pub fn development_chain_spec() -> Result<ChainSpec, String> {
    Ok(ChainSpec::builder(
        WASM_BINARY.ok_or_else(|| "Development wasm not available".to_string())?,
        None,
    )
    .with_name("Ghost Development")
    .with_id("ghost-dev")
    .with_chain_type(ChainType::Development)
    .with_genesis_config_patch(genesis_preset("dev"))
    .build())
}

pub fn local_chain_spec() -> Result<ChainSpec, String> {
    Ok(ChainSpec::builder(
        WASM_BINARY.ok_or_else(|| "Development wasm not available".to_string())?,
        None,
    )
    .with_name("Ghost Local Testnet")
    .with_id("ghost-local")
    .with_chain_type(ChainType::Local)
    .with_genesis_config_patch(genesis_preset("local_testnet"))
    .build())
}
