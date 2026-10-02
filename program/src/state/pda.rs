use {
    super::fast_hash::FastMap,
    crate::{
        error::Result, state::base::Syscall, AccountState, AccountType, BridgeProcessed,
        StarterUnwrapDone, Data,
        OwnerInfo, StateHolder, Storage, TxHolder, ACCOUNT_SEED, OWNER_INFO,
        STATE_HOLDER_SEED, STORAGE_LEN, TX_HOLDER_SEED, CONTRACT_SOL_WALLET,
        TREASURE_SEED, EXTERNAL_AUTHORITY, BRIDGE_PROCESSED_SEED,
        ESTIMATE_AUTHORITY,
        error::RomeProgramError::InvalidGasEstimateAuthority,
    },
    borsh::{BorshDeserialize, BorshSerialize},
    evm::{H160, U256},
    solana_program::{account_info::AccountInfo, pubkey::Pubkey},
    std::{cell::RefCell, rc::Rc},
};

#[derive(BorshSerialize, BorshDeserialize, Default, Clone)]
pub struct Seed {
    pub items: Vec<Vec<u8>>,
}
impl Seed {
    pub fn cast(&self) -> Vec<&[u8]> {
        self.items
            .iter()
            .map(|a| a.as_slice())
            .collect::<Vec<&[u8]>>()
    }
    pub fn add(&mut self, bump_seed: u8) {
        self.items.push(vec![bump_seed]);
    }
    pub fn add_seed(&mut self, seed: Vec<u8>) {
        self.items.push(seed);
    }
    pub fn from_vec(slice: Vec<&[u8]>) -> Self {
        let items = slice
            .iter()
            .map(|&a| a.to_vec())
            .collect::<Vec<_>>();

        Self {
            items
        }
    }
}

type BaseIndex = (Pubkey, [u8; 32]);

pub struct Pda<'a> {
    chain: Vec<u8>,
    program_id: &'a Pubkey,
    pub balance: RefCell<FastMap<H160, (Pubkey, Seed)>>,
    pub storage: RefCell<FastMap<BaseIndex, (Pubkey, Seed)>>,
    pub ro_lock: RefCell<FastMap<Pubkey, (Pubkey, Seed)>>,
    // Transient (not serialized into StateHolder; cleared in `reset`).
    pub external_auth_cache: RefCell<FastMap<H160, (Pubkey, Seed)>>,
    pub syscall: Rc<Syscall>,
}

impl<'a> Pda<'a> {
    pub fn new(program_id: &'a Pubkey, chain: u64, syscall: Rc<Syscall>) -> Self {
        Self {
            chain: chain.to_le_bytes().to_vec(),
            program_id,
            balance: RefCell::new(FastMap::default()),
            storage: RefCell::new(FastMap::default()),
            ro_lock: RefCell::new(FastMap::default()),
            external_auth_cache: RefCell::new(FastMap::default()),
            syscall,
        }
    }
    #[cfg(not(target_os = "solana"))]
    pub fn new_(program_id: &'a Pubkey, chain: u64) -> Self {
        Self {
            chain: chain.to_le_bytes().to_vec(),
            program_id,
            balance: RefCell::new(FastMap::default()),
            storage: RefCell::new(FastMap::default()),
            ro_lock: RefCell::new(FastMap::default()),
            external_auth_cache: RefCell::new(FastMap::default()),
            syscall: Rc::new(Syscall::new()),
        }
    }

    pub fn balance_key(&self, address: &H160) -> (Pubkey, Seed) {
        let mut balance = self.balance.borrow_mut();
        if let Some(cache) = balance.get(address) {
            return cache.clone();
        }

        let mut seed = Seed {
            items: vec![
                self.chain.clone(),
                ACCOUNT_SEED.to_vec(),
                address.as_bytes().to_vec(),
            ],
        };
        // TODO: move seed to find_pda
        let (key, bump_seed) = self.find_pda(&seed);
        seed.add(bump_seed);
        balance.insert(*address, (key, seed.clone()));

        (key, seed)
    }
    pub fn external_auth_seed(&self, address: &H160) -> Seed {
        Seed {
            items: vec![
                EXTERNAL_AUTHORITY.to_vec(),
                address.as_bytes().to_vec(),
            ],
        }
    }
    pub fn external_auth(&self, address: &H160) -> (Pubkey, Seed) {
        let mut cache = self.external_auth_cache.borrow_mut();
        if let Some(hit) = cache.get(address) {
            return hit.clone();
        }

        let mut seed = self.external_auth_seed(address);
        let (key, bump_seed) = self.find_pda(&seed);
        seed.add(bump_seed);
        cache.insert(*address, (key, seed.clone()));

        (key, seed)
    }
    /// Salt-derived variant of `external_auth`. Returns the 3-seed PDA
    /// `find_program_address([EXTERNAL_AUTHORITY, address, salt], program_id)`.
    /// Used by HelperProgram.pda_with_salt (read), create_mint_account /
    /// create_and_init_mint (signer for salt-derived mint account).
    /// Not cached — every call hits the find_program_address syscall.
    pub fn external_auth_with_salt(&self, address: &H160, salt: &[u8]) -> (Pubkey, Seed) {
        let mut seed = Seed {
            items: vec![
                EXTERNAL_AUTHORITY.to_vec(),
                address.as_bytes().to_vec(),
                salt.to_vec(),
            ],
        };
        let (key, bump_seed) = self.find_pda(&seed);
        seed.add(bump_seed);
        (key, seed)
    }
    pub fn tx_holder_key(&self, base: &Pubkey, index: u64) -> (Pubkey, Seed) {
        self.holder_key(base, index, TX_HOLDER_SEED)
    }

    pub fn state_holder_key(&self, base: &Pubkey, index: u64) -> (Pubkey, Seed) {
        self.holder_key(base, index, STATE_HOLDER_SEED)
    }
    fn holder_key(&self, base: &Pubkey, index: u64, salt: &[u8]) -> (Pubkey, Seed) {
        let mut seed = Seed {
            items: vec![
                self.chain.clone(),
                salt.to_vec(),
                base.as_ref().to_vec(),
                index.to_le_bytes().to_vec(),
            ],
        };
        let (key, bump_seed) = self.find_pda(&seed);
        seed.add(bump_seed);
        (key, seed)
    }

    pub fn storage_index(slot: &U256) -> ([u8; 32], u8) {
        let (index, sub_ix) = slot.div_mod(STORAGE_LEN.into());
        let mut index_be = [0_u8; 32];
        index.to_big_endian(&mut index_be);

        assert!(sub_ix <= u8::MAX.into());
        (index_be, sub_ix.as_usize() as u8)
    }

    pub fn storage_key(&self, base: &Pubkey, index_be: [u8; 32]) -> (Pubkey, Seed) {
        let mut storage = self.storage.borrow_mut();
        if let Some(cache) = storage.get(&(*base, index_be)) {
            return cache.clone();
        }

        let mut seed = Seed {
            items: vec![
                self.chain.clone(),
                base.as_ref().to_vec(),
                index_be.to_vec(),
            ],
        };
        let (key, bump_seed) = self.find_pda(&seed);
        seed.add(bump_seed);
        storage.insert((*base, index_be), (key, seed.clone()));

        (key, seed)
    }

    pub fn sol_wallet(&self) -> (Pubkey, Seed) {
        let mut seed = Seed {
            items: vec![self.chain.clone(), CONTRACT_SOL_WALLET.to_vec()],
        };
        let (key, bump_seed) = self.find_pda(&seed);
        seed.add(bump_seed);
        (key, seed)
    }
    pub fn treasure_wallet(&self, index: u64) -> (Pubkey, Seed) {
        let mut seed = Seed {
            items: vec![self.chain.clone(), TREASURE_SEED.to_vec(), index.to_le_bytes().to_vec()],
        };
        let (key, bump_seed) = self.find_pda(&seed);
        seed.add(bump_seed);
        (key, seed)
    }

    pub fn owner_info_key(&self) -> (Pubkey, Seed) {
        let mut seed = Seed {
            items: vec![OWNER_INFO.to_vec()],
        };
        let (key, bump_seed) = self.find_pda(&seed);
        seed.add(bump_seed);
        (key, seed)
    }

    pub fn bridge_processed_key(
        &self,
        source_chain: u64,
        source_tx_hash: &[u8; 32],
    ) -> (Pubkey, Seed) {
        let mut seed = Seed {
            items: vec![
                self.chain.clone(),
                BRIDGE_PROCESSED_SEED.to_vec(),
                source_chain.to_le_bytes().to_vec(),
                source_tx_hash.to_vec(),
            ],
        };
        let (key, bump_seed) = self.find_pda(&seed);
        seed.add(bump_seed);
        (key, seed)
    }

    pub fn init(info: &AccountInfo, typ: &AccountType) -> Result<()> {
        match typ {
            AccountType::New => unreachable!(),
            AccountType::Balance => AccountState::init(info),
            AccountType::Storage => Storage::init(info),
            AccountType::TxHolder => TxHolder::init(info),
            AccountType::StateHolder => StateHolder::init(info),
            AccountType::OwnerInfo => OwnerInfo::init(info),
            // Vestigial: nothing constructs AltSlots PDAs post-ALT-removal;
            // the variant byte stays for on-chain type-tagging only.
            AccountType::AltSlots => unreachable!(),
            AccountType::BridgeProcessed => BridgeProcessed::init(info),
            AccountType::StarterUnwrapDone => StarterUnwrapDone::init(info),
        }
    }
    pub fn empty_size(info: &AccountInfo, typ: &AccountType) -> usize {
        match typ {
            AccountType::New => unreachable!(),
            AccountType::Balance => AccountState::offset(info) + AccountState::size(info),
            AccountType::Storage => Storage::offset(info) + Storage::size(info),
            AccountType::TxHolder => TxHolder::offset(info) + TxHolder::size(info),
            AccountType::StateHolder => StateHolder::offset(info) + StateHolder::size(info),
            AccountType::OwnerInfo => OwnerInfo::offset(info),
            AccountType::AltSlots => unreachable!(),
            AccountType::BridgeProcessed => BridgeProcessed::offset(info) + BridgeProcessed::size(info),
            AccountType::StarterUnwrapDone => StarterUnwrapDone::offset(info) + StarterUnwrapDone::size(info),
        }
    }
    pub fn serialize(&self, into: &mut &mut [u8]) -> Result<()> {
        let balance = self.balance.borrow();
        let storage = self.storage.borrow();
        let ro_lock = self.ro_lock.borrow();

        balance.serialize(into)?;
        storage.serialize(into)?;
        ro_lock.serialize(into)?;

        Ok(())
    }
    pub fn deserialize(&self, from: &mut &[u8]) -> Result<()> {
        *self.balance.borrow_mut() = BorshDeserialize::deserialize(from)?;
        *self.storage.borrow_mut() = BorshDeserialize::deserialize(from)?;
        *self.ro_lock.borrow_mut() = BorshDeserialize::deserialize(from)?;

        Ok(())
    }
    pub fn gas_estimate_key(&self) -> Pubkey {
        let seed = Seed {
            items: vec![ESTIMATE_AUTHORITY.to_vec()],
        };
        let (key, _) = self.find_pda(&seed);
        key
    }

    pub fn gas_estimate_key_assert(&self, key: &Pubkey,) -> Result<()> {
        let sample = self.gas_estimate_key();

        if key !=  &sample {
            return Err(InvalidGasEstimateAuthority)
        }
        
        Ok(())
    }

    fn find_pda(&self, seed: &Seed) -> (Pubkey, u8) {
        Pubkey::find_program_address(seed.cast().as_slice(), self.program_id)
    }

    pub fn find_program_address(&self, seeds: &[&[u8]], program_id: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(seeds, program_id)
    }

    #[cfg(not(target_os = "solana"))]
    pub fn reset(&self) {
        self.balance.borrow_mut().clear();
        self.storage.borrow_mut().clear();
        self.ro_lock.borrow_mut().clear();
        self.external_auth_cache.borrow_mut().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `external_auth` returns the canonical PDA and memoizes it (cache read +
    /// cleared by `reset`).
    #[test]
    fn external_auth_returns_canonical_pda_and_caches() {
        let program_id = Pubkey::new_unique();
        let pda = Pda::new_(&program_id, 1u64);
        let addr = H160::from_slice(&[0xAB; 20]);

        // Behavior-preserving: same key + canonical bump as a direct derivation.
        let (key, seed) = pda.external_auth(&addr);
        let expected = Pubkey::find_program_address(&[EXTERNAL_AUTHORITY, addr.as_bytes()], &program_id);
        assert_eq!(key, expected.0, "external_auth must return the canonical PDA key");
        assert_eq!(
            seed.items.last().map(|v| v.as_slice()),
            Some([expected.1].as_slice()),
            "seed must carry the canonical bump"
        );

        // First call populated the cache.
        assert!(
            pda.external_auth_cache.borrow().contains_key(&addr),
            "external_auth must populate the cache on a miss"
        );

        // Cache is read, not re-derived: poison the entry, expect the sentinel back.
        let sentinel = Pubkey::new_unique();
        pda.external_auth_cache
            .borrow_mut()
            .insert(addr, (sentinel, seed.clone()));
        let (key2, _) = pda.external_auth(&addr);
        assert_eq!(key2, sentinel, "external_auth must read from cache, not re-derive");

        // reset() clears it (transient).
        pda.reset();
        assert!(
            pda.external_auth_cache.borrow().is_empty(),
            "reset must clear the external_auth cache"
        );
    }
}
