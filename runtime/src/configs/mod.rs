// This is free and unencumbered software released into the public domain.
//
// Anyone is free to copy, modify, publish, use, compile, sell, or
// distribute this software, either in source code form or as a compiled
// binary, for any purpose, commercial or non-commercial, and by any
// means.
//
// In jurisdictions that recognize copyright laws, the author or authors
// of this software dedicate any and all copyright interest in the
// software to the public domain. We make this dedication for the benefit
// of the public at large and to the detriment of our heirs and
// successors. We intend this dedication to be an overt act of
// relinquishment in perpetuity of all present and future rights to this
// software under copyright law.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
// EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
// MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
// IN NO EVENT SHALL THE AUTHORS BE LIABLE FOR ANY CLAIM, DAMAGES OR
// OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE,
// ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
// OTHER DEALINGS IN THE SOFTWARE.
//
// For more information, please refer to <http://unlicense.org>

// Substrate and Polkadot dependencies
use frame_support::{
    derive_impl, parameter_types,
    traits::{
        ConstBool, ConstU128, ConstU32, ConstU64, ConstU8, KeyOwnerProofSystem, VariantCountOf,
    },
    weights::{
        constants::{RocksDbWeight, WEIGHT_REF_TIME_PER_SECOND},
        IdentityFee, Weight,
    },
};
use frame_system::limits::{BlockLength, BlockWeights};
use pallet_grandpa::AuthorityId as GrandpaId;
use pallet_im_online::sr25519::AuthorityId as ImOnlineId;
use pallet_transaction_payment::{ConstFeeMultiplier, FungibleAdapter, Multiplier};
use sp_core::{crypto::KeyTypeId, U256};
use sp_runtime::{
    traits::{Convert, One, OpaqueKeys, Zero},
    transaction_validity::TransactionPriority,
    Perbill,
};
use sp_version::RuntimeVersion;

// Local module imports
use super::{
    AccountId, Balance, Balances, Block, BlockNumber, GhostConsensus, Hash, Historical, ImOnline,
    Nonce, Offences, PalletInfo, Runtime, RuntimeCall, RuntimeEvent, RuntimeFreezeReason,
    RuntimeHoldReason, RuntimeOrigin, RuntimeTask, SessionKeys, Signature, System, DAYS,
    EXISTENTIAL_DEPOSIT, SLOT_DURATION, UNIT, VERSION,
};

const NORMAL_DISPATCH_RATIO: Perbill = Perbill::from_percent(75);

parameter_types! {
    pub const BlockHashCount: BlockNumber = 2400;
    pub const Version: RuntimeVersion = VERSION;

    /// We allow for 2 seconds of compute with a 6 second average block time.
    pub RuntimeBlockWeights: BlockWeights = BlockWeights::with_sensible_defaults(
        Weight::from_parts(2u64 * WEIGHT_REF_TIME_PER_SECOND, u64::MAX),
        NORMAL_DISPATCH_RATIO,
    );
    pub RuntimeBlockLength: BlockLength = BlockLength::max_with_normal_ratio(5 * 1024 * 1024, NORMAL_DISPATCH_RATIO);
    pub const SS58Prefix: u8 = 42;
}

/// All migrations of the runtime, aside from the ones declared in the pallets.
///
/// This can be a tuple of types, each implementing `OnRuntimeUpgrade`.
#[allow(unused_parens)]
type SingleBlockMigrations = ();

/// The default types are being injected by [`derive_impl`](`frame_support::derive_impl`) from
/// [`SoloChainDefaultConfig`](`struct@frame_system::config_preludes::SolochainDefaultConfig`),
/// but overridden as needed.
#[derive_impl(frame_system::config_preludes::SolochainDefaultConfig)]
impl frame_system::Config for Runtime {
    /// The block type for the runtime.
    type Block = Block;
    /// Block & extrinsics weights: base values and limits.
    type BlockWeights = RuntimeBlockWeights;
    /// The maximum length of a block (in bytes).
    type BlockLength = RuntimeBlockLength;
    /// The identifier used to distinguish between accounts.
    type AccountId = AccountId;
    /// The type for storing how many extrinsics an account has signed.
    type Nonce = Nonce;
    /// The type for hashing blocks and tries.
    type Hash = Hash;
    /// Maximum number of block number to block hash mappings to keep (oldest pruned first).
    type BlockHashCount = BlockHashCount;
    /// The weight of database operations that the runtime can invoke.
    type DbWeight = RocksDbWeight;
    /// Version of the runtime.
    type Version = Version;
    /// The data to be stored in an account.
    type AccountData = pallet_balances::AccountData<Balance>;
    /// This is used as an identifier of the chain. 42 is the generic substrate prefix.
    type SS58Prefix = SS58Prefix;
    type MaxConsumers = frame_support::traits::ConstU32<16>;
    type SingleBlockMigrations = SingleBlockMigrations;
}

/// Session length in blocks (design doc: 20 for dev chains).
pub const SESSION_PERIOD: u32 = 20;

parameter_types! {
    pub const Period: u32 = SESSION_PERIOD;
    pub const Offset: u32 = 0;
    /// Equivocation reports are valid this many blocks after the offence.
    pub const ReportLongevity: u64 = 20 * SESSION_PERIOD as u64;
    /// Im-online heartbeat unsigned-tx priority.
    pub ImOnlineUnsignedPriority: TransactionPriority = TransactionPriority::MAX / 2;
    /// Floor for the PoW difficulty work factor (and genesis seed).
    pub MinDifficulty: U256 = U256::from(1_000_000u64);
}

/// Converts an account id to its validator id (identity on Ghost —
/// validator ids are account ids).
pub struct ValidatorIdOf;
impl Convert<AccountId, Option<AccountId>> for ValidatorIdOf {
    fn convert(a: AccountId) -> Option<AccountId> {
        Some(a)
    }
}

/// The offence identification stored in the historical session trie: the
/// validator's bonded stake at session time (used to compute slash amounts).
pub struct FullIdentificationOf;
impl Convert<AccountId, Option<Balance>> for FullIdentificationOf {
    fn convert(a: AccountId) -> Option<Balance> {
        let bonded = GhostConsensus::bonded(&a);
        (!bonded.is_zero()).then_some(bonded)
    }
}

/// `validate` gating: a candidate must have session keys queued via
/// `pallet_session::set_keys`.
pub struct SessionKeysLookup;
impl pallet_ghost_consensus::SessionKeysLookup<AccountId> for SessionKeysLookup {
    fn keys_registered(who: &AccountId) -> bool {
        pallet_session::NextKeys::<Runtime>::contains_key(who)
    }
}

impl pallet_session::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type ValidatorId = AccountId;
    type ValidatorIdOf = ValidatorIdOf;
    type ShouldEndSession = pallet_session::PeriodicSessions<Period, Offset>;
    type NextSessionRotation = pallet_session::PeriodicSessions<Period, Offset>;
    /// Validator selection comes from Ghost consensus stake; the wrapper
    /// also stores historical session tries for equivocation proofs.
    type SessionManager = pallet_session::historical::NoteHistoricalRoot<Runtime, GhostConsensus>;
    type SessionHandler = <SessionKeys as OpaqueKeys>::KeyTypeIdProviders;
    type Keys = SessionKeys;
    type WeightInfo = pallet_session::weights::SubstrateWeight<Runtime>;
}

impl pallet_session::historical::Config for Runtime {
    type FullIdentification = Balance;
    type FullIdentificationOf = FullIdentificationOf;
}

impl pallet_authorship::Config for Runtime {
    /// PoW authors are decoded from the pow_ digest by pallet-ghost-consensus;
    /// there is no session-indexed author map.
    type FindAuthor = ();
    /// im-online counts authored blocks for its liveness tracking.
    type EventHandler = ImOnline;
}

impl pallet_offences::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type IdentificationTuple = pallet_session::historical::IdentificationTuple<Self>;
    /// Slashing lands in the Ghost consensus pallet's OnOffenceHandler.
    type OnOffenceHandler = GhostConsensus;
}

impl pallet_im_online::Config for Runtime {
    type AuthorityId = ImOnlineId;
    type RuntimeEvent = RuntimeEvent;
    type ValidatorSet = Historical;
    type NextSessionRotation = pallet_session::PeriodicSessions<Period, Offset>;
    type ReportUnresponsiveness = Offences;
    type UnsignedPriority = ImOnlineUnsignedPriority;
    type WeightInfo = pallet_im_online::weights::SubstrateWeight<Runtime>;
    type MaxKeys = ConstU32<10_000>;
    type MaxPeerInHeartbeats = ConstU32<1_000>;
}

impl frame_system::offchain::SigningTypes for Runtime {
    type Public = <Signature as sp_runtime::traits::Verify>::Signer;
    type Signature = Signature;
}

impl<C> frame_system::offchain::SendTransactionTypes<C> for Runtime
where
    RuntimeCall: From<C>,
{
    type Extrinsic = super::UncheckedExtrinsic;
    type OverarchingCall = RuntimeCall;
}

impl pallet_grandpa::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;

    type WeightInfo = ();
    type MaxAuthorities = ConstU32<100>;
    type MaxNominators = ConstU32<0>;
    /// One entry per session; keep enough for the report-longevity window.
    type MaxSetIdSessionEntries = ConstU64<20>;

    type KeyOwnerProof = <Historical as KeyOwnerProofSystem<(KeyTypeId, GrandpaId)>>::Proof;
    type EquivocationReportSystem =
        pallet_grandpa::EquivocationReportSystem<Self, Offences, Historical, ReportLongevity>;
}

impl pallet_timestamp::Config for Runtime {
    /// A timestamp: milliseconds since the unix epoch.
    type Moment = u64;
    /// No slot consensus owns timestamp notes; PoW clients set timestamps.
    type OnTimestampSet = ();
    type MinimumPeriod = ConstU64<{ SLOT_DURATION / 2 }>;
    type WeightInfo = ();
}

impl pallet_balances::Config for Runtime {
    type MaxLocks = ConstU32<50>;
    type MaxReserves = ();
    type ReserveIdentifier = [u8; 8];
    /// The type for recording an account's balance.
    type Balance = Balance;
    /// The ubiquitous event type.
    type RuntimeEvent = RuntimeEvent;
    type DustRemoval = ();
    type ExistentialDeposit = ConstU128<EXISTENTIAL_DEPOSIT>;
    type AccountStore = System;
    type WeightInfo = pallet_balances::weights::SubstrateWeight<Runtime>;
    type FreezeIdentifier = RuntimeFreezeReason;
    type MaxFreezes = VariantCountOf<RuntimeFreezeReason>;
    type RuntimeHoldReason = RuntimeHoldReason;
    type RuntimeFreezeReason = RuntimeFreezeReason;
}

parameter_types! {
    pub FeeMultiplier: Multiplier = Multiplier::one();
    pub const GhostPalletId: frame_support::PalletId = frame_support::PalletId(*b"py/ghost");
}

impl pallet_transaction_payment::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type OnChargeTransaction = FungibleAdapter<Balances, ()>;
    type OperationalFeeMultiplier = ConstU8<5>;
    type WeightToFee = IdentityFee<Balance>;
    type LengthToFee = IdentityFee<Balance>;
    type FeeMultiplierUpdate = ConstFeeMultiplier<FeeMultiplier>;
}

impl pallet_sudo::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type RuntimeCall = RuntimeCall;
    type WeightInfo = pallet_sudo::weights::SubstrateWeight<Runtime>;
}

/// Configure the pallet-template in pallets/template.
impl pallet_template::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_template::weights::SubstrateWeight<Runtime>;
}

/// Configure the Ghost Consensus pallet (v2): staking holds, session
/// selection, difficulty retarget, digest-decoded rewards, offence slashing.
impl pallet_ghost_consensus::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type Currency = Balances;
    type RuntimeHoldReason = RuntimeHoldReason;
    type WeightInfo = ();
    type PalletId = GhostPalletId;
    type BlockReward = ConstU128<{ 10 * UNIT }>;
    type MinStake = ConstU128<UNIT>;
    type MaxStake = ConstU128<{ 1_000_000 * UNIT }>;
    type MaxValidators = ConstU32<100>;
    type MaxValidatorCandidates = ConstU32<1024>;
    type UnbondingPeriod = ConstU32<{ 14 * DAYS }>;
    type MaxUnbondingChunks = ConstU32<16>;
    type RetargetInterval = ConstU32<100>;
    type TargetBlockTimeMs = ConstU64<SLOT_DURATION>;
    type MinDifficulty = MinDifficulty;
    type SessionKeysLookup = SessionKeysLookup;
    /// () until pallet-ghost-pqc lands; RequirePqcKey stays false until then
    /// (no_std PQC verification per AGENTS.md).
    type PqcProvider = ();
    type RequirePqcKey = ConstBool<false>;
}
