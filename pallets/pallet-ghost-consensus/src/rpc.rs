//! RPC interface for Ghost Consensus Pallet

use codec::Codec;
use jsonrpsee::{
	core::{async_trait, RpcResult},
	proc_macros::rpc,
	types::error::{ErrorObject, ErrorObjectOwned},
};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use std::sync::Arc;

/// Ghost Consensus RPC API
#[rpc(client, server)]
pub trait GhostConsensusApi<BlockHash, AccountId, Balance> {
	/// Get current mining difficulty
	#[method(name = "ghost_getDifficulty")]
	fn get_difficulty(&self, at: Option<BlockHash>) -> RpcResult<u64>;

	/// Get current consensus phase
	#[method(name = "ghost_getCurrentPhase")]
	fn get_current_phase(&self, at: Option<BlockHash>) -> RpcResult<String>;

	/// Get validator stake
	#[method(name = "ghost_getValidatorStake")]
	fn get_validator_stake(
		&self,
		validator: AccountId,
		at: Option<BlockHash>,
	) -> RpcResult<Option<Balance>>;

	/// Get all validators
	#[method(name = "ghost_getAllValidators")]
	fn get_all_validators(&self, at: Option<BlockHash>) -> RpcResult<Vec<(AccountId, Balance)>>;

	/// Get slashing records
	#[method(name = "ghost_getSlashingRecords")]
	fn get_slashing_records(&self, at: Option<BlockHash>) -> RpcResult<u32>;
}

/// Runtime API definition for Ghost Consensus
sp_api::decl_runtime_apis! {
	pub trait GhostConsensusRuntimeApi<AccountId, Balance>
	where
		AccountId: Codec,
		Balance: Codec,
	{
		/// Get current mining difficulty
		fn get_difficulty() -> u64;

		/// Get current consensus phase
		fn get_current_phase() -> u8;

		/// Get validator stake
		fn get_validator_stake(validator: AccountId) -> Option<Balance>;

		/// Get all validators with their stakes
		fn get_all_validators() -> Vec<(AccountId, Balance)>;

		/// Get number of slashing records
		fn get_slashing_records_count() -> u32;
	}
}

/// Implementation of Ghost Consensus RPC API
pub struct GhostConsensusRpc<C, Block> {
	client: Arc<C>,
	_marker: std::marker::PhantomData<Block>,
}

impl<C, Block> GhostConsensusRpc<C, Block> {
	/// Create new instance
	pub fn new(client: Arc<C>) -> Self {
		Self { client, _marker: Default::default() }
	}
}

#[async_trait]
impl<C, Block, AccountId, Balance> GhostConsensusApiServer<Block::Hash, AccountId, Balance>
	for GhostConsensusRpc<C, Block>
where
	Block: BlockT,
	C: Send + Sync + 'static + ProvideRuntimeApi<Block> + HeaderBackend<Block>,
	C::Api: GhostConsensusRuntimeApi<Block, AccountId, Balance>,
	AccountId: Codec,
	Balance: Codec,
{
	fn get_difficulty(&self, at: Option<Block::Hash>) -> RpcResult<u64> {
		let api = self.client.runtime_api();
		let at = at.unwrap_or_else(|| self.client.info().best_hash);

		api.get_difficulty(at).map_err(runtime_error_into_rpc_error)
	}

	fn get_current_phase(&self, at: Option<Block::Hash>) -> RpcResult<String> {
		let api = self.client.runtime_api();
		let at = at.unwrap_or_else(|| self.client.info().best_hash);

		let phase_num = api.get_current_phase(at).map_err(runtime_error_into_rpc_error)?;

		let phase_name = match phase_num {
			0 => "PowMining",
			1 => "PosValidation",
			2 => "Finalization",
			_ => "Unknown",
		};

		Ok(phase_name.to_string())
	}

	fn get_validator_stake(
		&self,
		validator: AccountId,
		at: Option<Block::Hash>,
	) -> RpcResult<Option<Balance>> {
		let api = self.client.runtime_api();
		let at = at.unwrap_or_else(|| self.client.info().best_hash);

		api.get_validator_stake(at, validator).map_err(runtime_error_into_rpc_error)
	}

	fn get_all_validators(&self, at: Option<Block::Hash>) -> RpcResult<Vec<(AccountId, Balance)>> {
		let api = self.client.runtime_api();
		let at = at.unwrap_or_else(|| self.client.info().best_hash);

		api.get_all_validators(at).map_err(runtime_error_into_rpc_error)
	}

	fn get_slashing_records(&self, at: Option<Block::Hash>) -> RpcResult<u32> {
		let api = self.client.runtime_api();
		let at = at.unwrap_or_else(|| self.client.info().best_hash);

		api.get_slashing_records_count(at).map_err(runtime_error_into_rpc_error)
	}
}

/// Convert runtime error to RPC error
fn runtime_error_into_rpc_error(err: impl std::fmt::Display) -> ErrorObjectOwned {
	ErrorObject::owned(1, "Runtime error", Some(err.to_string()))
}
