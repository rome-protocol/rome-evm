//! Drives the emulator's TransmitTx arm over an in-memory account store across
//! the chunks of one transaction: the TxHolder header (hash, iteration) written
//! by chunk 0 must land in `Emulation.accounts`, so a consumer that carries the
//! emulated state into the next chunk sees the holder continued, not reset.
//! The on-chain arm works on one live account and lands every write.

use {
    emulator::{account_storage::AccountStorage, reg_owner, transmit_tx, Emulation},
    rome_evm::{
        error::Result, registration_key, state::pda::Pda, Data, Holder, TxHolder, H256,
    },
    solana_account::Account,
    solana_program::{
        account_info::AccountInfo, clock::Clock, pubkey::Pubkey, rent::Rent, sysvar,
    },
    solana_sdk_ids::native_loader,
    solana_system_interface::program as system_program,
    std::{
        collections::HashMap,
        sync::{Arc, RwLock},
    },
};

const CHAIN: u64 = 1;
const SIGNER_LAMPORTS: u64 = 1_000_000_000_000;
const HOLDER: u64 = 0;

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

/// A registered chain plus a funded signer that owns the tx holder.
fn chain_storage(program_id: &Pubkey, signer: &Pubkey) -> Arc<MemStorage> {
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
    storage.put(*signer, system_account(SIGNER_LAMPORTS));

    let mut data = CHAIN.to_le_bytes().to_vec();
    data.push(0); // single_state = false, no mint (SOL-based chain)
    let emu = reg_owner(program_id, &data, &registration_key::ID, storage.clone(), None).unwrap();
    storage.absorb(&emu);
    storage
}

/// `holder_index | offset | hash | chain_id | chunk`, the TransmitTx args layout.
fn transmit_args(offset: usize, hash: H256, chunk: &[u8]) -> Vec<u8> {
    let mut data = HOLDER.to_le_bytes().to_vec();
    data.extend_from_slice(&(offset as u64).to_le_bytes());
    data.extend_from_slice(hash.as_bytes());
    data.extend_from_slice(&CHAIN.to_le_bytes());
    data.extend_from_slice(chunk);
    data
}

/// The holder as the store now holds it: (hash, iteration, payload bytes).
fn holder_state(storage: &MemStorage, key: &Pubkey) -> (H256, u16, Vec<u8>) {
    let mut acc = storage.get_account(key).unwrap().expect("tx holder absent from the store");
    let owner = acc.owner;
    let info = AccountInfo::new(key, false, true, &mut acc.lamports, &mut acc.data, &owner, false);
    let (hash, iteration) = {
        let header = TxHolder::from_account(&info).unwrap();
        (header.hash, header.iter_cnt)
    };
    let payload = Holder::from_account(&info).unwrap().to_vec();
    (hash, iteration, payload)
}

#[test]
fn transmit_tx_header_write_lands_and_the_next_chunk_continues_the_holder() {
    let program_id = Pubkey::new_unique();
    let signer = Pubkey::new_unique();
    let storage = chain_storage(&program_id, &signer);
    let holder_key = Pda::new_(&program_id, CHAIN).tx_holder_key(&signer, HOLDER).0;

    let ix_hash = H256::repeat_byte(0x42);
    let tx: Vec<u8> = (0..96u8).collect();
    let (chunk0, chunk1) = tx.split_at(64);

    // chunk 0: a fresh holder is created, reset to the ix hash and bumped to iteration 1
    let emu = transmit_tx(&program_id, &transmit_args(0, ix_hash, chunk0), &signer, storage.clone(), None)
        .unwrap();
    storage.absorb(&emu);

    let (hash, iteration, payload) = holder_state(&storage, &holder_key);
    assert_eq!(hash, ix_hash, "chunk 0: the holder's hash must be the ix hash");
    assert_eq!(iteration, 1, "chunk 0: the holder's iteration must be 1");
    assert_eq!(payload, chunk0, "chunk 0: the holder's payload must be chunk 0");

    // chunk 1: the same hash continues the holder instead of resetting it
    let emu = transmit_tx(&program_id, &transmit_args(64, ix_hash, chunk1), &signer, storage.clone(), None)
        .unwrap();
    storage.absorb(&emu);

    let (hash, iteration, payload) = holder_state(&storage, &holder_key);
    assert_eq!(hash, ix_hash, "chunk 1: the holder's hash must be unchanged");
    assert_eq!(iteration, 2, "chunk 1: the holder must be continued, not reset");
    assert_eq!(payload, tx, "chunk 1: the holder's payload must be the concatenated chunks");
}
