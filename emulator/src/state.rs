use {
    crate::stubs::Stubs,
    mollusk::Mollusk,
    rome_evm::{
        assert::asserts,
        error::{Result, RomeProgramError::*},
        state::{base::Base, pda::Pda}, AccountType,
        Data, OwnerInfo, H160, U256, state::aux::Account, origin::Origin,
    },
    solana_program::{
        account_info::IntoAccountInfo, msg, pubkey::Pubkey, rent::Rent,
        sysvar::{self, Sysvar}, program_stubs::set_syscall_stubs, instruction::Instruction,
    },
    solana_system_interface::{
        instruction::{create_account, transfer, assign,},
        program as system_program,
    },
    solana_bincode::limited_deserialize,
    solana_loader_v3_interface::state::UpgradeableLoaderState,
    solana_sdk_ids::{
        bpf_loader_upgradeable,
    },
    solana_program::sysvar::instructions::{
        BorrowedAccountMeta, BorrowedInstruction,
    },
    std::{
        cell::RefCell, collections::{BTreeMap, HashMap,}, ops::Deref, sync::Arc,
    },
};
use crate::account_storage::AccountStorage;

#[derive(Clone, Debug, Default)]
pub struct Item {
    pub account: Account,
    pub address: Option<H160>,
}
pub type Slots = BTreeMap<U256, bool>;
pub type Bind = (Pubkey, Account);

impl<'a> Deref for State<'a> {
    type Target = Base<'a>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
pub struct State<'a> {
    pub base: Base<'a>,
    pub client: Arc<dyn AccountStorage>,
    pub accounts: RefCell<BTreeMap<Pubkey, Item>>,
    pub original: RefCell<HashMap<Pubkey, solana_account::Account>>,
    pub upgradeable_elf: RefCell<HashMap<Pubkey, solana_account::Account>>,
    pub storage: RefCell<BTreeMap<H160, Slots>>,
    pub signer: Option<Pubkey>,
    pub ed25519_ix: Option<Instruction>,
}

impl<'a> State<'a> {
    pub fn new(
        program_id: &'a Pubkey,
        signer: Option<Pubkey>,
        client: Arc<dyn AccountStorage>,
        chain: u64,
    ) -> Result<Self> {
        Self::new_ed25519(program_id, signer, client, chain, None)
    }
    pub fn new_ed25519(
        program_id: &'a Pubkey,
        signer: Option<Pubkey>,
        client: Arc<dyn AccountStorage>,
        chain: u64,
        ed25519_ix: Option<Instruction>,
    ) -> Result<Self> {
        let mut state = Self::new_unchecked(program_id, signer, client, chain)?;
        // 1. needs for transmit_tx,
        // 2. reduces the number of failures if tx depends on timestamp
        let _ = state.info_sys(&system_program::ID)?;

        if let Some(ix) = ed25519_ix.as_ref() {
            let bind = compose_sysvar(vec![ix]);
            state.insert(bind, None);
        }

        let mut bind = state.info_owner_reg(false)?;
        let info = bind.into_account_info();
        state.base.owner_info = Some(OwnerInfo::owner_info(&info, chain)?);
        state.ed25519_ix = ed25519_ix;

        Ok(state)
    }
    pub fn new_unchecked(
        program_id: &'a Pubkey,
        signer: Option<Pubkey>,
        client: Arc<dyn AccountStorage>,
        chain: u64,
    ) -> Result<Self> {
        asserts();
        let (stubs, sysvars) = Stubs::from_chain_with_accounts(Arc::clone(&client))?;
        set_syscall_stubs(stubs);

        let state = Self {
            base: Base::new(program_id, chain, None),
            client,
            accounts: RefCell::new(BTreeMap::new()),
            original: RefCell::new(HashMap::new()),
            storage: RefCell::new(BTreeMap::new()),
            upgradeable_elf: RefCell::new(HashMap::new()),
            signer,
            ed25519_ix: None,
        };

        // Carry the fetched clock/rent accounts in `original` so the
        // emulation result hands them to downstream consumers. The iterative
        // confirm-path (rome-sdk) rebuilds an `AccountStorage` from `original`
        // and re-runs `Stubs::from_chain`; without these it finds nothing and
        // DEFAULTS the sysvars (slot 0 / default `Rent`), installing those
        // process-globally — OZ `CheckpointUnorderedInsertion` (block.number 0)
        // and swap "PDA doesn't have enough lamports for rent".
        {
            let mut original = state.original.borrow_mut();
            for (key, acc) in sysvars {
                original.insert(key, acc);
            }
        }

        if let Some(signer) = signer {
            let bind = state.info_sys(&signer).map_err(|_| InvalidSigner)?;
            state.set_signer(&bind.0);
        }

        Ok(state)
    }

    pub fn info_addr_opt(&self, address: &H160) -> Result<Option<Bind>> {
        let key = self.pda.balance_key(address).0;
        self.info_pda_opt(&key, AccountType::Balance, Some(*address))
    }
    pub fn info_addr(&self, address: &H160, or_create: bool) -> Result<Bind> {
        let key = self.pda.balance_key(address).0;
        self.info_pda(&key, AccountType::Balance, Some(*address), or_create, true)
    }
    pub fn info_slot_opt(
        &self,
        address: &H160,
        slot: &U256,
    ) -> Result<(Option<Bind>, u8)> {
        let (key, _, sub_ix) = self.slot_to_key(address, slot);
        let pda = self.info_pda_opt(&key, AccountType::Storage, Some(*address))?;
        self.update_slots(address, slot, false);
        Ok((pda, sub_ix))
    }
    pub fn info_slot(&self, address: &H160, slot: &U256, or_create: bool) -> Result<(Bind, u8)> {
        let (key, _, sub_ix) = self.slot_to_key(address, slot);
        let bind = self.info_pda(&key, AccountType::Storage, Some(*address), or_create, true)?;
        self.update_slots(address, slot, or_create);

        Ok((bind, sub_ix))
    }
    pub fn info_tx_holder(&self, index: u64, or_create: bool) -> Result<Bind> {
        let signer = self.signer.expect("signer expected");
        let (key, _) = self.pda.tx_holder_key(&signer, index);
        self.info_pda(&key, AccountType::TxHolder, None, or_create, true)
    }
    pub fn info_tx_holder_by_key(&self, key: &Pubkey) -> Result<Bind> {
        self.info_pda(key, AccountType::TxHolder, None, false, true)
    }
    pub fn info_tx_holder_opt(&self, index: u64) -> Result<Option<Bind>> {
        let signer = self.signer.expect("signer expected");
        let (key, _) = self.pda.tx_holder_key(&signer, index);
        self.info_pda_opt(&key, AccountType::TxHolder, None)
    }
    pub fn info_state_holder_opt(&self, index: u64) -> Result<Option<Bind>> {
        let signer = self.signer.expect("signer expected");
        let (key, _) = self.pda.state_holder_key(&signer, index);
        self.info_pda_opt(&key, AccountType::StateHolder, None)
    }
    pub fn info_state_holder(&self, index: u64, or_create: bool) -> Result<Bind> {
        let signer = self.signer.expect("signer expected");
        let (key, _) = self.pda.state_holder_key(&signer, index);
        self.info_pda(&key, AccountType::StateHolder, None, or_create, true)
    }
    pub fn info_state_holder_by_key(&self, key: &Pubkey) -> Result<Bind> {
        self.info_pda(key, AccountType::StateHolder, None, false, true)
    }
    pub fn info_owner_reg(&self, or_create: bool) -> Result<Bind> {
        let (key, _) = self.pda.owner_info_key();
        self.info_pda(&key, AccountType::OwnerInfo, None, or_create, true)
    }
    pub fn info_pda_opt(
        &self,
        key: &Pubkey,
        typ: AccountType,
        addr: Option<H160>,
    ) -> Result<Option<Bind>> {
        let mut bind = self.info_pda(&key, typ.clone(), addr,false, false)?;

        if bind.1.owner == system_program::ID {
            assert!(bind.1.data.is_empty());
            return Ok(None)
        }

        let  info = bind.into_account_info();
        AccountType::is_ok(&info, typ, &self.program_id)?;
        Ok(Some(bind))
    }
    // TODO: the missing account must be included in the transaction accounts
    pub fn info_pda(
        &self,
        key: &Pubkey,
        typ: AccountType,
        addr: Option<H160>,
        or_create: bool,
        to_check: bool,
    ) -> Result<Bind> {
        let bind = self.info_external(key, or_create)?;

        if system_program::check_id(&bind.1.owner) && or_create {
            let len = State::pda_size(&typ);
            self.create_pda(&bind, len, typ.is_paid(), &self.program_id)?;
            self.set_addr(&key, addr);

            let mut accs = self.accounts.borrow_mut();
            let item = accs.get_mut(&key).unwrap();
            let info = (key, &mut item.account).into_account_info();
            Pda::init(&info, &typ)?;
        }

        let mut bind = self.info_external(key, or_create)?;
        if to_check {
            let  info = bind.into_account_info();
            AccountType::is_ok(&info, typ, self.program_id)?;
        }

        Ok(bind)
    }
    pub fn info_external(
        &self,
        key: &Pubkey,
        writable: bool,
    ) -> Result<Bind> {
        if let Some(mut bind) = self.load(key, None, writable)? {
            // TODO: move update_writable() to load()
            self.update_writable(&bind.0, writable);
            bind.1.writable |= writable;
            Ok(bind)
        } else {
            let new = Account {
                lamports: 0,
                data: vec![],
                owner: system_program::ID,
                executable: false,
                writable,
                signer: false,
            };

            let bind = (*key, new);
            self.insert(bind, None);
            self.info_external(key, writable)
        }
    }
    pub fn info_sol_wallet(&self, or_create: bool) -> Result<Bind>{
        let (key, _) = self.pda.sol_wallet();
        self
            .info_wallet(&key, &system_program::ID, or_create)?
            .ok_or(PdaUntypedAccountNotCreated(key))
    }
    pub fn info_treasure(&self, index: u64, or_create: bool) -> Result<Bind>{
        let (key, _) = self.pda.treasure_wallet(index);
        self
            .info_wallet(&key, &system_program::ID, or_create)?
            .ok_or(PdaUntypedAccountNotCreated(key))
    }
    pub fn info_wallet(&self, key :&Pubkey, owner: &Pubkey, or_create: bool) -> Result<Option<Bind>>{
        let bind = self.info_external(key, true)?;

        if bind.1.lamports == 0 {
            if !or_create {
                return Ok(None)
            }
            self.create_pda(&bind, 0, false, owner)?;
        }

        let bind = self.info_external(key, true)?;
        self.update(bind.clone());

        assert_eq!(bind.1.owner, *owner);
        assert!(bind.1.lamports > 0);
        assert!(bind.1.data.is_empty());

        Ok(Some(bind))
    }
    pub fn info_program(
        &self,
        key: &Pubkey,
    ) -> Result<Bind> {
        if !self.accounts.borrow_mut().contains_key(&key) {
            let new = Account {
                executable: true,
                ..Default::default()
            };

            let bind = (*key, new);
            self.insert(bind, None);
        }

        self.info_external(key, false)
    }
    fn update_slots(&self, address: &H160, slot: &U256, writable: bool) {
        let mut storage = self.storage.borrow_mut();

        storage
            .entry(*address)
            .and_modify(|slots| {
                slots
                    .entry(*slot)
                    .and_modify(|rw| *rw |= writable)
                    .or_insert(writable);
            })
            .or_insert(BTreeMap::from([(*slot, writable)]));
    }
    pub fn info_sys(&self, key: &Pubkey) -> Result<Bind> {
        self.load(key, None, false)?.ok_or(AccountNotFound(*key))
    }
    pub fn load(
        &self,
        key: &Pubkey,
        address: Option<H160>,
        writable: bool,
    ) -> Result<Option<Bind>> {

        if let Some(item) = self.accounts.borrow().get(key) {
            let bind = (*key, item.account.clone());
            return Ok(Some(bind))
        }

        // The Instructions sysvar is not a stored account: the runtime builds it
        // per transaction, so the RPC client has nothing to return for it.
        // Without this, a CPI target that lists it (klend deposit/redeem, any
        // program that inspects its caller) sees a system-owned empty account
        // and fails with InvalidAccountOwner. Approximation: one top-level
        // instruction to this program at index 0 (program id only — the real tx
        // has the Rome ix's accounts/data and may carry other top-level ixs).
        // Faithful for checks on the outermost program id. The ed25519 path
        // inserts its own sysvar in `new_ed25519` and returns from the cache above.
        if *key == sysvar::instructions::ID {
            let rome_ix = Instruction::new_with_bytes(*self.program_id, &[], vec![]);
            let (_, mut acc) = compose_sysvar(vec![&rome_ix]);
            acc.writable = writable;
            self.insert((*key, acc), address);
            return self.load(key, address, writable)
        }

        self
            .client
            .get_account(key)?
            .map_or_else(
                || Ok(None),
                |sdk| {
                    let acc = Account {
                        lamports: sdk.lamports,
                        data: sdk.data,
                        owner: sdk.owner,
                        executable: sdk.executable,
                        writable,
                        signer: false,
                    };
                    let bind = (*key, acc);
                    self.insert(bind, address);
                    self.load(key, address, writable)
            })
   }
    pub fn load_upgradeable_elf(&self, key: &Pubkey) -> Result<()> {
        if self.upgradeable_elf.borrow().get(key).is_some() {
            return Ok(())
        }

        let acc = self
            .client
            .get_account(key)?
            .ok_or(ProgramAccountNotFound(*key))?;
        let mut elfs = self.upgradeable_elf.borrow_mut();
        elfs.insert(*key, acc.clone());

        Ok(())
   }
    pub fn update(&self, bind: Bind) {
        let mut accounts = self.accounts.borrow_mut();
        let item = accounts.get_mut(&bind.0).unwrap();
        item.account = bind.1;
        item.account.writable = true;
    }
    pub fn set_signer(&self, key: &Pubkey) {
        let mut accounts = self.accounts.borrow_mut();
        let item = accounts.get_mut(key).unwrap();
        item.account.signer = true;
        item.account.writable = true;
    }
    pub fn update_writable(&self, key: &Pubkey, writeable: bool) {
        let mut accounts = self.accounts.borrow_mut();
        let item = accounts.get_mut(key).unwrap();
        item.account.writable |= writeable;
    }
    pub fn insert(&self, bind: Bind, address: Option<H160>) {
        let item = Item {
            account: bind.1,
            address,
        };
        let mut accounts = self.accounts.borrow_mut();
        let mut original = self.original.borrow_mut();

        original.insert(bind.0, item.account.clone().into());
        if let Some (item) = accounts.insert(bind.0, item) {
            assert_eq!(item.account.lamports, 0);
            assert!(item.account.data.is_empty());
            assert_eq!(item.account.owner, system_program::ID);
            assert_eq!(item.account.writable, false);
        }

    }

    pub fn set_addr(&self, key: &Pubkey, address: Option<H160>) {
        if let Some(address) = address {
            let mut accounts = self.accounts.borrow_mut();
            let item = accounts.get_mut(key).unwrap();
            item.address = Some(address)
        }
    }

    pub fn inc_space_counter(&self, alloc: usize, dealloc: usize, refund_to_signer: bool) -> Result<()> {
        Base::inc_alloc(&self.base, alloc)?;
        Base::inc_dealloc(&self.base, dealloc)?;

        if refund_to_signer {
            Base::inc_alloc_payed(&self.base, alloc)?;
            Base::inc_dealloc_payed(&self.base, dealloc)?;
        }

        Ok(())
    }

    pub fn realloc(&self, key: &Pubkey, len: usize, refund_to_signer: bool) -> Result<()> {
        let mut bind = self.info_sys(key)?;
        assert_eq!(&bind.1.owner, self.program_id);

        if bind.1.data.len() == len {
            return Ok(());
        }

        let alloc = len.saturating_sub(bind.1.data.len());
        let dealloc = bind.1.data.len().saturating_sub(len);

        bind.1.data.resize(len, 0);
        msg!("resized len: {}", bind.1.data.len());

        self.inc_space_counter(alloc, dealloc, refund_to_signer)?;

        let lamports = bind.1.lamports;
        let rent = Rent::get()?.minimum_balance(bind.1.data.len());
        self.update(bind);

        if rent > lamports {
            let ix = transfer(&self.signer(), key, rent - lamports);
            self.invoke_signed(&ix, vec![], refund_to_signer)?;
        }

        Ok(())
    }
    fn pda_size(typ: &AccountType) -> usize {
        let mut def = def_bind();
        let info = def.into_account_info();
        Pda::empty_size(&info, typ)
    }
    pub fn create_pda(
        &self,
        bind: &Bind,
        len: usize,
        refund_to_signer: bool,
        owner: &Pubkey,
    ) -> Result<()> {
        assert_eq!(bind.1.data.len(), 0);
        assert!(bind.1.writable);
        assert_eq!(bind.1.owner, system_program::ID);

        if bind.1.lamports > 0 {
            let ix = assign(&bind.0, owner);
            self.invoke_signed(&ix, vec![], refund_to_signer)?;
            self.realloc(&bind.0, len, refund_to_signer)?; // TODO do contract-appropriate refactoring
        } else {
            let rent = Rent::get()?.minimum_balance(len);

            let ix = create_account(
                &self.signer(),
                &bind.0,
                rent,
                len as u64,
                owner,
            );

            self.invoke_signed(&ix, vec![], refund_to_signer)?;
        }

        Ok(())
    }

    pub fn remove_pda(&self, mut bind: Bind) -> Result<()> {
        {
            let info = bind.into_account_info();
            let typ = AccountType::from_account(&info)?;
            match *typ {
                AccountType::AltSlots | AccountType::TxHolder | AccountType::StateHolder => {},
                _ => unreachable!(),
            }
        }

        bind.1.data.clear();

        let mut signer = self.info_sys(&self.signer.unwrap())?;
        let lamports = signer.1.lamports.checked_add(bind.1.lamports)
            .ok_or(CalculationOverflow)?;

        signer.1.lamports = lamports;
        bind.1.lamports = 0;
        bind.1.owner = system_program::ID;

        self.update(signer);
        self.update(bind);

        Ok(())
    }
    pub fn lamports(&self, key: &Pubkey) -> Result<u64>{
        let accs = self.accounts.borrow();

        let lamports = accs
            .get(&key)
            .ok_or(AccountNotFound(*key))?
            .account
            .lamports;
        Ok(lamports)
    }

    pub fn data_len(&self, keys: &Vec<Pubkey>) -> Result<Vec<usize>> {
        let accs = self.accounts.borrow();
        let vec = keys
            .iter()
            .map(|k| accs.get(k).ok_or(AccountNotFound(*k)))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .map(|a| a.account.data.len())
            .collect::<Vec<_>>();

        Ok(vec)
    }

    pub fn update_accs(&self, resulting_accs: Vec<(Pubkey, solana_account::Account)>) {
        let mut accs = self.accounts.borrow_mut();

        for (key, sim) in resulting_accs.into_iter() {
            // the following accounts have empty data or incorrect data size in mollusk account store
            if Mollusk::builtin(&key) {
                continue
            }

            if let Some(item) = accs.get_mut(&key) {
                if item.account.executable {
                    continue
                }
                item.account.data = sim.data.clone();
                item.account.lamports = sim.lamports;
                item.account.owner = sim.owner;
            }
        }
    }

    pub fn fetch_accs(&self, ix: &Instruction) -> Result<Vec<Pubkey>> {
        let mut vec = ix
            .accounts
            .iter()
            .map(|a| (a.pubkey, a.is_writable))
            .collect::<Vec<_>>();

        let return_val =  vec
            .iter()
            .map(|(k, _)| *k)
            .collect::<Vec<_>>();

        vec.push((ix.program_id, false));

        for (key, writable) in vec.iter() {
            let (_, acc) = self.info_external(key, *writable)?;

            if acc.executable && acc.owner == bpf_loader_upgradeable::id() {
                match limited_deserialize(&acc.data, u64::MAX)? {
                    UpgradeableLoaderState::Program { programdata_address: key_ } => {
                        self.load_upgradeable_elf(&key_)?;
                    },
                    _ => {},
                }
            }
        }

        Ok(return_val)
    }
}

pub fn ix_store(ix: &Instruction, accs: &BTreeMap<Pubkey, Item>) -> HashMap<Pubkey, solana_account::Account> {
    let mut vec = ix.accounts.iter().map(|a| a.pubkey).collect::<Vec<_>>();
    vec.push(ix.program_id);

    accs
        .iter()
        .filter(|(key, _)| {
            vec.iter().any(|&a| a == **key)
        })
        .map(|(key, item)| {
            let acc = solana_account::Account {
                lamports: item.account.lamports,
                data: item.account.data.clone(),
                owner: item.account.owner,
                executable: item.account.executable,
                rent_epoch: 0,
            };
            (*key, acc)
        })
        .collect::<HashMap<Pubkey, solana_account::Account>>()
}

pub fn def_bind() -> Bind {
    (Pubkey::default(), Account::default())
}

fn convert_to_borrowed<'a>(ix: &'a Instruction) -> BorrowedInstruction<'a> {
    let accs: Vec<BorrowedAccountMeta<'a>> = ix
        .accounts
        .iter()
        .map(|meta| BorrowedAccountMeta {
            pubkey: &meta.pubkey,
            is_signer: meta.is_signer,
            is_writable: meta.is_writable,
        })
        .collect();

    BorrowedInstruction {
        program_id: &ix.program_id,
        accounts: accs,
        data: &ix.data,
    }
}

pub fn compose_sysvar(ixs: Vec<&Instruction>) -> (Pubkey, Account){
    let borrowed_ixs = ixs
        .iter()
        .map(|&ix| convert_to_borrowed(ix))
        .collect::<Vec<BorrowedInstruction>>()
        ;
    let sysvar_data = sysvar::instructions::construct_instructions_data(&borrowed_ixs);

    let acc = Account {
        lamports: 1,
        data: sysvar_data,
        owner: sysvar::id(),
        executable: false,
        writable: false,
        signer: false,
    };
    (sysvar::instructions::ID, acc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::instruction::AccountMeta;

    fn item_with_executable(executable: bool) -> Item {
        Item {
            account: Account {
                lamports: 1,
                data: vec![],
                owner: Pubkey::default(),
                executable,
                writable: false,
                signer: false,
            },
            address: None,
        }
    }

    /// `ix_store` must include the instruction's `program_id` in its returned
    /// store, even though `ix.program_id` is a separate field and is NOT
    /// listed in `ix.accounts`. Mollusk's `process_instruction` derives
    /// `message.account_keys()` from BOTH the program id and the account
    /// list; the program account must be in the provided slice for
    /// `compile_accounts` to find it. `mollusk.rs::load_elf` also reads from
    /// this store directly to build the runtime program cache, returning
    /// `ProgramAccountNotFound` if the program account is missing.
    ///
    /// Pre-fix, the filter `ix.accounts.iter().any(|meta| meta.pubkey == key)`
    /// dropped `ix.program_id` from the store, causing
    /// `ProgramAccountNotFound(<program_id>)` for every helper-program CPI
    /// whose program id wasn't also listed as a regular account
    /// (e.g. `HelperProgram.create_ata` → ATA program).
    #[test]
    fn ix_store_includes_program_id_when_not_in_accounts() {
        let program_id = Pubkey::new_unique();
        let referenced_account = Pubkey::new_unique();
        let unrelated_cached_account = Pubkey::new_unique();

        let mut accs: BTreeMap<Pubkey, Item> = BTreeMap::new();
        accs.insert(program_id, item_with_executable(true));
        accs.insert(referenced_account, item_with_executable(false));
        accs.insert(unrelated_cached_account, item_with_executable(false));

        let ix = Instruction {
            program_id,
            accounts: vec![AccountMeta::new(referenced_account, false)],
            data: vec![],
        };

        let store = ix_store(&ix, &accs);

        assert!(
            store.contains_key(&program_id),
            "ix.program_id ({}) must be in store; pre-fix the filter dropped it, \
             surfacing as ProgramAccountNotFound from mollusk.rs::load_elf",
            program_id,
        );
        assert!(
            store.contains_key(&referenced_account),
            "accounts in ix.accounts must still be in store"
        );
    }

    fn item_with_owner_data(owner: Pubkey, data: Vec<u8>, executable: bool) -> Item {
        Item {
            account: Account {
                lamports: 1_141_440,
                data,
                owner,
                executable,
                writable: false,
                signer: false,
            },
            address: None,
        }
    }

    /// Answers nothing: the Instructions sysvar is never a stored account, so a
    /// real RPC client returns None for it too.
    struct EmptyStorage;

    impl AccountStorage for EmptyStorage {
        fn get_account(&self, _key: &Pubkey) -> Result<Option<solana_account::Account>> {
            Ok(None)
        }
        fn get_multiple_accounts(&self, keys: &[Pubkey]) -> Result<Vec<Option<solana_account::Account>>> {
            keys.iter().map(|k| self.get_account(k)).collect()
        }
    }

    /// A CPI target that lists the Instructions sysvar (klend deposit/redeem)
    /// must see a sysvar-owned account describing the outer Rome instruction,
    /// not a system-owned placeholder (InvalidAccountOwner in mollusk).
    #[test]
    fn instructions_sysvar_is_composed_when_absent_from_client() {
        let program_id = Pubkey::new_unique();
        let storage: Arc<dyn AccountStorage> = Arc::new(EmptyStorage);
        let state = State::new_unchecked(&program_id, None, storage, 1).unwrap();

        let (key, mut acc) = state.info_external(&sysvar::instructions::ID, false).unwrap();
        assert_eq!(acc.owner, sysvar::id());

        let mut lamports = acc.lamports;
        let info = solana_program::account_info::AccountInfo::new(
            &key, false, false, &mut lamports, &mut acc.data, &acc.owner, false,
        );
        let ix = sysvar::instructions::load_instruction_at_checked(0, &info).unwrap();
        assert_eq!(ix.program_id, program_id);
        assert_eq!(sysvar::instructions::load_current_index_checked(&info).unwrap(), 0);
    }

    /// A second load (writable or not) hits the cache and must not re-insert.
    #[test]
    fn instructions_sysvar_reload_is_cached() {
        let program_id = Pubkey::new_unique();
        let storage: Arc<dyn AccountStorage> = Arc::new(EmptyStorage);
        let state = State::new_unchecked(&program_id, None, storage, 1).unwrap();

        let (_, first) = state.info_external(&sysvar::instructions::ID, false).unwrap();
        let (_, second) = state.info_external(&sysvar::instructions::ID, true).unwrap();
        assert_eq!(first.data, second.data);
        assert_eq!(second.owner, sysvar::id());
    }
}
