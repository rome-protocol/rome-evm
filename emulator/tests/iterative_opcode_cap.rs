//! Drives the emulator's iterative estimate end to end over an in-memory
//! account store: the estimate of a long-running CREATE must stop at the
//! request-scoped opcode budget instead of running until the signer is drained.

use {
    emulator::{account_storage::AccountStorage, do_call_iterative, reg_owner, Emulation},
    rome_evm::{
        error::{Result, RomeProgramError::*},
        registration_key, state::pda::Pda, H160, MAX_OPCODES_PER_EMULATION,
    },
    solana_account::Account,
    solana_program::{clock::Clock, pubkey::Pubkey, rent::Rent, sysvar},
    solana_sdk_ids::native_loader,
    solana_system_interface::program as system_program,
    std::{
        collections::HashMap,
        sync::{Arc, RwLock},
    },
};

const CHAIN: u64 = 1;
const SIGNER_LAMPORTS: u64 = 1_000_000_000_000;
const LIMIT: u64 = 1_000_000;

struct MemStorage(RwLock<HashMap<Pubkey, Account>>);

impl AccountStorage for MemStorage {
    fn get_account(&self, key: &Pubkey) -> Result<Option<Account>> {
        Ok(self.0.read().unwrap().get(key).cloned())
    }
    fn get_multiple_accounts(&self, keys: &[Pubkey]) -> Result<Vec<Option<Account>>> {
        let map = self.0.read().unwrap();
        Ok(keys.iter().map(|k| map.get(k).cloned()).collect())
    }
}

impl MemStorage {
    fn put(&self, key: Pubkey, acc: Account) {
        self.0.write().unwrap().insert(key, acc);
    }
    fn absorb(&self, emu: &Emulation) {
        let mut map = self.0.write().unwrap();
        for (key, item) in &emu.accounts {
            map.insert(*key, item.account.clone().into());
        }
    }
}

fn system_account(lamports: u64) -> Account {
    Account { lamports, data: vec![], owner: system_program::ID, executable: false, rent_epoch: 0 }
}

fn sysvar_account(data: Vec<u8>) -> Account {
    Account { lamports: 1, data, owner: sysvar::ID, executable: false, rent_epoch: 0 }
}

/// A registered chain whose estimate authority holds `SIGNER_LAMPORTS`.
fn chain_storage(program_id: &Pubkey) -> Arc<MemStorage> {
    let storage = Arc::new(MemStorage(RwLock::new(HashMap::new())));
    let legacy = bincode::config::legacy();
    storage.put(
        sysvar::clock::ID,
        sysvar_account(bincode::serde::encode_to_vec(Clock::default(), legacy).unwrap()),
    );
    storage.put(
        sysvar::rent::ID,
        sysvar_account(bincode::serde::encode_to_vec(Rent::default(), legacy).unwrap()),
    );
    storage.put(
        system_program::ID,
        Account { lamports: 1, data: vec![], owner: native_loader::ID, executable: true, rent_epoch: 0 },
    );
    storage.put(registration_key::ID, system_account(SIGNER_LAMPORTS));
    storage.put(Pda::new_(program_id, CHAIN).gas_estimate_key(), system_account(SIGNER_LAMPORTS));

    let mut data = CHAIN.to_le_bytes().to_vec();
    data.push(0); // single_state = false, no mint (SOL-based chain)
    let emu = reg_owner(program_id, &data, &registration_key::ID, storage.clone(), None).unwrap();
    storage.absorb(&emu);
    storage
}

fn rlp_str(b: &[u8]) -> Vec<u8> {
    match b {
        [] => vec![0x80],
        [x] if *x < 0x80 => vec![*x],
        _ => {
            assert!(b.len() < 56);
            [&[0x80 + b.len() as u8][..], b].concat()
        }
    }
}

fn rlp_list(items: &[Vec<u8>]) -> Vec<u8> {
    let body = items.concat();
    assert!(body.len() < 56);
    [&[0xc0 + body.len() as u8][..], &body].concat()
}

/// `0x02 || rlp([chain, nonce, prio, max_fee, gas, to, value, data, access_list])`
/// for a CREATE of `init_code`; gas fields are irrelevant on the estimate path.
fn unsigned_create(init_code: &[u8]) -> Vec<u8> {
    let list = rlp_list(&[
        rlp_str(&[CHAIN as u8]),
        rlp_str(&[]),
        rlp_str(&[]),
        rlp_str(&[]),
        rlp_str(&[]),
        rlp_str(&[]),
        rlp_str(&[]),
        rlp_str(init_code),
        rlp_list(&[]),
    ]);
    [&[0x02][..], &list].concat()
}

/// Init code that counts `rounds` down to zero and stops: ten opcodes per
/// round, so the executed step count is the world this test varies.
fn countdown(rounds: u32) -> Vec<u8> {
    let mut code = vec![0x63]; // PUSH4 rounds
    code.extend_from_slice(&rounds.to_be_bytes());
    code.extend_from_slice(&[
        0x5b, // 05 JUMPDEST
        0x80, // 06 DUP1
        0x15, // 07 ISZERO
        0x60, 0x12, // 08 PUSH1 end
        0x57, // 0a JUMPI
        0x60, 0x01, // 0b PUSH1 1
        0x90, // 0d SWAP1
        0x03, // 0e SUB
        0x60, 0x05, // 0f PUSH1 loop
        0x56, // 11 JUMP
        0x5b, // 12 JUMPDEST
        0x00, // 13 STOP
    ]);
    code
}

fn estimate(program_id: &Pubkey, storage: Arc<MemStorage>, init_code: &[u8]) -> Result<Emulation> {
    let signer = Pda::new_(program_id, CHAIN).gas_estimate_key();
    let mut data = H160::repeat_byte(0x11).as_bytes().to_vec();
    data.extend_from_slice(&LIMIT.to_le_bytes());
    data.extend_from_slice(&unsigned_create(init_code));
    do_call_iterative(program_id, &data, &signer, storage, None)
}

#[test]
fn estimate_past_the_opcode_budget_stops_with_opcode_limit_exceeded() {
    let program_id = Pubkey::new_unique();
    let storage = chain_storage(&program_id);
    let rounds = (MAX_OPCODES_PER_EMULATION / 10 + MAX_OPCODES_PER_EMULATION / 50) as u32;

    let res = estimate(&program_id, storage, &countdown(rounds));

    match res {
        Err(OpcodeLimitExceeded) => {}
        Ok(emu) => panic!(
            "estimate ran to completion past the budget: steps_executed = {}",
            emu.vm.unwrap().steps_executed
        ),
        Err(e) => panic!("unexpected error: {e}"),
    }
}

#[test]
fn estimate_under_the_opcode_budget_still_completes_across_legs() {
    let program_id = Pubkey::new_unique();
    let storage = chain_storage(&program_id);
    let rounds = (MAX_OPCODES_PER_EMULATION / 100) as u32;

    let emu = estimate(&program_id, storage, &countdown(rounds)).unwrap();
    let vm = emu.vm.unwrap();

    assert!(vm.exit_reason.is_succeed(), "{:?}", vm.exit_reason);
    assert!(vm.steps_executed > LIMIT, "must span more than one leg: {}", vm.steps_executed);
    assert!(vm.steps_executed < MAX_OPCODES_PER_EMULATION);
}
