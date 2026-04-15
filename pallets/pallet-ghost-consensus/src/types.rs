//! Types for the Ghost Consensus Pallet

use codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_core::H256;

/// Block header for Ghost consensus
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct GhostBlockHeader {
    /// Block number
    pub number: u32,
    /// Parent hash
    pub parent_hash: H256,
    /// State root
    pub state_root: H256,
    /// Extrinsics root
    pub extrinsics_root: H256,
    /// PoW nonce
    pub nonce: u64,
    /// PoW difficulty
    pub difficulty: u64,
    /// PoS validator signature
    pub validator_signature: Option<H256>,
    /// PQC finalization signature
    pub pqc_signature: Option<PqcSignature>,
}

/// PQC Signature for Crystal-Dilithium
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct PqcSignature(pub [u8; 4627]);

impl Default for PqcSignature {
    fn default() -> Self {
        Self([0u8; 4627])
    }
}

/// Mining difficulty adjustment
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct DifficultyAdjustment {
    /// Current difficulty
    pub current: u64,
    /// Target block time in milliseconds
    pub target_block_time: u64,
    /// Adjustment factor
    pub adjustment_factor: u64,
}

/// Validator stake information
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct ValidatorStake<AccountId, Balance> {
    /// Validator account
    pub account: AccountId,
    /// Staked amount
    pub stake: Balance,
    /// Stake weight for selection
    pub weight: u64,
}

/// PoW mining result
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct PowResult {
    /// Nonce found
    pub nonce: u64,
    /// Hash of the block
    pub hash: H256,
    /// Mining difficulty met
    pub difficulty: u64,
}

/// PoS validator selection result
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct PosSelection<AccountId> {
    /// Selected validator
    pub validator: AccountId,
    /// Selection weight
    pub weight: u64,
    /// Selection round
    pub round: u64,
}

/// Block reward distribution
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct BlockReward<Balance> {
    /// Total block reward
    pub total: Balance,
    /// Miner reward (40%)
    pub miner_reward: Balance,
    /// Stakers reward (60%)
    pub stakers_reward: Balance,
}

/// Consensus phase
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub enum ConsensusPhase {
    /// Proof-of-Work mining phase
    PowMining,
    /// Proof-of-Stake validation phase
    PosValidation,
    /// Block finalization phase
    Finalization,
}

impl Default for ConsensusPhase {
    fn default() -> Self {
        Self::PowMining
    }
}

/// Block validation status
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub enum BlockValidationStatus {
    /// Block is valid
    Valid,
    /// Block is invalid
    Invalid,
    /// Block validation pending
    Pending,
}

/// Slashing reason
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub enum SlashingReason {
    /// Double signing detected
    DoubleSigning,
    /// Invalid block produced
    InvalidBlock,
    /// Validator offline
    Downtime,
    /// Other slashing reason
    Other,
}
