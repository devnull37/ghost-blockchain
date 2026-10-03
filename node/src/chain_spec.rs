use sc_service::ChainType;
use solochain_template_runtime::WASM_BINARY;

/// Specialized `ChainSpec`. This is a specialization of the general Substrate ChainSpec type.
pub type ChainSpec = sc_service::GenericChainSpec;

/// Served verbatim by `system_properties` — wallets/explorers (incl. PJS
/// Apps) read tokenDecimals/tokenSymbol for display and ss58Format for
/// address rendering. An empty map leaves balances shown in raw planck.
fn ghost_properties() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "ss58Format": 42,
        "tokenDecimals": 12,
        "tokenSymbol": "GHOST",
    })
    .as_object()
    .expect("property literal is an object")
    .clone()
}

pub fn development_chain_spec() -> Result<ChainSpec, String> {
    Ok(ChainSpec::builder(
        WASM_BINARY.ok_or_else(|| "Development wasm not available".to_string())?,
        None,
    )
    .with_name("Ghost Development")
    .with_id("ghost-dev")
    .with_chain_type(ChainType::Development)
    .with_properties(ghost_properties())
    .with_genesis_config_preset_name("dev")
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
    .with_properties(ghost_properties())
    .with_genesis_config_preset_name("local_testnet")
    .build())
}
