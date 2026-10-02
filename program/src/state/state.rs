use {
    super::{
        base::Base,
        pda::{Pda, Seed},
        origin::Origin,
    },
    crate::{
        error::RomeProgramError::*, error::*, AccountType, OwnerInfo, Data, CHAIN_ID,
    },
    evm::{H160, U256},
    solana_program::{
        account_info::AccountInfo, pubkey::Pubkey, rent::Rent, sysvar::Sysvar,
        instruction::Instruction, log::sol_log_data,
    },
    solana_system_interface::{
        program as system_program,
        instruction::{
            create_account, transfer, assign, allocate,
        }
    },
    std::{iter::FromIterator, ops::Deref},
};
use super::fast_hash::FastMap;

pub struct State<'a> {
    all: FastMap<Pubkey, &'a AccountInfo<'a>>,
    pub base: Base<'a>,
    pub signer: &'a AccountInfo<'a>,
}

impl<'a> Deref for State<'a> {
    type Target = Base<'a>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

#[allow(dead_code)]
impl<'a> State<'a> {
    pub fn new(
        program_id: &'a Pubkey,
        accounts: &'a [AccountInfo<'a>],
        chain: u64,
    ) -> Result<Self> {
        let mut state = Self::new_unchecked(program_id, accounts, chain)?;
        let info = state.info_owner_reg(false)?;
        state.base.owner_info = Some(OwnerInfo::owner_info(info, chain)?);

        Ok(state)
    }
    pub fn new_unchecked(
        program_id: &'a Pubkey,
        accounts: &'a [AccountInfo<'a>],
        chain: u64,
    ) -> Result<Self> {
        sol_log_data(&[CHAIN_ID, &chain.to_le_bytes()]);

        let keys = accounts.iter().map(|a| *a.key);
        let all = FastMap::from_iter(keys.zip(accounts.iter()));
        let signer = Self::signer(&all)?; 

        Ok(Self {
            all,
            base: Base::new(program_id, chain, None),
            signer,
        })
    }
    pub fn info_addr_opt(&self, address: &H160) -> Result<Option<&'a AccountInfo<'a>>> {
        let (key, seed) = self.pda.balance_key(address);
        self.info_pda_opt(&key, &seed, AccountType::Balance)
    }
    pub fn info_addr(&self, address: &H160, or_create: bool) -> Result<&'a AccountInfo<'a>> {
        let (key, seed) = self.pda.balance_key(address);
        self.info_pda(&key, &seed, AccountType::Balance, or_create, true)
    }
    pub fn info_slot_opt(
        &self,
        address: &H160,
        slot: &U256,
    ) -> Result<(Option<&'a AccountInfo<'a>>, u8)> {
        let (key, seed, sub_ix) = self.slot_to_key(address, slot);
        let pda = self.info_pda_opt(&key, &seed, AccountType::Storage)?;
        Ok((pda, sub_ix))
    }
    pub fn info_slot(
        &self,
        address: &H160,
        slot: &U256,
        or_create: bool,
    ) -> Result<(&'a AccountInfo<'a>, u8)> {
        let (key, seed, sub_ix) = self.slot_to_key(address, slot);
        let info = self.info_pda(&key, &seed, AccountType::Storage, or_create, true)?;

        Ok((info, sub_ix))
    }
    pub fn info_tx_holder(&self, index: u64, or_create: bool) -> Result<&'a AccountInfo<'a>> {
        let (key, seed) = self.pda.tx_holder_key(self.signer.key, index);
        self.info_pda(&key, &seed, AccountType::TxHolder, or_create, true)
    }
    pub fn info_tx_holder_opt(&self, index: u64) -> Result<Option<&'a AccountInfo<'a>>> {
        let (key, seed) = self.pda.tx_holder_key(self.signer.key, index);
        self.info_pda_opt(&key, &seed, AccountType::TxHolder)
    }
    pub fn info_state_holder(&self, index: u64, or_create: bool) -> Result<&'a AccountInfo<'a>> {
        let (key, seed) = self.pda.state_holder_key(self.signer.key, index);
        self.info_pda(&key, &seed, AccountType::StateHolder, or_create, true)
    }
    pub fn info_state_holder_opt(&self, index: u64) -> Result<Option<&'a AccountInfo<'a>>> {
        let (key, seed) = self.pda.state_holder_key(self.signer.key, index);
        self.info_pda_opt(&key, &seed, AccountType::StateHolder)
    }
    pub fn info_owner_reg(&self, or_create: bool) -> Result<&'a AccountInfo<'a>> {
        let (key, seed) = self.pda.owner_info_key();
        self.info_pda(&key, &seed, AccountType::OwnerInfo, or_create, true)
    }
    pub fn info_bridge_processed(
        &self,
        source_chain: u64,
        source_tx_hash: &[u8; 32],
        or_create: bool,
    ) -> Result<&'a AccountInfo<'a>> {
        let (key, seed) = self.pda.bridge_processed_key(source_chain, source_tx_hash);
        self.info_pda(&key, &seed, AccountType::BridgeProcessed, or_create, true)
    }
    pub fn info_pda_opt(
        &self,
        key: &Pubkey,
        seed: &Seed,
        typ: AccountType
    ) -> Result<Option<&'a AccountInfo<'a>>> {
        let pda = self.info_pda(&key, &seed, typ.clone(), false, false)?;

        if *pda.owner == system_program::ID {
            assert!(pda.data_is_empty());
            return Ok(None)
        }

        AccountType::is_ok(&pda, typ, &self.program_id)?;
        Ok(Some(pda))
    }
    pub fn info_pda(
        &self,
        key: &Pubkey,
        seed: &Seed,
        typ: AccountType,
        or_create: bool,
        to_check: bool,
    ) -> Result<&'a AccountInfo<'a>> {
        let pda = self
            .all
            .get(key)
            .cloned()
            .ok_or(PdaAccountNotFound(*key, typ.clone()))?;

        if system_program::check_id(pda.owner) && or_create {
            let len = Pda::empty_size(pda, &typ);
            self.create_pda(pda, seed, len, typ.is_paid(), &self.program_id)?;
            Pda::init(pda, &typ)?;
        }

        if to_check {
            AccountType::is_ok(pda, typ, self.program_id)?;
        }
        Ok(pda)
    }
    pub fn info_sol_wallet(&self, or_create: bool) -> Result<&'a AccountInfo<'a>>{
        let (key, seed) = self.pda.sol_wallet();
        self
            .info_wallet(&key, &seed, &system_program::ID, or_create)?
            .ok_or(PdaUntypedAccountNotCreated(key))
    }
    pub fn info_treasure(&self, index: u64,  or_create: bool) -> Result<&'a AccountInfo<'a>>{
        let (key, seed) = self.pda.treasure_wallet(index);
        self
            .info_wallet(&key, &seed, &system_program::ID, or_create)?
            .ok_or(PdaUntypedAccountNotCreated(key))
    }
    pub fn info_wallet(&self, key :&Pubkey, seed: &Seed, owner: &Pubkey, or_create: bool) -> Result<Option<&'a AccountInfo<'a>>>{
        let pda = self
            .all
            .get(key)
            .cloned()
            .ok_or(PdaUntypedAccountNotFound(*key))?;

        if pda.lamports() == 0 {
            if !or_create {
                return Ok(None)
            }
            self.create_pda(pda, seed, 0, false, owner)?;
        }

        assert_eq!(*pda.owner, *owner);
        assert!(pda.lamports() > 0);
        assert!(pda.data_is_empty()); // SOL wallet cannot hold data
        Ok(Some(pda))
    }

    fn signer(map: &FastMap<Pubkey, &'a AccountInfo<'a>>) -> Result<&'a AccountInfo<'a>> {
        let mut signer = None;

        for &info in map.values() {
            if info.is_signer && info.is_writable {
                if signer.is_some() {
                    return Err(InvalidSigner);
                }
                signer = Some(info)
            }
        }

        signer.ok_or(InvalidSigner)
    }
    pub fn realloc(&self, info: &'a AccountInfo<'a>, len: usize) -> Result<()> {
        assert_eq!(info.owner, self.program_id);
        if info.data_len() == len {
            return Ok(());
        }

        if info.data_len() < len {
            self.inc_alloc(len.saturating_sub(info.data_len()))?
        } else {
            self.inc_dealloc(info.data_len().saturating_sub(len))?
        }

        info.resize(len)?;

        let is_paid = AccountType::from_account(info)?.is_paid();
        self.transfer_rent(info, is_paid)
    }

    pub fn transfer_rent(&self, pda: &'a AccountInfo<'a>, refund_to_signer: bool) -> Result<()> {
        let rent = Rent::get()?.minimum_balance(pda.data_len());

        if rent > pda.lamports() {
            let ix = transfer(self.signer.key, pda.key, rent - pda.lamports());
            self.invoke_signed(&ix, vec![], refund_to_signer)?;
        }

        Ok(())
    }
    pub fn create_pda(
        &self,
        pda: &'a AccountInfo<'a>,
        seed: &Seed,
        len: usize,
        refund_to_signer: bool,
        owner: &Pubkey,
    ) -> Result<()> {
        assert_eq!(pda.data_len(), 0); // it is not possible to allocate the pda account outside the program
        assert!(!pda.is_signer);
        assert!(pda.is_writable);
        assert_eq!(*pda.owner, system_program::ID);

        if pda.lamports() > 0 {
            let ix = allocate(pda.key, len as u64);
            self.invoke_signed(&ix, vec![seed.clone()], refund_to_signer)?;

            let ix = assign(pda.key, owner);
            self.invoke_signed(&ix, vec![seed.clone()], refund_to_signer)?;

            self.transfer_rent(pda, refund_to_signer)?;
        } else {
            let ix = create_account(
                self.signer.key,
                pda.key,
                Rent::get()?.minimum_balance(len),
                len as u64,
                owner,
            );
            self.invoke_signed(&ix, vec![seed.clone()], refund_to_signer)?;
        }

        self.inc_alloc(len)
    }
    pub fn all(&self) -> &FastMap<Pubkey, &'a AccountInfo<'a>> {
        &self.all
    }
    pub fn info_any(&self, key: &Pubkey) -> Result<&'a AccountInfo<'a>> {
        self
            .all()
            .get(key)
            .cloned()
            .ok_or(AccountNotFound(*key))
    }

    pub fn ix_infos(&self, ix: &Instruction) -> Result<Vec<AccountInfo<'a>>> {
        let f_info = |key: Pubkey| self.info_any(&key).cloned();

        #[allow(unused_assignments)]
        let mut infos = Vec::with_capacity(ix.accounts.len() + 1);

        infos = ix
            .accounts
            .iter()
            .map(|a| f_info(a.pubkey))
            .collect::<Result<Vec<_>>>()?;

        infos.push(f_info(ix.program_id)?);

        Ok(infos)
    }

    pub fn remove_pda(&self, info: &'a AccountInfo<'a>) -> Result<()> {
        match *AccountType::from_account(info)? {
            AccountType::AltSlots | AccountType::TxHolder | AccountType::StateHolder => {},
            _ => unreachable!(),
        }

        self.inc_dealloc(info.data_len())?;
        info.resize(0)?;

        let lamports = self
            .signer.
            lamports()
            .checked_add(info.lamports())
            .ok_or(CalculationOverflow)?;

        **self.signer.lamports.borrow_mut() = lamports;
        **info.lamports.borrow_mut() = 0;

        info.assign(&system_program::ID);

        Ok(())
    }
}
