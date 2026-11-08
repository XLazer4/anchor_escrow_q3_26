#![allow(dead_code)]

use {
    anchor_lang::{
        prelude::{Clock, Pubkey},
        solana_program::{
            instruction::{error::InstructionError, Instruction},
            system_program,
        },
        InstructionData, ToAccountMetas,
    },
    anchor_spl::associated_token::{self, get_associated_token_address_with_program_id},
    escrow::ESCROW_SEED,
    litesvm::{types::TransactionResult, LiteSVM},
    litesvm_token::{CreateAssociatedTokenAccount, CreateMint, MintTo},
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
    solana_transaction_error::TransactionError,
};

pub const DECIMALS: u8 = 6;
pub const MAKER_A: u64 = 1_000_000_000;
pub const TAKER_B: u64 = 1_000_000_000;
/// default offer length used by make_ix
pub const DURATION: i64 = 60 * 60 * 24;

pub struct Env {
    pub svm: LiteSVM,
    pub maker: Keypair,
    pub taker: Keypair,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub maker_ata_a: Pubkey,
    pub taker_ata_b: Pubkey,
    pub token_program: Pubkey,
}

impl Env {
    pub fn new() -> Self {
        Self::with_token_program(anchor_spl::token::ID)
    }

    /// maker holds MAKER_A of mint_a, taker holds TAKER_B of mint_b
    pub fn with_token_program(token_program: Pubkey) -> Self {
        let mut svm = LiteSVM::new();
        let bytes = include_bytes!(concat!(env!("CARGO_TARGET_TMPDIR"), "/../deploy/escrow.so"));
        svm.add_program(escrow::id(), bytes).unwrap();

        let maker = Keypair::new();
        let taker = Keypair::new();
        svm.airdrop(&maker.pubkey(), 10_000_000_000).unwrap();
        svm.airdrop(&taker.pubkey(), 10_000_000_000).unwrap();

        let mint_a = CreateMint::new(&mut svm, &maker)
            .decimals(DECIMALS)
            .token_program_id(&token_program)
            .send()
            .unwrap();
        let mint_b = CreateMint::new(&mut svm, &taker)
            .decimals(DECIMALS)
            .token_program_id(&token_program)
            .send()
            .unwrap();

        let maker_ata_a = CreateAssociatedTokenAccount::new(&mut svm, &maker, &mint_a)
            .token_program_id(&token_program)
            .send()
            .unwrap();
        let taker_ata_b = CreateAssociatedTokenAccount::new(&mut svm, &taker, &mint_b)
            .token_program_id(&token_program)
            .send()
            .unwrap();

        MintTo::new(&mut svm, &maker, &mint_a, &maker_ata_a, MAKER_A)
            .token_program_id(&token_program)
            .send()
            .unwrap();
        MintTo::new(&mut svm, &taker, &mint_b, &taker_ata_b, TAKER_B)
            .token_program_id(&token_program)
            .send()
            .unwrap();

        Env {
            svm,
            maker,
            taker,
            mint_a,
            mint_b,
            maker_ata_a,
            taker_ata_b,
            token_program,
        }
    }

    pub fn ata(&self, owner: &Pubkey, mint: &Pubkey) -> Pubkey {
        get_associated_token_address_with_program_id(owner, mint, &self.token_program)
    }

    pub fn escrow_pda(&self, maker: &Pubkey, seed: u64) -> Pubkey {
        Pubkey::find_program_address(
            &[ESCROW_SEED, maker.as_ref(), &seed.to_le_bytes()],
            &escrow::id(),
        )
        .0
    }

    pub fn balance(&self, token_account: &Pubkey) -> u64 {
        token_balance(&self.svm, token_account)
    }

    pub fn lamports(&self, account: &Pubkey) -> u64 {
        self.svm.get_balance(account).unwrap_or(0)
    }

    pub fn exists(&self, account: &Pubkey) -> bool {
        self.svm
            .get_account(account)
            .is_some_and(|a| a.lamports > 0)
    }

    pub fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }

    /// move the clock forward (or back with a negative value)
    pub fn warp(&mut self, secs: i64) {
        let mut clock: Clock = self.svm.get_sysvar();
        clock.unix_timestamp += secs;
        self.svm.set_sysvar(&clock);
    }

    /// open to anyone, expires in DURATION
    pub fn make_ix(&self, seed: u64, deposit: u64, receive: u64) -> Instruction {
        self.make_ix_with(seed, deposit, receive, DURATION, None)
    }

    pub fn make_ix_with(
        &self,
        seed: u64,
        deposit: u64,
        receive: u64,
        duration: i64,
        taker: Option<Pubkey>,
    ) -> Instruction {
        let maker = self.maker.pubkey();
        let escrow = self.escrow_pda(&maker, seed);
        Instruction::new_with_bytes(
            escrow::id(),
            &escrow::instruction::Make {
                seed,
                deposit,
                receive,
                duration,
                taker,
            }
            .data(),
            escrow::accounts::Make {
                maker,
                mint_a: self.mint_a,
                mint_b: self.mint_b,
                maker_ata_a: self.maker_ata_a,
                escrow,
                vault: self.ata(&escrow, &self.mint_a),
                associated_token_program: associated_token::ID,
                token_program: self.token_program,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        )
    }

    pub fn take_ix(&self, taker: &Pubkey, seed: u64, expected_receive: u64) -> Instruction {
        let maker = self.maker.pubkey();
        let escrow = self.escrow_pda(&maker, seed);
        Instruction::new_with_bytes(
            escrow::id(),
            &escrow::instruction::Take { expected_receive }.data(),
            escrow::accounts::Take {
                taker: *taker,
                maker,
                mint_a: self.mint_a,
                mint_b: self.mint_b,
                taker_ata_a: self.ata(taker, &self.mint_a),
                taker_ata_b: self.ata(taker, &self.mint_b),
                maker_ata_b: self.ata(&maker, &self.mint_b),
                escrow,
                vault: self.ata(&escrow, &self.mint_a),
                associated_token_program: associated_token::ID,
                token_program: self.token_program,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        )
    }

    /// `caller` signs, funds always go to the real maker
    pub fn refund_ix(&self, caller: &Pubkey, seed: u64) -> Instruction {
        let maker = self.maker.pubkey();
        let escrow = self.escrow_pda(&maker, seed);
        Instruction::new_with_bytes(
            escrow::id(),
            &escrow::instruction::Refund {}.data(),
            escrow::accounts::Refund {
                caller: *caller,
                maker,
                mint_a: self.mint_a,
                maker_ata_a: self.ata(&maker, &self.mint_a),
                escrow,
                vault: self.ata(&escrow, &self.mint_a),
                associated_token_program: associated_token::ID,
                token_program: self.token_program,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
        )
    }

    pub fn update_ix(
        &self,
        signer: &Pubkey,
        seed: u64,
        new_receive: Option<u64>,
        new_expires_at: Option<i64>,
    ) -> Instruction {
        let escrow = self.escrow_pda(&self.maker.pubkey(), seed);
        Instruction::new_with_bytes(
            escrow::id(),
            &escrow::instruction::Update {
                new_receive,
                new_expires_at,
            }
            .data(),
            escrow::accounts::Update {
                maker: *signer,
                escrow,
            }
            .to_account_metas(None),
        )
    }
}

pub fn token_balance(svm: &LiteSVM, token_account: &Pubkey) -> u64 {
    let acc = svm
        .get_account(token_account)
        .expect("token account missing");
    // amount sits right after mint (32) + owner (32) in both token programs
    u64::from_le_bytes(acc.data[64..72].try_into().unwrap())
}

pub fn try_send(svm: &mut LiteSVM, payer: &Keypair, ix: Instruction) -> TransactionResult {
    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[payer]).unwrap();
    let res = svm.send_transaction(tx);
    svm.expire_blockhash();
    res
}

pub fn send(svm: &mut LiteSVM, payer: &Keypair, ix: Instruction) {
    try_send(svm, payer, ix).unwrap_or_else(|err| {
        panic!("tx failed: {err:?}\nlogs: {logs:#?}", logs = err.meta.logs);
    });
}

pub fn expect_err(svm: &mut LiteSVM, payer: &Keypair, ix: Instruction, code: u32) {
    let err = try_send(svm, payer, ix).expect_err("tx should have failed");
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::Custom(code)),
        "logs: {:#?}",
        err.meta.logs
    );
}
