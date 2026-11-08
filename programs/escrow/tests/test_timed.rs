//! Tests for the timed part of the escrow: expiry, the permissionless
//! refund after expiry, and the optional designated taker.

mod common;

use {
    anchor_lang::{prelude::Pubkey, AccountDeserialize},
    common::*,
    escrow::Escrow,
    litesvm_token::{CreateAssociatedTokenAccount, MintTo},
    solana_keypair::Keypair,
    solana_signer::Signer,
};

const SEED: u64 = 7;
const DEPOSIT: u64 = 100_000_000;
const RECEIVE: u64 = 300_000_000;
const HOUR: i64 = 60 * 60;

const ERR_INVALID_EXPIRY: u32 = 6003;
const ERR_EXPIRED: u32 = 6004;
const ERR_NOT_EXPIRED: u32 = 6005;
const ERR_WRONG_TAKER: u32 = 6006;
const ERR_NOTHING_TO_UPDATE: u32 = 6007;

/// start the clock somewhere realistic instead of 0
fn env() -> Env {
    let mut env = Env::new();
    env.warp(1_750_000_000);
    env
}

fn read_escrow(env: &Env) -> Escrow {
    let pda = env.escrow_pda(&env.maker.pubkey(), SEED);
    let acc = env.svm.get_account(&pda).expect("escrow missing");
    Escrow::try_deserialize(&mut acc.data.as_slice()).unwrap()
}

fn make(env: &mut Env, duration: i64, taker: Option<Pubkey>) {
    let ix = env.make_ix_with(SEED, DEPOSIT, RECEIVE, duration, taker);
    send(&mut env.svm, &env.maker, ix);
}

/// a third wallet with some mint_b so it can take
fn funded_wallet(env: &mut Env) -> Keypair {
    let kp = Keypair::new();
    env.svm.airdrop(&kp.pubkey(), 1_000_000_000).unwrap();
    let token_program = env.token_program;
    let mint_b = env.mint_b;
    let ata = CreateAssociatedTokenAccount::new(&mut env.svm, &kp, &mint_b)
        .token_program_id(&token_program)
        .send()
        .unwrap();
    // taker is mint_b's authority
    MintTo::new(&mut env.svm, &env.taker, &mint_b, &ata, RECEIVE)
        .token_program_id(&token_program)
        .send()
        .unwrap();
    kp
}

// ---- make ----

#[test]
fn make_sets_expiry_from_the_clock() {
    let mut env = env();
    let now = env.now();
    make(&mut env, HOUR, None);

    let state = read_escrow(&env);
    assert_eq!(state.expires_at, now + HOUR);
    assert_eq!(state.taker, None);
}

#[test]
fn make_rejects_zero_or_negative_duration() {
    let mut env = env();
    let ix = env.make_ix_with(SEED, DEPOSIT, RECEIVE, 0, None);
    expect_err(&mut env.svm, &env.maker, ix, ERR_INVALID_EXPIRY);

    let ix = env.make_ix_with(SEED, DEPOSIT, RECEIVE, -HOUR, None);
    expect_err(&mut env.svm, &env.maker, ix, ERR_INVALID_EXPIRY);
}

#[test]
fn make_rejects_duration_that_overflows() {
    let mut env = env();
    let ix = env.make_ix_with(SEED, DEPOSIT, RECEIVE, i64::MAX, None);
    expect_err(&mut env.svm, &env.maker, ix, ERR_INVALID_EXPIRY);
}

// ---- take vs expiry ----

#[test]
fn take_works_one_second_before_expiry() {
    let mut env = env();
    make(&mut env, HOUR, None);
    env.warp(HOUR - 1);

    let taker = env.taker.pubkey();
    let ix = env.take_ix(&taker, SEED, RECEIVE);
    send(&mut env.svm, &env.taker, ix);
    assert_eq!(env.balance(&env.ata(&taker, &env.mint_a)), DEPOSIT);
}

#[test]
fn take_fails_at_expiry() {
    let mut env = env();
    make(&mut env, HOUR, None);
    env.warp(HOUR);

    let ix = env.take_ix(&env.taker.pubkey(), SEED, RECEIVE);
    expect_err(&mut env.svm, &env.taker, ix, ERR_EXPIRED);
}

#[test]
fn take_fails_long_after_expiry() {
    let mut env = env();
    make(&mut env, HOUR, None);
    env.warp(30 * 24 * HOUR);

    let ix = env.take_ix(&env.taker.pubkey(), SEED, RECEIVE);
    expect_err(&mut env.svm, &env.taker, ix, ERR_EXPIRED);
}

// ---- refund vs expiry ----

#[test]
fn maker_can_refund_before_expiry() {
    let mut env = env();
    make(&mut env, HOUR, None);

    let maker = env.maker.pubkey();
    let ix = env.refund_ix(&maker, SEED);
    send(&mut env.svm, &env.maker, ix);
    assert_eq!(env.balance(&env.maker_ata_a), MAKER_A);
}

#[test]
fn stranger_cannot_refund_before_expiry() {
    let mut env = env();
    make(&mut env, HOUR, None);
    env.warp(HOUR - 1);

    let ix = env.refund_ix(&env.taker.pubkey(), SEED);
    expect_err(&mut env.svm, &env.taker, ix, ERR_NOT_EXPIRED);
}

#[test]
fn anyone_can_push_refund_after_expiry() {
    let mut env = env();
    make(&mut env, HOUR, None);
    env.warp(HOUR);

    let maker = env.maker.pubkey();
    let cranker = Keypair::new();
    env.svm.airdrop(&cranker.pubkey(), 1_000_000_000).unwrap();

    let escrow_pda = env.escrow_pda(&maker, SEED);
    let vault = env.ata(&escrow_pda, &env.mint_a);
    let rent_back = env.lamports(&escrow_pda) + env.lamports(&vault);
    let maker_lamports = env.lamports(&maker);
    let cranker_lamports = env.lamports(&cranker.pubkey());

    let ix = env.refund_ix(&cranker.pubkey(), SEED);
    send(&mut env.svm, &cranker, ix);

    // tokens and both rents go to the maker, cranker only pays the fee
    assert_eq!(env.balance(&env.maker_ata_a), MAKER_A);
    assert_eq!(env.lamports(&maker), maker_lamports + rent_back);
    assert_eq!(env.lamports(&cranker.pubkey()), cranker_lamports - 5000);
    assert!(!env.exists(&escrow_pda));
    assert!(!env.exists(&vault));
}

// ---- designated taker ----

#[test]
fn designated_taker_can_take() {
    let mut env = env();
    let taker = env.taker.pubkey();
    make(&mut env, HOUR, Some(taker));
    assert_eq!(read_escrow(&env).taker, Some(taker));

    let ix = env.take_ix(&taker, SEED, RECEIVE);
    send(&mut env.svm, &env.taker, ix);
    assert_eq!(env.balance(&env.ata(&taker, &env.mint_a)), DEPOSIT);
}

#[test]
fn other_wallets_cannot_take_a_reserved_escrow() {
    let mut env = env();
    let taker = env.taker.pubkey();
    make(&mut env, HOUR, Some(taker));

    let outsider = funded_wallet(&mut env);
    let ix = env.take_ix(&outsider.pubkey(), SEED, RECEIVE);
    expect_err(&mut env.svm, &outsider, ix, ERR_WRONG_TAKER);
}

#[test]
fn open_escrow_can_be_taken_by_anyone() {
    let mut env = env();
    make(&mut env, HOUR, None);

    let outsider = funded_wallet(&mut env);
    let ix = env.take_ix(&outsider.pubkey(), SEED, RECEIVE);
    send(&mut env.svm, &outsider, ix);
    assert_eq!(
        env.balance(&env.ata(&outsider.pubkey(), &env.mint_a)),
        DEPOSIT
    );
}

#[test]
fn designated_taker_still_bound_by_expiry() {
    let mut env = env();
    let taker = env.taker.pubkey();
    make(&mut env, HOUR, Some(taker));
    env.warp(HOUR);

    let ix = env.take_ix(&taker, SEED, RECEIVE);
    expect_err(&mut env.svm, &env.taker, ix, ERR_EXPIRED);
}

// ---- update ----

#[test]
fn update_can_extend_expiry() {
    let mut env = env();
    make(&mut env, HOUR, None);

    let new_expiry = env.now() + 3 * HOUR;
    let ix = env.update_ix(&env.maker.pubkey(), SEED, None, Some(new_expiry));
    send(&mut env.svm, &env.maker, ix);
    assert_eq!(read_escrow(&env).expires_at, new_expiry);
    assert_eq!(read_escrow(&env).receive, RECEIVE, "receive untouched");

    // past the original expiry, still takeable
    env.warp(2 * HOUR);
    let ix = env.take_ix(&env.taker.pubkey(), SEED, RECEIVE);
    send(&mut env.svm, &env.taker, ix);
}

#[test]
fn update_can_revive_an_expired_escrow() {
    let mut env = env();
    make(&mut env, HOUR, None);
    env.warp(2 * HOUR);

    let new_expiry = env.now() + HOUR;
    let ix = env.update_ix(&env.maker.pubkey(), SEED, None, Some(new_expiry));
    send(&mut env.svm, &env.maker, ix);

    let ix = env.take_ix(&env.taker.pubkey(), SEED, RECEIVE);
    send(&mut env.svm, &env.taker, ix);
}

#[test]
fn update_rejects_expiry_in_the_past() {
    let mut env = env();
    make(&mut env, HOUR, None);

    let now = env.now();
    let ix = env.update_ix(&env.maker.pubkey(), SEED, None, Some(now));
    expect_err(&mut env.svm, &env.maker, ix, ERR_INVALID_EXPIRY);
}

#[test]
fn update_with_nothing_fails() {
    let mut env = env();
    make(&mut env, HOUR, None);

    let ix = env.update_ix(&env.maker.pubkey(), SEED, None, None);
    expect_err(&mut env.svm, &env.maker, ix, ERR_NOTHING_TO_UPDATE);
}

#[test]
fn update_both_fields_at_once() {
    let mut env = env();
    make(&mut env, HOUR, None);

    let new_expiry = env.now() + 5 * HOUR;
    let ix = env.update_ix(
        &env.maker.pubkey(),
        SEED,
        Some(RECEIVE + 1),
        Some(new_expiry),
    );
    send(&mut env.svm, &env.maker, ix);

    let state = read_escrow(&env);
    assert_eq!(state.receive, RECEIVE + 1);
    assert_eq!(state.expires_at, new_expiry);
}
