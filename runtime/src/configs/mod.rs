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
    derive_impl,
    pallet_prelude::BoundedVec,
    parameter_types,
    traits::{
        ConstBool, ConstU128, ConstU32, ConstU64, ConstU8, EitherOfDiverse, Get,
        KeyOwnerProofSystem, VariantCountOf,
    },
    weights::{
        constants::{RocksDbWeight, WEIGHT_REF_TIME_PER_SECOND},
        IdentityFee, Weight,
    },
};
use frame_system::{
    limits::{BlockLength, BlockWeights},
    EnsureRoot, EnsureSigned,
};
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
    AccountId, Balance, Balances, Block, BlockNumber, Council, GhostConsensus, GhostPqc, Hash,
    Historical, ImOnline, Nonce, Offences, OriginCaller, PalletInfo, Preimage, Runtime,
    RuntimeCall, RuntimeEvent, RuntimeFreezeReason, RuntimeHoldReason, RuntimeOrigin, RuntimeTask,
    Scheduler, SessionKeys, Signature, System, DAYS, EXISTENTIAL_DEPOSIT, HOURS, MILLI_UNIT,
    SLOT_DURATION, UNIT, VERSION,
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
type SingleBlockMigrations = (
    // v1 -> v2 Ghost consensus pallet storage cleanup. Unwired, the legacy
    // u64 `Difficulty` bytes would be read as `U256` post-upgrade (see
    // docs/security-review/round-1.md).
    pallet_ghost_consensus::migrations::MigrateToV2<Runtime>,
);

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

/// Benchmark helper for `pallet-ghost-consensus`: `validate` requires
/// `SessionKeysLookup::keys_registered(who)`, which the real extrinsic
/// satisfies via `pallet_session::set_keys`. The benchmark pre-seeds
/// `NextKeys` with an all-zeroes decode (the same trick
/// `pallet-session`'s own benchmarks use — `SessionKeys::generate` needs a
/// keystore that doesn't exist under externalities).
#[cfg(feature = "runtime-benchmarks")]
pub struct GhostBenchmarkHelper;
#[cfg(feature = "runtime-benchmarks")]
impl pallet_ghost_consensus::BenchmarkHelper<Runtime> for GhostBenchmarkHelper {
    fn prepare_validate(who: &AccountId) {
        let keys = <SessionKeys as codec::Decode>::decode(
            &mut sp_runtime::traits::TrailingZeroInput::new(&[]),
        )
        .expect("zeroed SessionKeys always decode");
        pallet_session::NextKeys::<Runtime>::insert(who, keys);
        // `validate` also requires a registered PQC key (`RequirePqcKey`):
        // seed PqcKeys directly — a full ML-DSA registration would only
        // measure verify, not the gate we actually want worst-case for. The
        // gate only checks map presence, so key bytes don't matter.
        pallet_ghost_pqc::PqcKeys::<Runtime>::insert(
            who,
            pallet_ghost_pqc::PqcPublicKey::default(),
        );
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
    /// PoW authors are decoded from the `PreRuntime(pow_, AccountId)` digest
    /// by pallet-ghost-consensus; there is no session-indexed author map.
    type FindAuthor = pallet_ghost_consensus::PowFindAuthor<AccountId>;
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

impl<C> frame_system::offchain::CreateTransactionBase<C> for Runtime
where
    RuntimeCall: From<C>,
{
    type Extrinsic = super::UncheckedExtrinsic;
    type RuntimeCall = RuntimeCall;
}

// 2412 splits the old SendTransactionTypes into CreateTransactionBase +
// CreateInherent (bare extrinsics for pallet inherents — im-online
// heartbeats and GRANDPA equivocation reports are submitted unsigned).
impl<C> frame_system::offchain::CreateInherent<C> for Runtime
where
    RuntimeCall: From<C>,
{
    fn create_inherent(call: Self::RuntimeCall) -> Self::Extrinsic {
        super::UncheckedExtrinsic::new_bare(call)
    }
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
    /// 1ms — the minimum that keeps "time advances" true. PoW has no slot to
    /// protect: a larger floor ratchets `Now` above wall time whenever block
    /// production outpaces it, pinning `Now` at the +30s drift cap and
    /// getting honest blocks born "in the future" rejected at import (soak
    /// finding, docs/soak-report.md).
    type MinimumPeriod = ConstU64<1>;
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
    /// No post-slash callback — Ghost slashing goes through OnOffenceHandler.
    type DoneSlashHandler = ();
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
    type WeightInfo = pallet_transaction_payment::weights::SubstrateWeight<Runtime>;
}

impl pallet_sudo::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type RuntimeCall = RuntimeCall;
    type WeightInfo = pallet_sudo::weights::SubstrateWeight<Runtime>;
}

/// Configure the Ghost Consensus pallet (v2): staking holds, session
/// selection, difficulty retarget, digest-decoded rewards, offence slashing.
impl pallet_ghost_consensus::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type Currency = Balances;
    type RuntimeHoldReason = RuntimeHoldReason;
    type WeightInfo = pallet_ghost_consensus::weights::SubstrateWeight<Runtime>;
    #[cfg(feature = "runtime-benchmarks")]
    type BenchmarkHelper = GhostBenchmarkHelper;
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
    /// The GhostPqc pallet's registry, reached through the consensus pallet's
    /// provider trait (adapter below — the two pallets define identically
    /// shaped traits deliberately to stay decoupled).
    type PqcProvider = PqcKeyAdapter;
    /// Validators must hold a registered ML-DSA-87 key to `validate()`.
    /// Genesis stakers are seeded directly and unaffected.
    type RequirePqcKey = ConstBool<true>;
}

/// Adapt `pallet_ghost_pqc::PqcKeyProvider` onto
/// `pallet_ghost_consensus::PqcKeyProvider` — same contract, separate traits.
pub struct PqcKeyAdapter;
impl pallet_ghost_consensus::PqcKeyProvider<AccountId> for PqcKeyAdapter {
    fn has_pqc_key(who: &AccountId) -> bool {
        <GhostPqc as pallet_ghost_pqc::PqcKeyProvider<AccountId>>::has_pqc_key(who)
    }
    fn pqc_key(
        who: &AccountId,
    ) -> Option<BoundedVec<u8, ConstU32<{ pallet_ghost_consensus::MAX_PQC_KEY_SIZE }>>> {
        <GhostPqc as pallet_ghost_pqc::PqcKeyProvider<AccountId>>::pqc_key(who)
    }
}

/// Bonded-validator lookup for `pqc_attest` gating: at least `MinStake`
/// bonded in the consensus pallet (queried from its Config so the two
/// never drift).
pub struct BondedValidatorAdapter;
impl pallet_ghost_pqc::BondedValidatorProvider<AccountId> for BondedValidatorAdapter {
    fn is_bonded_validator(who: &AccountId) -> bool {
        GhostConsensus::bonded(who) >= <Runtime as pallet_ghost_consensus::Config>::MinStake::get()
    }
}

/// Benchmark seed for the `pqc_attest` bond gate: give `who` `MinStake` in
/// the consensus pallet's `Bonded` map (the storage
/// `BondedValidatorAdapter` actually reads).
#[cfg(feature = "runtime-benchmarks")]
pub struct GhostPqcBenchmarkHelper;
#[cfg(feature = "runtime-benchmarks")]
impl pallet_ghost_pqc::PqcBenchmarkHelper<AccountId> for GhostPqcBenchmarkHelper {
    fn seed_bonded_validator(who: &AccountId) {
        let stake: Balance = <Runtime as pallet_ghost_consensus::Config>::MinStake::get();
        pallet_ghost_consensus::Bonded::<Runtime>::insert(who, stake);
    }
}

impl pallet_ghost_pqc::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_ghost_pqc::weights::SubstrateWeight<Runtime>;
    type BondedValidators = BondedValidatorAdapter;
    #[cfg(feature = "runtime-benchmarks")]
    type BenchmarkHelper = GhostPqcBenchmarkHelper;
}

// ---------------------------------------------------------------------------
// Governance: council (collective + membership) + democracy + scheduler +
// preimage + utility.
//
// Shape: public proposals/referenda are the sovereign path (token holders
// decide, enactment via Scheduler). The Council is a small technocratic body
// seeded at genesis from the validator committee; it can table external
// proposals (majority), fast-track (2/3), cancel (2/3), and veto
// (single member). Instant origin is disabled — nothing executes without a
// vote or root.
// ---------------------------------------------------------------------------

parameter_types! {
    pub const CouncilMotionDuration: BlockNumber = 3 * DAYS;
    pub const CouncilMaxProposals: u32 = 100;
    pub const CouncilMaxMembers: u32 = 100;
    /// A council motion may not consume more than half a block on dispatch.
    pub MaxProposalWeight: Weight = Perbill::from_percent(50) * RuntimeBlockWeights::get().max_block;
}

pub type CouncilCollective = pallet_collective::Instance1;
impl pallet_collective::Config<CouncilCollective> for Runtime {
    type RuntimeOrigin = RuntimeOrigin;
    type Proposal = RuntimeCall;
    type RuntimeEvent = RuntimeEvent;
    type MotionDuration = CouncilMotionDuration;
    type MaxProposals = CouncilMaxProposals;
    type MaxMembers = CouncilMaxMembers;
    type DefaultVote = pallet_collective::PrimeDefaultVote;
    type WeightInfo = pallet_collective::weights::SubstrateWeight<Runtime>;
    type SetMembersOrigin = EnsureRoot<AccountId>;
    type MaxProposalWeight = MaxProposalWeight;
    /// Root can disapprove/kill motions — emergency brake only; council
    /// motions otherwise live and die by votes. No proposal deposit.
    type DisapproveOrigin = EnsureRoot<AccountId>;
    type KillOrigin = EnsureRoot<AccountId>;
    type Consideration = ();
}

pub type CouncilMembershipInstance = pallet_membership::Instance1;
impl pallet_membership::Config<CouncilMembershipInstance> for Runtime {
    type RuntimeEvent = RuntimeEvent;
    // Membership changes are governance-level decisions: root (democracy
    // root, once sudo is removed) or root during the sudo era.
    type AddOrigin = EnsureRoot<AccountId>;
    type RemoveOrigin = EnsureRoot<AccountId>;
    type SwapOrigin = EnsureRoot<AccountId>;
    type ResetOrigin = EnsureRoot<AccountId>;
    type PrimeOrigin = EnsureRoot<AccountId>;
    type MembershipInitialized = Council;
    type MembershipChanged = Council;
    type MaxMembers = CouncilMaxMembers;
    type WeightInfo = pallet_membership::weights::SubstrateWeight<Runtime>;
}

impl pallet_utility::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type RuntimeCall = RuntimeCall;
    type PalletsOrigin = OriginCaller;
    type WeightInfo = pallet_utility::weights::SubstrateWeight<Runtime>;
}

parameter_types! {
    /// A scheduled (incl. referendum-enacted) call may use up to 80% of a block.
    pub MaximumSchedulerWeight: Weight = Perbill::from_percent(80) * RuntimeBlockWeights::get().max_block;
    pub const MaxScheduledPerBlock: u32 = 50;
}

impl pallet_scheduler::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type RuntimeOrigin = RuntimeOrigin;
    type PalletsOrigin = OriginCaller;
    type RuntimeCall = RuntimeCall;
    type MaximumWeight = MaximumSchedulerWeight;
    type ScheduleOrigin = EnsureRoot<AccountId>;
    type OriginPrivilegeCmp = frame_support::traits::EqualPrivilegeOnly;
    type MaxScheduledPerBlock = MaxScheduledPerBlock;
    type WeightInfo = pallet_scheduler::weights::SubstrateWeight<Runtime>;
    type Preimages = Preimage;
}

parameter_types! {
    /// Hold reason token used by `HoldConsideration` for preimage deposits.
    pub const PreimageHoldReason: RuntimeHoldReason =
        RuntimeHoldReason::Preimage(pallet_preimage::HoldReason::Preimage);
    pub const PreimageBaseDeposit: Balance = 10 * UNIT;
    pub const PreimageByteDeposit: Balance = MILLI_UNIT;
}

impl pallet_preimage::Config for Runtime {
    type WeightInfo = pallet_preimage::weights::SubstrateWeight<Runtime>;
    type RuntimeEvent = RuntimeEvent;
    type Currency = Balances;
    type ManagerOrigin = EnsureRoot<AccountId>;
    type Consideration = frame_support::traits::fungible::HoldConsideration<
        AccountId,
        Balances,
        PreimageHoldReason,
        frame_support::traits::LinearStoragePrice<
            PreimageBaseDeposit,
            PreimageByteDeposit,
            Balance,
        >,
    >;
}

parameter_types! {
    // Periods sized for a 5s-block testnet but sane for mainnet governance:
    // ~1 day proposal table, ~5 day vote, ~1 day enactment.
    pub const LaunchPeriod: BlockNumber = DAYS;
    pub const VotingPeriod: BlockNumber = 5 * DAYS;
    pub const FastTrackVotingPeriod: BlockNumber = 3 * HOURS;
    pub const MinimumDeposit: Balance = 10 * UNIT;
    pub const EnactmentPeriod: BlockNumber = DAYS;
    pub const VoteLockingPeriod: BlockNumber = DAYS;
    pub const CooloffPeriod: BlockNumber = 7 * DAYS;
    pub const MaxVotesPerAccount: u32 = 100;
    pub const MaxProposalsPerPublic: u32 = 100;
    pub const MaxDepositsPerAccount: u32 = 100;
    pub const MaxBlacklistedProposals: u32 = 100;
}

impl pallet_democracy::Config for Runtime {
    type WeightInfo = pallet_democracy::weights::SubstrateWeight<Runtime>;
    type RuntimeEvent = RuntimeEvent;
    type Scheduler = Scheduler;
    type Preimages = Preimage;
    type Currency = Balances;
    type EnactmentPeriod = EnactmentPeriod;
    type LaunchPeriod = LaunchPeriod;
    type VotingPeriod = VotingPeriod;
    type VoteLockingPeriod = VoteLockingPeriod;
    type MinimumDeposit = MinimumDeposit;
    // Instant enactment is allowed ONLY through `InstantOrigin` — a unanimous
    // council (EnsureProportionAtLeast 1/1). It's the emergency fast path:
    // on a real committee unanimity is a high bar, and it also lets the
    // governance e2e drill run a full referendum in gate time.
    type InstantAllowed = ConstBool<true>;
    type FastTrackVotingPeriod = FastTrackVotingPeriod;
    type CooloffPeriod = CooloffPeriod;
    type MaxVotes = MaxVotesPerAccount;
    type MaxProposals = MaxProposalsPerPublic;
    type MaxDeposits = MaxDepositsPerAccount;
    type MaxBlacklisted = MaxBlacklistedProposals;
    type SubmitOrigin = EnsureSigned<AccountId>;
    // External proposals come from the Council: a plain majority tables one,
    // a 3/5 majority passes by default-majority, and it fast-tracks at 2/3.
    // (`externalPropose` tables SuperMajorityApprove, which fast_track
    // refuses by design — drills should use externalProposeMajority.)
    type ExternalOrigin =
        pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 1, 2>;
    type ExternalMajorityOrigin =
        pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 1, 2>;
    type ExternalDefaultOrigin =
        pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 3, 5>;
    type FastTrackOrigin =
        pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 2, 3>;
    type InstantOrigin =
        pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 1, 1>;
    // Referendum cancellation: root, or a 2/3 council vote.
    type CancellationOrigin = EitherOfDiverse<
        EnsureRoot<AccountId>,
        pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 2, 3>,
    >;
    // Blacklisting a (hash, index) pair is a root-level emergency control.
    type BlacklistOrigin = EnsureRoot<AccountId>;
    // Proposal (not referendum) cancellation: root or council majority.
    type CancelProposalOrigin = EitherOfDiverse<
        EnsureRoot<AccountId>,
        pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 1, 2>,
    >;
    // Any single council member may veto an external proposal once per
    // cooloff — a circuit breaker against council capture.
    type VetoOrigin = pallet_collective::EnsureMember<AccountId, CouncilCollective>;
    type PalletsOrigin = OriginCaller;
    // Slashed referendum deposits are burned: Ghost routes block rewards
    // through the consensus pallet, and there is no treasury yet. Burning is
    // the conservative default (deflationary, no discretionary pot).
    type Slash = ();
}
