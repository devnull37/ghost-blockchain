use sc_service::ChainType;
use solochain_template_runtime::WASM_BINARY;

/// Specialized `ChainSpec`. This is a specialization of the general Substrate ChainSpec type.
pub type ChainSpec = sc_service::GenericChainSpec;

pub fn development_chain_spec() -> Result<ChainSpec, String> {
    Ok(ChainSpec::builder(
        WASM_BINARY.ok_or_else(|| "Development wasm not available".to_string())?,
        None,
    )
    .with_name("Ghost Development")
    .with_id("ghost-dev")
    .with_chain_type(ChainType::Development)
    // Claimed PQ metadata registry data is not exposed here yet because the runtime preset only accepts
    // fields that the pallet genesis APIs support. Extend the runtime preset first if the pallet
    // gains a dedicated claimed-PQ-metadata genesis field.
    .with_genesis_config_preset_name(solochain_template_runtime::genesis_config_presets::DEV_PRESET)
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
    // Keep local/dev presets aligned: claimed PQ metadata seeding belongs in the runtime genesis payload
    // once pallet-ghost-consensus exposes that surface, not in the chain spec wrapper itself.
    .with_genesis_config_preset_name(solochain_template_runtime::genesis_config_presets::LOCAL_PRESET)
    .build())
}
