//! A collection of node-specific RPC methods.
//! Substrate provides the `sc-rpc` crate, which defines the core RPC layer
//! used by Substrate nodes. This file extends those RPC definitions with
//! capabilities that are specific to this project's runtime configuration and
//! honest Ghost node status reporting.

#![warn(missing_docs)]

use std::sync::Arc;

use jsonrpsee::{
    core::{async_trait, RpcResult},
    proc_macros::rpc,
    RpcModule,
};
pub use sc_rpc_api::DenyUnsafe;
use sc_transaction_pool_api::TransactionPool;
use solochain_template_runtime::{opaque::Block, AccountId, Balance, Nonce};
use sp_api::ProvideRuntimeApi;
use sp_block_builder::BlockBuilder;
use sp_blockchain::{Error as BlockChainError, HeaderBackend, HeaderMetadata};

/// Full client dependencies.
pub struct FullDeps<C, P> {
    /// The client instance to use.
    pub client: Arc<C>,
    /// Transaction pool instance.
    pub pool: Arc<P>,
    /// Whether unsafe RPC calls should be denied.
    pub deny_unsafe: DenyUnsafe,
}

/// Ghost node RPC surface for honest status reporting.
#[rpc(client, server)]
pub trait GhostNodeApi {
    /// Report the actual block authoring and finality engines in this build.
    #[method(name = "ghost_getNodeStatus")]
    fn get_node_status(&self) -> RpcResult<String>;

    /// Report the consensus mode the node is currently using.
    #[method(name = "ghost_getConsensusMode")]
    fn get_consensus_mode(&self) -> RpcResult<String>;

    /// Report whether the Dilithium5 / PQC prototype is active in the node RPC surface.
    #[method(name = "ghost_getPqcStatus")]
    fn get_pqc_status(&self) -> RpcResult<String>;
}

/// Concrete Ghost node RPC implementation.
pub struct GhostNodeRpc;

impl GhostNodeRpc {
    /// Create a new RPC handler.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl GhostNodeApiServer for GhostNodeRpc {
    fn get_node_status(&self) -> RpcResult<String> {
        Ok("Ghost node uses Aura for authoring and GRANDPA for finality in this build.".into())
    }

    fn get_consensus_mode(&self) -> RpcResult<String> {
        Ok(
            "Aura + GRANDPA live; Ghost hybrid PoW + PoS remains informational/prototype code."
                .into(),
        )
    }

    fn get_pqc_status(&self) -> RpcResult<String> {
        Ok("Dilithium5 support is not wired into the live node RPC surface yet.".into())
    }
}

/// Instantiate all full RPC extensions.
pub fn create_full<C, P>(
    deps: FullDeps<C, P>,
) -> Result<RpcModule<()>, Box<dyn std::error::Error + Send + Sync>>
where
    C: ProvideRuntimeApi<Block>,
    C: HeaderBackend<Block> + HeaderMetadata<Block, Error = BlockChainError> + 'static,
    C: Send + Sync + 'static,
    C::Api: substrate_frame_rpc_system::AccountNonceApi<Block, AccountId, Nonce>,
    C::Api: pallet_transaction_payment_rpc::TransactionPaymentRuntimeApi<Block, Balance>,
    C::Api: BlockBuilder<Block>,
    P: TransactionPool + 'static,
{
    use pallet_transaction_payment_rpc::{TransactionPayment, TransactionPaymentApiServer};
    use substrate_frame_rpc_system::{System, SystemApiServer};

    let mut module = RpcModule::new(());
    let FullDeps {
        client,
        pool,
        deny_unsafe,
    } = deps;

    module.merge(System::new(client.clone(), pool, deny_unsafe).into_rpc())?;
    module.merge(TransactionPayment::new(client).into_rpc())?;
    module.merge(GhostNodeRpc::new().into_rpc())?;

    Ok(module)
}
