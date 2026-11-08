mod common;

use {
    anchor_lang::AccountDeserialize, common::*, escrow::Escrow, litesvm_token::CloseAccount,
    solana_keypair::Keypair, solana_signer::Signer,
};

const SEED: u64 = 42;
const DEPOSIT: u64 = 250_000_000;
const RECEIVE: u64 = 400_000_000;

const ERR_INVALID_AMOUNT: u32 = 6000;
const ERR_SAME_MINT: u32 = 6001;
const ERR_RECEIVE_CHANGED: u32 = 6002;
const ERR_NOT_EXPIRED: u32 = 6005;
// seeds are checked before has_one, and both are derived from the maker
// key, so a wrong signer trips ConstraintSeeds first
const ERR_SEEDS: u32 = 2006;
const ERR_NOT_INITIALIZED: u32 = 3012;

fn read_escrow(env: &Env, seed: u64) -> Escrow {
    let pda = env.escrow_pda(&env.maker.pubkey(), seed);
    let acc = env.svm.get_account(&pda).expect("escrow missing");
    Escrow::try_deserialize(&mut acc.data.as_slice()).unwrap()
}

fn make(env: &mut Env, seed: u64) {
    let ix = env.make_ix(seed, DEPOSIT, RECEIVE);
    send(&mut env.svm, &env.maker, ix);
}

// ---- make ----

#[test]
fn make_locks_deposit_and_saves_terms() {
    let mut env = Env::new();
    make(&mut env, SEED);

    let escrow_pda = env.escrow_pda(&env.maker.pubkey(), SEED);
    let vault = env.ata(&escrow_pda, &env.mint_a);

    assert_eq!(env.balance(&vault), DEPOSIT);
    assert_eq!(env.balance(&env.maker_ata_a), MAKER_A - DEPOSIT);

    let state = read_escrow(&env, SEED);
    assert_eq!(state.seed, SEED);
    assert_eq!(state.maker, env.maker.pubkey());
    assert_eq!(state.mint_a, env.mint_a);
    assert_eq!(state.mint_b, env.mint_b);
    assert_eq!(state.receive, RECEIVE);
}

#[test]
fn make_rejects_zero_deposit() {
    let mut env = Env::new();
    let ix = env.make_ix(SEED, 0, RECEIVE);
    expect_err(&mut env.svm, &env.maker, ix, ERR_INVALID_AMOUNT);
}

#[test]
fn make_rejects_zero_receive() {
    let mut env = Env::new();
    let ix = env.make_ix(SEED, DEPOSIT, 0);
    expect_err(&mut env.svm, &env.maker, ix, ERR_INVALID_AMOUNT);
}

#[test]
fn make_rejects_same_mint_on_both_sides() {
    let mut env = Env::new();
    env.mint_b = env.mint_a;
    let ix = env.make_ix(SEED, DEPOSIT, RECEIVE);
    expect_err(&mut env.svm, &env.maker, ix, ERR_SAME_MINT);
}

#[test]
fn make_fails_with_more_than_balance() {
    let mut env = Env::new();
    let ix = env.make_ix(SEED, MAKER_A + 1, RECEIVE);
    assert!(try_send(&mut env.svm, &env.maker, ix).is_err());
}

#[test]
fn maker_can_have_several_escrows() {
    let mut env = Env::new();
    make(&mut env, 1);
    make(&mut env, 2);

    assert_eq!(read_escrow(&env, 1).seed, 1);
    assert_eq!(read_escrow(&env, 2).seed, 2);
    assert_eq!(env.balance(&env.maker_ata_a), MAKER_A - 2 * DEPOSIT);
}

#[test]
fn same_seed_cannot_be_reused_while_open() {
    let mut env = Env::new();
    make(&mut env, SEED);
    let ix = env.make_ix(SEED, DEPOSIT, RECEIVE);
    assert!(try_send(&mut env.svm, &env.maker, ix).is_err());
}

// ---- take ----

#[test]
fn take_swaps_tokens_and_closes_everything() {
    let mut env = Env::new();
    make(&mut env, SEED);

    let maker = env.maker.pubkey();
    let taker = env.taker.pubkey();
    let escrow_pda = env.escrow_pda(&maker, SEED);
    let vault = env.ata(&escrow_pda, &env.mint_a);
    let rent_back = env.lamports(&escrow_pda) + env.lamports(&vault);
    let maker_lamports = env.lamports(&maker);

    let ix = env.take_ix(&taker, SEED, RECEIVE);
    send(&mut env.svm, &env.taker, ix);

    assert_eq!(env.balance(&env.ata(&taker, &env.mint_a)), DEPOSIT);
    assert_eq!(env.balance(&env.taker_ata_b), TAKER_B - RECEIVE);
    assert_eq!(env.balance(&env.ata(&maker, &env.mint_b)), RECEIVE);

    assert!(!env.exists(&escrow_pda), "escrow should be closed");
    assert!(!env.exists(&vault), "vault should be closed");
    // taker paid for maker_ata_b, so the maker gets exactly the two rents back
    assert_eq!(env.lamports(&maker), maker_lamports + rent_back);
}

#[test]
fn take_fails_if_taker_is_short_on_mint_b() {
    let mut env = Env::new();
    let ix = env.make_ix(SEED, DEPOSIT, TAKER_B + 1);
    send(&mut env.svm, &env.maker, ix);

    let ix = env.take_ix(&env.taker.pubkey(), SEED, TAKER_B + 1);
    assert!(try_send(&mut env.svm, &env.taker, ix).is_err());

    // nothing moved
    let escrow_pda = env.escrow_pda(&env.maker.pubkey(), SEED);
    assert_eq!(env.balance(&env.ata(&escrow_pda, &env.mint_a)), DEPOSIT);
    assert_eq!(env.balance(&env.taker_ata_b), TAKER_B);
}

#[test]
fn cannot_take_twice() {
    let mut env = Env::new();
    make(&mut env, SEED);

    let ix = env.take_ix(&env.taker.pubkey(), SEED, RECEIVE);
    send(&mut env.svm, &env.taker, ix.clone());
    expect_err(&mut env.svm, &env.taker, ix, ERR_NOT_INITIALIZED);
}

// ---- refund ----

#[test]
fn refund_returns_deposit_and_closes_everything() {
    let mut env = Env::new();
    make(&mut env, SEED);

    let maker = env.maker.pubkey();
    let escrow_pda = env.escrow_pda(&maker, SEED);
    let vault = env.ata(&escrow_pda, &env.mint_a);

    let ix = env.refund_ix(&maker, SEED);
    send(&mut env.svm, &env.maker, ix);

    assert_eq!(env.balance(&env.maker_ata_a), MAKER_A);
    assert!(!env.exists(&escrow_pda));
    assert!(!env.exists(&vault));
}

#[test]
fn only_maker_can_refund_while_live() {
    let mut env = Env::new();
    make(&mut env, SEED);

    let thief = Keypair::new();
    env.svm.airdrop(&thief.pubkey(), 1_000_000_000).unwrap();

    let ix = env.refund_ix(&thief.pubkey(), SEED);
    expect_err(&mut env.svm, &thief, ix, ERR_NOT_EXPIRED);

    let escrow_pda = env.escrow_pda(&env.maker.pubkey(), SEED);
    assert_eq!(env.balance(&env.ata(&escrow_pda, &env.mint_a)), DEPOSIT);
}

#[test]
fn refund_recreates_maker_ata_if_it_was_closed() {
    let mut env = Env::new();
    // put everything in so the ata can be closed
    let ix = env.make_ix(SEED, MAKER_A, RECEIVE);
    send(&mut env.svm, &env.maker, ix);

    let maker = env.maker.pubkey();
    let token_program = env.token_program;
    CloseAccount::new(&mut env.svm, &env.maker, &env.maker_ata_a, &maker)
        .token_program_id(&token_program)
        .send()
        .unwrap();
    assert!(!env.exists(&env.maker_ata_a));

    let ix = env.refund_ix(&maker, SEED);
    send(&mut env.svm, &env.maker, ix);

    assert_eq!(env.balance(&env.maker_ata_a), MAKER_A);
}

#[test]
fn cannot_take_after_refund() {
    let mut env = Env::new();
    make(&mut env, SEED);

    let ix = env.refund_ix(&env.maker.pubkey(), SEED);
    send(&mut env.svm, &env.maker, ix);

    let ix = env.take_ix(&env.taker.pubkey(), SEED, RECEIVE);
    expect_err(&mut env.svm, &env.taker, ix, ERR_NOT_INITIALIZED);
}

// ---- update ----

#[test]
fn update_changes_receive_amount() {
    let mut env = Env::new();
    make(&mut env, SEED);

    let ix = env.update_ix(&env.maker.pubkey(), SEED, Some(RECEIVE / 2), None);
    send(&mut env.svm, &env.maker, ix);

    assert_eq!(read_escrow(&env, SEED).receive, RECEIVE / 2);
}

#[test]
fn take_after_update_pays_the_new_amount() {
    let mut env = Env::new();
    make(&mut env, SEED);

    let new_receive = 123_000_000;
    let ix = env.update_ix(&env.maker.pubkey(), SEED, Some(new_receive), None);
    send(&mut env.svm, &env.maker, ix);

    let taker = env.taker.pubkey();
    let ix = env.take_ix(&taker, SEED, new_receive);
    send(&mut env.svm, &env.taker, ix);

    assert_eq!(
        env.balance(&env.ata(&env.maker.pubkey(), &env.mint_b)),
        new_receive
    );
    assert_eq!(env.balance(&env.taker_ata_b), TAKER_B - new_receive);
}

#[test]
fn take_fails_if_maker_raised_the_price_first() {
    let mut env = Env::new();
    make(&mut env, SEED);

    // taker saw RECEIVE, maker bumps it before the take lands
    let ix = env.update_ix(&env.maker.pubkey(), SEED, Some(RECEIVE * 2), None);
    send(&mut env.svm, &env.maker, ix);

    let ix = env.take_ix(&env.taker.pubkey(), SEED, RECEIVE);
    expect_err(&mut env.svm, &env.taker, ix, ERR_RECEIVE_CHANGED);
    assert_eq!(env.balance(&env.taker_ata_b), TAKER_B);
}

#[test]
fn update_rejects_zero() {
    let mut env = Env::new();
    make(&mut env, SEED);

    let ix = env.update_ix(&env.maker.pubkey(), SEED, Some(0), None);
    expect_err(&mut env.svm, &env.maker, ix, ERR_INVALID_AMOUNT);
}

#[test]
fn only_maker_can_update() {
    let mut env = Env::new();
    make(&mut env, SEED);

    let ix = env.update_ix(&env.taker.pubkey(), SEED, Some(1), None);
    expect_err(&mut env.svm, &env.taker, ix, ERR_SEEDS);
    assert_eq!(read_escrow(&env, SEED).receive, RECEIVE);
}

// ---- token-2022 ----

#[test]
fn works_with_token_2022_mints() {
    let mut env = Env::with_token_program(anchor_spl::token_2022::ID);
    make(&mut env, SEED);

    let taker = env.taker.pubkey();
    let ix = env.take_ix(&taker, SEED, RECEIVE);
    send(&mut env.svm, &env.taker, ix);

    assert_eq!(env.balance(&env.ata(&taker, &env.mint_a)), DEPOSIT);
    assert_eq!(
        env.balance(&env.ata(&env.maker.pubkey(), &env.mint_b)),
        RECEIVE
    );
}
