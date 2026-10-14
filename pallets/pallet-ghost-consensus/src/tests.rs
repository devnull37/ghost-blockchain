//! Tests for the Ghost consensus pallet v2: staking holds, unbonding queue,
//! session validator selection, difficulty retarget (work-factor semantics),
//! digest-decoded rewards, offence slashing, and the v1->v2 migration.

use super::*;
use crate::migrations::{MigrateToV2, STORAGE_VERSION};
use crate::mock::*;
use frame_support::{
    assert_noop, assert_ok,
    storage::storage_prefix,
    traits::{fungible::InspectHold, GetStorageVersion, Hooks, OnRuntimeUpgrade, StorageVersion},
};
use sp_core::U256;
use sp_runtime::Perbill;
use sp_staking::offence::{OffenceDetails, OnOffenceHandler};

type HoldReasonStaking = crate::HoldReason;

fn on_hold(who: AccountId) -> Balance {
    <Balances as InspectHold<AccountId>>::balance_on_hold(&HoldReasonStaking::Staking.into(), &who)
}

fn slash(offenders: &[(AccountId, Balance)], fractions: &[Perbill], session: SessionIndex) {
    let details: Vec<OffenceDetails<AccountId, (AccountId, Balance)>> = offenders
        .iter()
        .map(|o| OffenceDetails {
            offender: *o,
            reporters: vec![],
        })
        .collect();
    <GhostConsensus as OnOffenceHandler<AccountId, (AccountId, Balance), Weight>>::on_offence(
        &details, fractions, session,
    );
}

// ---------------------------------------------------------------------------
// Staking
// ---------------------------------------------------------------------------

#[test]
fn bond_below_min_fails() {
    new_test_ext().execute_with(|| {
        assert_noop!(
            GhostConsensus::bond(RuntimeOrigin::signed(ALICE), MIN_STAKE - 1),
            Error::<Test>::BondBelowMinimum
        );
    });
}

#[test]
fn bond_holds_funds_and_stays_spendable_on_account() {
    new_test_ext().execute_with(|| {
        assert_ok!(GhostConsensus::bond(RuntimeOrigin::signed(ALICE), 100));
        assert_eq!(GhostConsensus::bonded(&ALICE), 100);
        assert_eq!(on_hold(ALICE), 100);
        // Held funds stay on the account (no transfer); free balance reports
        // the un-held remainder.
        assert_eq!(Balances::free_balance(ALICE), 10_000 - 100);
        System::assert_last_event(
            Event::Bonded {
                who: ALICE,
                added: 100,
                total: 100,
            }
            .into(),
        );
    });
}

#[test]
fn bond_extra_stacks_and_respects_max() {
    new_test_ext().execute_with(|| {
        assert_ok!(GhostConsensus::bond(RuntimeOrigin::signed(ALICE), 100));
        assert_ok!(GhostConsensus::bond_extra(RuntimeOrigin::signed(ALICE), 50));
        assert_eq!(GhostConsensus::bonded(&ALICE), 150);
        assert_eq!(on_hold(ALICE), 150);

        // bond_extra on a fresh account fails.
        assert_noop!(
            GhostConsensus::bond_extra(RuntimeOrigin::signed(BOB), 100),
            Error::<Test>::NotBonded
        );
    });
}

#[test]
fn bond_above_max_fails() {
    new_test_ext().execute_with(|| {
        assert_noop!(
            GhostConsensus::bond(RuntimeOrigin::signed(ALICE), MAX_STAKE + 1),
            Error::<Test>::BondAboveMaximum
        );
        assert_ok!(GhostConsensus::bond(RuntimeOrigin::signed(ALICE), 100));
        assert_noop!(
            GhostConsensus::bond_extra(RuntimeOrigin::signed(ALICE), MAX_STAKE),
            Error::<Test>::BondAboveMaximum
        );
    });
}

#[test]
fn unbond_queues_chunk_and_withdraw_releases_after_period() {
    new_test_ext().execute_with(|| {
        assert_ok!(GhostConsensus::bond(RuntimeOrigin::signed(ALICE), 200));
        assert_ok!(GhostConsensus::unbond(RuntimeOrigin::signed(ALICE), 50));

        assert_eq!(GhostConsensus::bonded(&ALICE), 150);
        assert_eq!(on_hold(ALICE), 200); // still held while unbonding
        let chunks = crate::Unbonding::<Test>::get(ALICE);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].amount, 50);
        assert_eq!(chunks[0].unlock_at, 1 + UNBONDING_PERIOD);

        // Too early -> NothingToWithdraw.
        assert_noop!(
            GhostConsensus::withdraw_unbonded(RuntimeOrigin::signed(ALICE)),
            Error::<Test>::NothingToWithdraw
        );

        // After the unbonding period, withdraw releases the hold.
        System::set_block_number(1 + UNBONDING_PERIOD);
        assert_ok!(GhostConsensus::withdraw_unbonded(RuntimeOrigin::signed(
            ALICE
        )));
        assert_eq!(on_hold(ALICE), 150);
        assert!(crate::Unbonding::<Test>::get(ALICE).is_empty());
        System::assert_last_event(
            Event::WithdrawnUnbonded {
                who: ALICE,
                amount: 50,
            }
            .into(),
        );
    });
}

#[test]
fn unbond_full_amount_removes_bonded_entry() {
    new_test_ext().execute_with(|| {
        assert_ok!(GhostConsensus::bond(RuntimeOrigin::signed(ALICE), 100));
        assert_ok!(GhostConsensus::unbond(RuntimeOrigin::signed(ALICE), 100));
        assert_eq!(GhostConsensus::bonded(&ALICE), 0);
        assert_noop!(
            GhostConsensus::unbond(RuntimeOrigin::signed(ALICE), 1),
            Error::<Test>::NotBonded
        );
    });
}

#[test]
fn unbonding_queue_is_bounded() {
    new_test_ext().execute_with(|| {
        assert_ok!(GhostConsensus::bond(RuntimeOrigin::signed(ALICE), 400));
        for _ in 0..4 {
            assert_ok!(GhostConsensus::unbond(RuntimeOrigin::signed(ALICE), 10));
        }
        assert_noop!(
            GhostConsensus::unbond(RuntimeOrigin::signed(ALICE), 10),
            Error::<Test>::TooManyUnbondingChunks
        );
        // Withdrawing matured chunks frees queue slots.
        System::set_block_number(1 + UNBONDING_PERIOD);
        assert_ok!(GhostConsensus::withdraw_unbonded(RuntimeOrigin::signed(
            ALICE
        )));
        assert_ok!(GhostConsensus::unbond(RuntimeOrigin::signed(ALICE), 10));
    });
}

// ---------------------------------------------------------------------------
// Candidates / validate / chill
// ---------------------------------------------------------------------------

#[test]
fn validate_requires_min_stake_keys_and_pqc() {
    new_test_ext().execute_with(|| {
        // Bonded but below the minimum after a partial unbond.
        assert_ok!(GhostConsensus::bond(
            RuntimeOrigin::signed(ALICE),
            MIN_STAKE
        ));
        assert_ok!(GhostConsensus::unbond(RuntimeOrigin::signed(ALICE), 1));
        assert_eq!(GhostConsensus::bonded(&ALICE), MIN_STAKE - 1);
        set_keys_registered(ALICE, true);
        assert_noop!(
            GhostConsensus::validate(RuntimeOrigin::signed(ALICE)),
            Error::<Test>::BondBelowMinimum
        );

        // Enough stake but no session keys.
        System::set_block_number(1 + UNBONDING_PERIOD);
        assert_ok!(GhostConsensus::withdraw_unbonded(RuntimeOrigin::signed(
            ALICE
        )));
        assert_ok!(GhostConsensus::bond_extra(RuntimeOrigin::signed(ALICE), 1));
        assert_eq!(GhostConsensus::bonded(&ALICE), MIN_STAKE);
        set_keys_registered(ALICE, false);
        assert_noop!(
            GhostConsensus::validate(RuntimeOrigin::signed(ALICE)),
            Error::<Test>::KeysNotRegistered
        );

        // PQC gate enforced when RequirePqcKey is on.
        set_keys_registered(ALICE, true);
        set_require_pqc(true);
        assert_noop!(
            GhostConsensus::validate(RuntimeOrigin::signed(ALICE)),
            Error::<Test>::PqcKeyRequired
        );

        set_pqc_key(ALICE, true);
        assert_ok!(GhostConsensus::validate(RuntimeOrigin::signed(ALICE)));
        assert!(crate::Candidates::<Test>::get().contains(&ALICE));

        assert_noop!(
            GhostConsensus::validate(RuntimeOrigin::signed(ALICE)),
            Error::<Test>::AlreadyCandidate
        );
    });
}

#[test]
fn chill_removes_candidate() {
    new_test_ext().execute_with(|| {
        assert_ok!(GhostConsensus::bond(RuntimeOrigin::signed(ALICE), 100));
        set_keys_registered(ALICE, true);
        assert_ok!(GhostConsensus::validate(RuntimeOrigin::signed(ALICE)));
        assert_ok!(GhostConsensus::chill(RuntimeOrigin::signed(ALICE)));
        assert!(!crate::Candidates::<Test>::get().contains(&ALICE));
        assert_noop!(
            GhostConsensus::chill(RuntimeOrigin::signed(ALICE)),
            Error::<Test>::NotCandidate
        );
    });
}

// ---------------------------------------------------------------------------
// SessionManager
// ---------------------------------------------------------------------------

fn bond_and_validate(who: AccountId, amount: Balance) {
    assert_ok!(GhostConsensus::bond(RuntimeOrigin::signed(who), amount));
    set_keys_registered(who, true);
    assert_ok!(GhostConsensus::validate(RuntimeOrigin::signed(who)));
}

#[test]
fn new_session_selects_top_n_by_stake_with_tiebreak() {
    new_test_ext().execute_with(|| {
        bond_and_validate(ALICE, 100);
        bond_and_validate(BOB, 300);
        bond_and_validate(CHARLIE, 100); // ties ALICE; lower id wins
        bond_and_validate(DAVE, 200);
        bond_and_validate(EVE, 50); // below the rest, drops off

        let set =
            <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session_genesis(0)
                .unwrap();
        // MaxValidators = 3; stakes 300/200/100 tie-broken ALICE < CHARLIE.
        assert_eq!(set, vec![BOB, DAVE, ALICE]);
    });
}

#[test]
fn new_session_genesis_falls_back_to_initial_validators() {
    new_test_ext().execute_with(|| {
        crate::InitialValidators::<Test>::put(BoundedVec::<u64, ConstU32<3>>::truncate_from(vec![
            DAVE, EVE,
        ]));
        let set =
            <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session_genesis(0)
                .unwrap();
        assert_eq!(set, vec![DAVE, EVE]);
    });
}

#[test]
fn new_session_empty_selection_keeps_active_committee() {
    new_test_ext().execute_with(|| {
        bond_and_validate(ALICE, 100);
        bond_and_validate(BOB, 200);
        <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session_genesis(0)
            .unwrap();
        <GhostConsensus as pallet_session::SessionManager<AccountId>>::start_session(0);
        assert_eq!(
            crate::ActiveValidators::<Test>::get().into_inner(),
            vec![BOB, ALICE]
        );

        // Both candidates chill: selection would be empty, which would seat
        // an empty GRANDPA committee and stall finality forever (M-4).
        assert_ok!(GhostConsensus::chill(RuntimeOrigin::signed(ALICE)));
        assert_ok!(GhostConsensus::chill(RuntimeOrigin::signed(BOB)));
        assert!(crate::Candidates::<Test>::get().is_empty());

        // Instead of Some([]) the active committee is retained, so the set
        // equals the active one and `None` is returned (no gratuitous change).
        assert_eq!(
            <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session(1),
            None
        );
        assert_eq!(
            crate::PendingValidators::<Test>::get().into_inner(),
            vec![BOB, ALICE]
        );
    });
}

#[test]
fn new_session_empty_active_falls_back_to_initial_validators() {
    new_test_ext().execute_with(|| {
        // Pathological state: a past session seated an empty committee and no
        // candidates exist — the genesis-declared set is re-seated rather
        // than staying empty.
        crate::InitialValidators::<Test>::put(BoundedVec::<u64, ConstU32<3>>::truncate_from(vec![
            DAVE, EVE,
        ]));
        assert_eq!(
            <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session(1),
            Some(vec![DAVE, EVE])
        );
    });
}

#[test]
fn new_session_empty_everything_is_documented_degraded() {
    new_test_ext().execute_with(|| {
        // With no candidates, no active committee, and no genesis fallbacks
        // the chain is misconfigured; selection stays empty rather than
        // fabricating validators.
        assert_eq!(
            <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session(1),
            None
        );
        assert!(crate::PendingValidators::<Test>::get().is_empty());
    });
}

#[test]
fn session_lifecycle_tracks_active_validators() {
    new_test_ext().execute_with(|| {
        bond_and_validate(ALICE, 100);
        bond_and_validate(BOB, 200);

        // Genesis selection populates pending.
        let set =
            <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session_genesis(0)
                .unwrap();
        assert_eq!(set, vec![BOB, ALICE]);

        <GhostConsensus as pallet_session::SessionManager<AccountId>>::start_session(0);
        assert_eq!(
            crate::ActiveValidators::<Test>::get().into_inner(),
            vec![BOB, ALICE]
        );

        // Same set recomputed -> None (no gratuitous GRANDPA change).
        assert_eq!(
            <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session(1),
            None
        );

        // New candidate with bigger bond changes the set.
        bond_and_validate(CHARLIE, 400);
        assert_eq!(
            <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session(2),
            Some(vec![CHARLIE, BOB, ALICE])
        );
    });
}

#[test]
fn historical_session_manager_carries_bond() {
    new_test_ext().execute_with(|| {
        bond_and_validate(ALICE, 100);
        let set = <GhostConsensus as pallet_session::historical::SessionManager<
            AccountId,
            Balance,
        >>::new_session_genesis(0)
        .unwrap();
        assert_eq!(set, vec![(ALICE, 100)]);
    });
}

// ---------------------------------------------------------------------------
// Difficulty retarget (work factor: larger = harder)
// ---------------------------------------------------------------------------

#[test]
fn first_retarget_boundary_only_sets_baseline() {
    new_test_ext().execute_with(|| {
        assert_eq!(crate::Difficulty::<Test>::get(), U256::from(1_000u64));
        run_to_block(RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS);
        assert_eq!(crate::Difficulty::<Test>::get(), U256::from(1_000u64));
        assert_eq!(crate::RetargetsDone::<Test>::get(), 1);
        // 99 deltas elapsed when block 100 runs on_initialize.
        assert_eq!(
            crate::LastRetargetTime::<Test>::get(),
            99 * TARGET_BLOCK_TIME_MS
        );
    });
}

#[test]
fn blocks_too_fast_increase_difficulty() {
    new_test_ext().execute_with(|| {
        // First interval at target -> baseline only.
        run_to_block(RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS);
        // Second interval twice as fast -> difficulty doubles.
        run_to_block(2 * RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS / 2);
        assert_eq!(crate::Difficulty::<Test>::get(), U256::from(2_000u64));
    });
}

#[test]
fn blocks_too_slow_decrease_difficulty() {
    new_test_ext().execute_with(|| {
        run_to_block(RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS);
        // Twice as slow -> difficulty halves.
        run_to_block(2 * RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS * 2);
        assert_eq!(crate::Difficulty::<Test>::get(), U256::from(500u64));
    });
}

#[test]
fn retarget_clamps_at_factor_4() {
    new_test_ext().execute_with(|| {
        run_to_block(RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS);
        // 100x too fast -> clamped to 4x, not 100x.
        run_to_block(2 * RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS / 100);
        assert_eq!(crate::Difficulty::<Test>::get(), U256::from(4_000u64));
        // 100x too slow -> clamped to /4.
        run_to_block(3 * RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS * 100);
        assert_eq!(crate::Difficulty::<Test>::get(), U256::from(1_000u64));
    });
}

#[test]
fn retarget_floors_at_min_difficulty() {
    new_test_ext().execute_with(|| {
        crate::Difficulty::<Test>::put(U256::from(101u64)); // just above min 100
        run_to_block(RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS);
        // 5x too slow -> raw would be ~20, clamp /4 -> 25, floor -> 100.
        run_to_block(2 * RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS * 5);
        assert_eq!(crate::Difficulty::<Test>::get(), U256::from(100u64));
    });
}

#[test]
fn zero_elapsed_retargets_max_up() {
    new_test_ext().execute_with(|| {
        run_to_block(RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS);
        // No time elapsed between boundaries -> clamp max-up (4x).
        run_to_block(2 * RETARGET_INTERVAL, 0);
        assert_eq!(crate::Difficulty::<Test>::get(), U256::from(4_000u64));
    });
}

#[test]
fn next_difficulty_returns_work_factor() {
    new_test_ext().execute_with(|| {
        assert_eq!(GhostConsensus::next_difficulty(), U256::from(1_000u64));
    });
}

// ---------------------------------------------------------------------------
// Rewards
// ---------------------------------------------------------------------------

#[test]
fn reward_splits_40_author_60_validators_prorata() {
    new_test_ext().execute_with(|| {
        bond_and_validate(ALICE, 100);
        bond_and_validate(BOB, 300);
        // Activate the session set {BOB 300, ALICE 100}.
        <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session_genesis(0);
        <GhostConsensus as pallet_session::SessionManager<AccountId>>::start_session(0);

        // CHARLIE mines the block (not a validator).
        set_author(CHARLIE);
        <GhostConsensus as Hooks<u64>>::on_finalize(System::block_number());

        // 100 reward: author 40 + pro-rata dust; validators 60 => BOB 45, ALICE 15.
        assert_eq!(Balances::free_balance(CHARLIE), 10_000 + 40);
        // Validator balances = endowed + share - bonded hold.
        assert_eq!(Balances::free_balance(BOB), 10_000 + 45 - 300);
        assert_eq!(Balances::free_balance(ALICE), 10_000 + 15 - 100);
        assert!(crate::RecentAuthors::<Test>::get().contains(&CHARLIE));
    });
}

#[test]
fn reward_with_no_validators_goes_to_reserve() {
    new_test_ext().execute_with(|| {
        set_author(ALICE);
        <GhostConsensus as Hooks<u64>>::on_finalize(System::block_number());
        // 60% -> pallet reserve account; author still gets 40%.
        assert_eq!(Balances::free_balance(ALICE), 10_000 + 40);
        assert_eq!(Balances::free_balance(GhostConsensus::account_id()), 60);
    });
}

#[test]
fn reward_rounding_remainder_goes_to_author() {
    new_test_ext().execute_with(|| {
        // BlockReward=100 -> author 40, validators pot 60. Three equal-stake
        // validators get 20 each; if weights made it uneven the author would
        // pick up the floor-division dust.
        bond_and_validate(ALICE, MIN_STAKE);
        bond_and_validate(BOB, MIN_STAKE);
        bond_and_validate(CHARLIE, MIN_STAKE);
        <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session_genesis(0);
        <GhostConsensus as pallet_session::SessionManager<AccountId>>::start_session(0);

        let before: Balance = [ALICE, BOB, CHARLIE, DAVE]
            .iter()
            .map(Balances::free_balance)
            .sum();
        set_author(DAVE);
        <GhostConsensus as Hooks<u64>>::on_finalize(System::block_number());
        let after: Balance = [ALICE, BOB, CHARLIE, DAVE]
            .iter()
            .map(Balances::free_balance)
            .sum();
        // No dust lost: total minted == BlockReward, all to accounts.
        assert_eq!(after - before, BLOCK_REWARD);
    });
}

#[test]
fn reward_skips_when_no_pow_digest() {
    new_test_ext().execute_with(|| {
        <GhostConsensus as Hooks<u64>>::on_finalize(System::block_number());
        System::assert_last_event(
            Event::BlockRewardSkipped {
                block_number: System::block_number(),
            }
            .into(),
        );
        assert!(crate::RecentAuthors::<Test>::get().is_empty());
    });
}

#[test]
fn reward_skips_when_digest_malformed() {
    new_test_ext().execute_with(|| {
        // pow_ pre-runtime item whose payload does not decode to AccountId.
        System::deposit_log(sp_runtime::DigestItem::PreRuntime(
            POW_ENGINE_ID,
            vec![0xde, 0xad],
        ));
        <GhostConsensus as Hooks<u64>>::on_finalize(System::block_number());
        System::assert_last_event(
            Event::BlockRewardSkipped {
                block_number: System::block_number(),
            }
            .into(),
        );
    });
}

#[test]
fn reward_ignores_unrelated_preruntime_digests() {
    new_test_ext().execute_with(|| {
        System::deposit_log(sp_runtime::DigestItem::PreRuntime(*b"othr", ALICE.encode()));
        <GhostConsensus as Hooks<u64>>::on_finalize(System::block_number());
        System::assert_last_event(
            Event::BlockRewardSkipped {
                block_number: System::block_number(),
            }
            .into(),
        );
    });
}

// ---------------------------------------------------------------------------
// Slashing (OnOffenceHandler)
// ---------------------------------------------------------------------------

#[test]
fn on_offence_slashes_burns_chills_and_records() {
    new_test_ext().execute_with(|| {
        bond_and_validate(ALICE, 400);
        <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session_genesis(0);
        <GhostConsensus as pallet_session::SessionManager<AccountId>>::start_session(0);
        let issuance_before = Balances::total_issuance();

        slash(&[(ALICE, 400)], &[Perbill::from_percent(25)], 7);

        // 25% of 400 burned: hold reduced, issuance reduced, record appended.
        assert_eq!(GhostConsensus::bonded(&ALICE), 300);
        assert_eq!(on_hold(ALICE), 300);
        assert_eq!(Balances::total_issuance(), issuance_before - 100);
        assert!(!crate::Candidates::<Test>::get().contains(&ALICE));
        // Stays in ActiveValidators until the next session boundary: that
        // record mirrors the committee pallet_session still has seated.
        assert!(crate::ActiveValidators::<Test>::get().contains(&ALICE));

        let records = crate::SlashRecords::<Test>::get();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].who, ALICE);
        assert_eq!(records[0].amount, 100);
        assert_eq!(records[0].session_index, 7);
    });
}

#[test]
fn on_offence_skips_accounts_with_no_bond() {
    new_test_ext().execute_with(|| {
        slash(&[(EVE, 0)], &[Perbill::from_percent(50)], 1);
        assert!(crate::SlashRecords::<Test>::get().is_empty());
    });
}

#[test]
fn on_offence_applies_fraction_per_offender() {
    new_test_ext().execute_with(|| {
        bond_and_validate(ALICE, 100);
        bond_and_validate(BOB, 200);
        slash(
            &[(ALICE, 100), (BOB, 200)],
            &[Perbill::from_percent(10), Perbill::from_percent(50)],
            3,
        );
        assert_eq!(GhostConsensus::bonded(&ALICE), 90);
        assert_eq!(GhostConsensus::bonded(&BOB), 100);
        assert_eq!(crate::SlashRecords::<Test>::get().len(), 2);
    });
}

#[test]
fn on_offence_slashes_unbonding_chunks_too() {
    new_test_ext().execute_with(|| {
        // Regression test for round-1 finding: `unbond` must not grant slash
        // immunity. Chunk funds stay held, so they stay slashable.
        assert_ok!(GhostConsensus::bond(RuntimeOrigin::signed(ALICE), 400));
        assert_ok!(GhostConsensus::unbond(RuntimeOrigin::signed(ALICE), 200));
        assert_eq!(on_hold(ALICE), 400); // 200 bonded + 200 unbonding

        let issuance_before = Balances::total_issuance();
        slash(&[(ALICE, 400)], &[Perbill::from_percent(50)], 3);

        // 50% of the bonded half (100) plus 50% of the chunk (100) burned.
        assert_eq!(GhostConsensus::bonded(&ALICE), 100);
        let chunks = crate::Unbonding::<Test>::get(ALICE);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].amount, 100);
        assert_eq!(on_hold(ALICE), 200);
        assert_eq!(Balances::total_issuance(), issuance_before - 200);

        // After maturity only the slashed-down chunk releases.
        System::set_block_number(1 + UNBONDING_PERIOD);
        assert_ok!(GhostConsensus::withdraw_unbonded(RuntimeOrigin::signed(
            ALICE
        )));
        assert_eq!(on_hold(ALICE), 100);
        // 10_000 endowed - 100 still held (bonded) - 200 burned by the slash.
        assert_eq!(Balances::free_balance(ALICE), 10_000 - 100 - 200);
    });
}

#[test]
fn on_offence_full_unbond_does_not_escape_chill_or_slash() {
    new_test_ext().execute_with(|| {
        // Worst case of the same bug: unbond *everything* used to zero the
        // slash and even skip the chill via the early `continue`.
        bond_and_validate(ALICE, 400);
        <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session_genesis(0);
        <GhostConsensus as pallet_session::SessionManager<AccountId>>::start_session(0);
        assert_ok!(GhostConsensus::unbond(RuntimeOrigin::signed(ALICE), 400));
        assert_eq!(GhostConsensus::bonded(&ALICE), 0);
        assert_eq!(on_hold(ALICE), 400);

        slash(&[(ALICE, 400)], &[Perbill::from_percent(25)], 3);

        // 25% of the unbonding chunk is burned and the offender is chilled.
        assert_eq!(on_hold(ALICE), 300);
        assert!(!crate::Candidates::<Test>::get().contains(&ALICE));
        // Removal from the seated committee happens at the session boundary,
        // not mid-session (pallet_session still has them voting).
        assert!(crate::ActiveValidators::<Test>::get().contains(&ALICE));
        let records = crate::SlashRecords::<Test>::get();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].amount, 100);
    });
}

#[test]
fn on_offence_drains_all_chunks_at_full_slash() {
    new_test_ext().execute_with(|| {
        assert_ok!(GhostConsensus::bond(RuntimeOrigin::signed(ALICE), 400));
        for _ in 0..4 {
            assert_ok!(GhostConsensus::unbond(RuntimeOrigin::signed(ALICE), 100));
        }
        assert_eq!(GhostConsensus::bonded(&ALICE), 0);
        assert_eq!(on_hold(ALICE), 400);

        slash(&[(ALICE, 400)], &[Perbill::from_percent(100)], 3);

        assert_eq!(on_hold(ALICE), 0);
        assert!(crate::Unbonding::<Test>::get(ALICE).is_empty());
        assert_eq!(crate::SlashRecords::<Test>::get()[0].amount, 400);
    });
}

// ---------------------------------------------------------------------------
// PowFindAuthor (authorship -> im-online liveness)
// ---------------------------------------------------------------------------

#[test]
fn pow_find_author_decodes_pow_preruntime() {
    new_test_ext().execute_with(|| {
        let alice_bytes = ALICE.encode();
        let items: Vec<(sp_runtime::ConsensusEngineId, &[u8])> =
            vec![(POW_ENGINE_ID, alice_bytes.as_slice())];
        assert_eq!(
            <crate::PowFindAuthor<AccountId> as frame_support::traits::FindAuthor<
                AccountId,
            >>::find_author(items),
            Some(ALICE)
        );
    });
}

#[test]
fn pow_find_author_ignores_non_pow_and_undecodable() {
    new_test_ext().execute_with(|| {
        let alice_bytes = ALICE.encode();
        // Wrong engine id.
        let items: Vec<(sp_runtime::ConsensusEngineId, &[u8])> =
            vec![(*b"aura", alice_bytes.as_slice())];
        assert_eq!(
            <crate::PowFindAuthor<AccountId> as frame_support::traits::FindAuthor<
                AccountId,
            >>::find_author(items),
            None
        );
        // Undecodable payload.
        let bad: &[u8] = &[0xde, 0xad];
        let items: Vec<(sp_runtime::ConsensusEngineId, &[u8])> = vec![(POW_ENGINE_ID, bad)];
        assert_eq!(
            <crate::PowFindAuthor<AccountId> as frame_support::traits::FindAuthor<
                AccountId,
            >>::find_author(items),
            None
        );
        // Empty input.
        assert_eq!(
            <crate::PowFindAuthor<AccountId> as frame_support::traits::FindAuthor<
                AccountId,
            >>::find_author(core::iter::empty()),
            None
        );
        // First undecodable pow_ item, second decodable: takes the decodable
        // one (same find_map semantics as `block_author`).
        let bad: &[u8] = &[0xde, 0xad];
        let items: Vec<(sp_runtime::ConsensusEngineId, &[u8])> = vec![
            (POW_ENGINE_ID, bad),
            (POW_ENGINE_ID, alice_bytes.as_slice()),
        ];
        assert_eq!(
            <crate::PowFindAuthor<AccountId> as frame_support::traits::FindAuthor<
                AccountId,
            >>::find_author(items),
            Some(ALICE)
        );
    });
}

// ---------------------------------------------------------------------------
// Retarget boundary pinning
// ---------------------------------------------------------------------------

#[test]
fn retarget_exact_clamp_boundaries() {
    new_test_ext().execute_with(|| {
        run_to_block(RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS); // baseline only

        // elapsed == expected * 4 exactly: lands on the /4 boundary.
        run_to_block(2 * RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS * 4);
        assert_eq!(crate::Difficulty::<Test>::get(), U256::from(250u64));

        // elapsed * 4 == expected exactly: lands on the *4 boundary.
        crate::Difficulty::<Test>::put(U256::from(1_000u64));
        run_to_block(3 * RETARGET_INTERVAL, TARGET_BLOCK_TIME_MS / 4);
        assert_eq!(crate::Difficulty::<Test>::get(), U256::from(4_000u64));
    });
}

// ---------------------------------------------------------------------------
// Migration v1 -> v2
// ---------------------------------------------------------------------------

#[test]
fn migration_clears_all_legacy_storage() {
    new_test_ext().execute_with(|| {
        let pallet_prefix =
            <GhostConsensus as frame_support::traits::PalletInfoAccess>::name().as_bytes();

        // Populate every legacy v1 item with junk bytes.
        for item in [
            b"Difficulty".as_slice(),
            b"CurrentPhase".as_slice(),
            b"SlashingRecords".as_slice(),
            b"RecentBlockProducers".as_slice(),
            b"CurrentEntropy".as_slice(),
        ] {
            sp_io::storage::set(&storage_prefix(pallet_prefix, item), &[0xaa; 32]);
        }
        for item in [
            b"BlockHeaders".as_slice(),
            b"BlockProducers".as_slice(),
            b"ValidatorStakes".as_slice(),
            b"LastActiveBlock".as_slice(),
            b"DoubleSignReports".as_slice(),
            b"InvalidBlockReports".as_slice(),
            b"ValidatorPqcPublicKeys".as_slice(),
        ] {
            let prefix = storage_prefix(pallet_prefix, item);
            sp_io::storage::set(
                &[
                    &prefix[..],
                    &sp_io::hashing::blake2_128(&[1u8])[..],
                    &[1u8][..],
                ]
                .concat(),
                &[0xbb; 8],
            );
        }

        // Pretend chain is on v1.
        StorageVersion::new(1).put::<GhostConsensus>();
        assert_eq!(GhostConsensus::on_chain_storage_version(), 1);

        MigrateToV2::<Test>::on_runtime_upgrade();

        assert_eq!(GhostConsensus::on_chain_storage_version(), STORAGE_VERSION);
        // Values cleared.
        for item in [
            b"Difficulty".as_slice(),
            b"CurrentPhase".as_slice(),
            b"SlashingRecords".as_slice(),
            b"RecentBlockProducers".as_slice(),
            b"CurrentEntropy".as_slice(),
        ] {
            // "Difficulty" was repopulated by the migration seed; check it
            // reads as a valid v2 U256 work factor, not v1 junk.
            if item == b"Difficulty".as_slice() {
                continue;
            }
            assert_eq!(
                sp_io::storage::get(&storage_prefix(pallet_prefix, item)),
                None
            );
        }
        assert_eq!(crate::Difficulty::<Test>::get(), U256::from(100u64));
        // Map entries cleared.
        let prefix = storage_prefix(pallet_prefix, b"BlockHeaders");
        let key = [
            &prefix[..],
            &sp_io::hashing::blake2_128(&[1u8])[..],
            &[1u8][..],
        ]
        .concat();
        assert_eq!(sp_io::storage::get(&key), None);
    });
}

#[test]
fn migration_is_noop_when_already_v2() {
    new_test_ext().execute_with(|| {
        // Fresh storage has no stamped version; simulate an already-migrated
        // chain by stamping v2 first.
        StorageVersion::new(2).put::<GhostConsensus>();
        assert_eq!(GhostConsensus::on_chain_storage_version(), STORAGE_VERSION);
        crate::Difficulty::<Test>::put(U256::from(9_999u64));
        MigrateToV2::<Test>::on_runtime_upgrade();
        assert_eq!(crate::Difficulty::<Test>::get(), U256::from(9_999u64));
    });
}

// ---------------------------------------------------------------------------
// Property tests (proptest)
// ---------------------------------------------------------------------------

proptest::proptest! {
    /// Block reward conservation for arbitrary validator sets: exactly
    /// `BlockReward` is minted per block, each staked active validator
    /// receives `floor(60% * stake / total_stake)`, and the author absorbs
    /// the 40% share plus every rounding remainder — no dust is created or
    /// destroyed anywhere in the split.
    #[test]
    fn reward_conservation_arbitrary(
        stakes in proptest::collection::vec(MIN_STAKE..9_000u128, 0..=5usize),
        author_idx in 0usize..6usize,
    ) {
        new_test_ext().execute_with(|| {
            const ACCOUNTS: [AccountId; 5] = [ALICE, BOB, CHARLIE, DAVE, EVE];
            for (i, stake) in stakes.iter().enumerate() {
                bond_and_validate(ACCOUNTS[i], *stake);
            }
            <GhostConsensus as pallet_session::SessionManager<AccountId>>::new_session_genesis(0);
            <GhostConsensus as pallet_session::SessionManager<AccountId>>::start_session(0);

            // Author can be an outsider (minted-into fresh account) or a
            // staked validator.
            let author = *ACCOUNTS.get(author_idx).unwrap_or(&99u64);
            let reserve = GhostConsensus::account_id();
            let watched: Vec<AccountId> = ACCOUNTS
                .iter()
                .copied()
                .chain([author, reserve])
                .collect();
            // Baseline AFTER bonding: bonds are holds (already reflected in
            // free_balance), so deltas across on_finalize are pure mints.
            let before: Vec<Balance> = watched.iter().map(Balances::free_balance).collect();
            let issuance_before = Balances::total_issuance();

            set_author(author);
            <GhostConsensus as Hooks<u64>>::on_finalize(System::block_number());

            // Reconstruct the expected active set + split from storage.
            let mut scored: Vec<(AccountId, Balance)> = crate::Candidates::<Test>::get()
                .into_iter()
                .map(|who| (who, GhostConsensus::bonded(&who)))
                .filter(|(_, s)| *s >= MIN_STAKE)
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            scored.truncate(3); // MaxValidators
            let total_stake: Balance = scored.iter().map(|(_, s)| s).sum();

            let pot = 60u128; // 60% of BLOCK_REWARD = 100
            let mut expected: std::collections::BTreeMap<AccountId, Balance> =
                std::collections::BTreeMap::new();
            let paid_to_validators: Balance = if total_stake.is_zero() {
                expected.insert(reserve, pot);
                pot
            } else {
                scored
                    .iter()
                    .map(|(who, stake)| {
                        let share: Balance = (U256::from(pot) * U256::from(*stake)
                            / U256::from(total_stake))
                        .try_into()
                        .unwrap();
                        *expected.entry(*who).or_default() += share;
                        share
                    })
                    .sum()
            };
            // Author: 40% + all rounding remainder (100 - paid - 40 → +40).
            *expected.entry(author).or_default() += BLOCK_REWARD - paid_to_validators;

            for (i, who) in watched.iter().enumerate() {
                let delta = Balances::free_balance(who).saturating_sub(before[i]);
                assert_eq!(
                    delta,
                    *expected.get(who).unwrap_or(&0),
                    "unexpected mint for account {who}"
                );
            }
            assert_eq!(
                Balances::total_issuance().saturating_sub(issuance_before),
                BLOCK_REWARD,
                "total minted must equal BlockReward exactly"
            );
        });
    }
}
