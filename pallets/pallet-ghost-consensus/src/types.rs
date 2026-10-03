//! Types for the Ghost Consensus Pallet.
//!
//! Everything here must be `no_std`-compatible: these types live in storage and
//! are SCALE-encoded into the chain state.

use codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;

/// A chunk of bonded stake that is unbonding.
///
/// Funds stay on hold until `unlock_at`, after which `withdraw_unbonded` can
/// release them back to the free balance.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
#[scale_info(skip_type_params(BlockNumber))]
pub struct UnbondingChunk<Balance, BlockNumber> {
    /// Amount of stake being unbonded.
    pub amount: Balance,
    /// Block number at which this chunk becomes withdrawable.
    pub unlock_at: BlockNumber,
}

/// A record of a slashing applied to a bonded account.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TypeInfo, MaxEncodedLen)]
#[scale_info(skip_type_params(T))]
pub struct SlashRecord<AccountId, Balance, BlockNumber> {
    /// Account that was slashed.
    pub who: AccountId,
    /// Amount of bonded stake burned.
    pub amount: Balance,
    /// Session index in which the offence occurred.
    pub session_index: u32,
    /// Block number at which the slash was applied.
    pub block_number: BlockNumber,
}
