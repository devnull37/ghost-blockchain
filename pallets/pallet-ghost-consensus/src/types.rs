//! Types for the Ghost consensus pallet.

use codec::{Decode, Encode, MaxEncodedLen};
use frame_support::{traits::ConstU32, BoundedVec};
use scale_info::TypeInfo;
use sp_core::H256;

/// Block header stored by the Ghost consensus pallet.
#[derive(Clone, Debug, Default, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
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

/// Evidence supplied with a slashing report.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub enum MisbehaviorEvidence {
    DoubleSigning { first_vote: H256, second_vote: H256 },
    InvalidBlock { block_number: u32 },
    Downtime,
    Other { proof_hash: H256 },
}

/// Claimed post-quantum primitive family recorded in metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub enum PqAlgorithm {
    #[default]
    Unknown,
    MlDsa44,
    MlDsa65,
    MlDsa87,
    Falcon512,
    Falcon1024,
    SphincsPlus,
    Hybrid,
    Other([u8; 16]),
}

/// Opaque proof artifact category tracked by claimed PQ metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub enum PqProofKind {
    #[default]
    Unknown,
    Signature,
    AggregateSignature,
    KeyEncapsulation,
    Attestation,
    Transcript,
    Other,
}

/// Versioned metadata describing a claimed PQ key or proof bundle.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct PqReadinessMetadata<BlockNumber> {
    pub version: u16,
    pub algorithm: PqAlgorithm,
    pub proof_kind: PqProofKind,
    pub key_strength_bits: u16,
    pub claimed_nist_level: Option<u8>,
    pub issued_at: Option<BlockNumber>,
    pub expires_at: Option<BlockNumber>,
    pub public_key_commitment: H256,
    pub metadata_hash: Option<H256>,
    pub flags: u8,
}

/// Bounded opaque PQ proof payload plus hashes for off-chain consistency checks.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub struct PqProofEnvelope<BlockNumber, const MAX_PROOF_BYTES: u32, const MAX_CONTEXT_BYTES: u32> {
    pub algorithm: PqAlgorithm,
    pub proof_kind: PqProofKind,
    pub submitted_at: BlockNumber,
    pub statement_hash: H256,
    pub public_key_commitment: H256,
    pub proof: BoundedVec<u8, ConstU32<MAX_PROOF_BYTES>>,
    pub context: BoundedVec<u8, ConstU32<MAX_CONTEXT_BYTES>>,
    pub auxiliary_hash: Option<H256>,
}

/// Default bounded opaque proof type for pallet storage and extrinsic payloads.
pub type DefaultPqProof<BlockNumber> = PqProofEnvelope<BlockNumber, 4096, 256>;

/// Backward-compatible alias used by test/genesis helpers.
pub type GenesisHeaderInit = (u32, H256, H256, H256, u64, u64, Option<H256>);
