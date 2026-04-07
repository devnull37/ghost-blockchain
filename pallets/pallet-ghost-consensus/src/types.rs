//! Types for the Ghost consensus pallet.

use codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_core::H256;

/// Block header stored by the Ghost consensus pallet.
#[derive(Clone, Debug, Default, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct GhostBlockHeader {
	pub number: u32,
	pub parent_hash: H256,
	pub state_root: H256,
	pub extrinsics_root: H256,
	pub nonce: u64,
	pub difficulty: u64,
	pub validator_signature: Option<H256>,
}

/// Stake tracked for a validator candidate.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct ValidatorStake<AccountId, Balance> {
	pub account: AccountId,
	pub stake: Balance,
	pub weight: u64,
}

/// Result of weighted validator selection.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct PosSelection<AccountId> {
	pub validator: AccountId,
	pub weight: u64,
	pub round: u64,
}

/// Block reward split between the miner and stakers.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct BlockReward<Balance> {
	pub total: Balance,
	pub miner_reward: Balance,
	pub stakers_reward: Balance,
}

/// Current phase of the Ghost pallet state machine.
#[derive(Clone, Debug, Default, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub enum ConsensusPhase {
	#[default]
	PowMining,
	PosValidation,
	Finalization,
}

/// Why a validator was slashed.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub enum SlashingReason {
	DoubleSigning,
	InvalidBlock,
	Downtime,
	Other,
}
