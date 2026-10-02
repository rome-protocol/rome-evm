use {
    crate::{
        allocate::Allocate, aux::Ix, context::AccountLock, error::{Result, RomeProgramError::*},
        origin::Origin, NUMBER_SYSCALLS_PER_ALLOC_ITER, dmsg, Account,
    },
    borsh::{BorshDeserialize, BorshSerialize},
    evm::{H160, H256, U256},
    std::collections::BTreeMap,
    solana_program::pubkey::Pubkey,
};
use super::fast_hash::{FastMap, FastSet};

#[allow(dead_code)]
#[derive(BorshSerialize, BorshDeserialize, Clone)]
pub enum Diff {
    TransferFrom { balance: U256 },
    TransferTo { balance: U256 },
    NonceChange,
    StorageChange { key: U256, value: U256 },
    TStorageChange { key: U256, value: U256 },
    CodeChange { code: Vec<u8>, valids: Vec<u8> },
    Event { topics: Vec<H256>, data: Vec<u8> },
}

pub type NonEvmState = FastMap<Pubkey, Account>;
/// Journal entries that are used to track changes to the state and are used to revert it.
#[derive(Default)]
pub struct Journal {
    pub diff: BTreeMap<H160, Vec<Diff>>,
    pub non_evm_ix: Option<Vec<Ix>>,
    non_evm_state: Option<NonEvmState>,
    pub parent: Option<Box<Journal>>,
    pub page: u64,
    pub found_cpi: bool,
}

impl Journal {
    pub fn new() -> Self {
        Self {
            diff: BTreeMap::new(),
            non_evm_ix: None,
            non_evm_state: None, // lazy, cloned from parent on demand
            parent: None,
            page: 0,
            found_cpi: false,
        }
    }

    pub fn non_evm_state(&self) -> Option<&NonEvmState> {
        if self.non_evm_state.is_some() {
            return self.non_evm_state.as_ref();
        }

        if let Some(parent) = &self.parent {
            parent.non_evm_state()
        } else {
            None
        }
    }

    pub fn non_evm_state_mut(&mut self) -> &mut NonEvmState {
        if self.non_evm_state.is_none() {
            self.non_evm_state = self.non_evm_state_latest();
        }
        self.non_evm_state.as_mut().unwrap()
    }

    fn non_evm_state_latest(&self) -> Option<NonEvmState> {
        if self.non_evm_state.is_some() {
            return self.non_evm_state.clone();
        }

        if let Some(parent) = &self.parent {
            parent.non_evm_state_latest()
        } else {
            Some(NonEvmState::default())
        }
    }

    pub fn next_page(parent: Box<Journal>) -> Self {
        let mut new = Self::new();
        new.page = parent.page + 1;
        new.parent = Some(parent);
        new
    }

    pub fn nonce_diff(&self, address: &H160) -> u64 {
        let mut nonce = if let Some(parent) = &self.parent {
            parent.nonce_diff(address)
        } else {
            0
        };

        if let Some(items) = self.diff.get(address) {
            for item in items {
                if let Diff::NonceChange = item {
                    // nonce = nonce
                    nonce += 1;
                }
            }
        }
        nonce
    }

    pub fn transfer_from(&self, address: &H160) -> Result<U256> {
        let mut value = if let Some(parent) = &self.parent {
            parent.transfer_from(address)?
        } else {
            U256::zero()
        };

        if let Some(items) = self.diff.get(address) {
            for item in items {
                if let Diff::TransferFrom { balance } = item {
                    value = value.checked_add(*balance).ok_or(CalculationOverflow)?;
                }
            }
        }

        Ok(value)
    }

    pub fn transfer_to(&self, address: &H160) -> Result<U256> {
        let mut value = if let Some(parent) = &self.parent {
            parent.transfer_to(address)?
        } else {
            U256::zero()
        };

        if let Some(items) = self.diff.get(address) {
            for item in items {
                if let Diff::TransferTo { balance } = item {
                    value = value.checked_add(*balance).ok_or(CalculationOverflow)?;
                }
            }
        }

        Ok(value)
    }

    pub fn get_mut(&mut self, address: &H160) -> &mut Vec<Diff> {
        self.diff.entry(*address).or_default()
    }

    pub fn code_valids_diff(&self, address: &H160) -> Option<(&Vec<u8>, &Vec<u8>)> {
        if let Some(items) = self.diff.get(address) {
            for item in items.iter().rev() {
                if let Diff::CodeChange { code, valids } = item {
                    return Some((code, valids));
                }
            }
        }

        self.parent
            .as_ref()
            .and_then(|parent| parent.code_valids_diff(address))
    }

    pub fn storage_diff(&self, address: &H160, index: &U256) -> Option<U256> {
        if let Some(items) = self.diff.get(address) {
            for item in items.iter().rev() {
                if let Diff::StorageChange { key, value } = item {
                    if key == index {
                        return Some(*value);
                    }
                }
            }
        }

        self.parent
            .as_ref()
            .and_then(|parent| parent.storage_diff(address, index))
    }
    pub fn t_storage_diff(&self, address: &H160, index: &U256) -> Option<U256> {
        if let Some(items) = self.diff.get(address) {
            for item in items.iter().rev() {
                if let Diff::TStorageChange { key, value } = item {
                    if key == index {
                        return Some(*value);
                    }
                }
            }
        }

        self.parent
            .as_ref()
            .and_then(|parent| parent.t_storage_diff(address, index))
    }

    pub fn depth(&self) -> usize {
        let depth = if let Some(parent) = self.parent.as_ref() {
            parent.depth()
        } else {
            0
        };
        depth + 1
    }

    fn serialize_recursive(&self, into: &mut &mut [u8]) -> Result<()> {
        if let Some(parent) = self.parent.as_ref() {
            parent.serialize_recursive(into)?
        }

        self.diff.serialize(into)?;
        self.non_evm_ix.serialize(into)?;
        self.non_evm_state.serialize(into)?;
        self.page.serialize(into)?;
        self.found_cpi.serialize(into)?;
        Ok(())
    }
    pub fn serialize(&self, into: &mut &mut [u8]) -> Result<()> {
        let depth = self.depth();
        depth.serialize(into)?;
        self.serialize_recursive(into)
    }
    pub fn deserialize(from: &mut &[u8]) -> Result<Self> {
        let mut journal = None;
        let depth: usize = BorshDeserialize::deserialize(from)?;
        assert!(depth > 0);

        for _ in 0..depth {
            let diff: BTreeMap<H160, Vec<Diff>> = BorshDeserialize::deserialize(from)?;
            let non_evm_ix: Option<Vec<Ix>> = BorshDeserialize::deserialize(from)?;
            let non_evm_state: Option<NonEvmState> = BorshDeserialize::deserialize(from)?;
            let page: u64 = BorshDeserialize::deserialize(from)?;
            let found_cpi: bool = BorshDeserialize::deserialize(from)?;

            journal = Some(Box::new(Self {
                diff,
                non_evm_ix,
                non_evm_state,
                parent: journal,
                page,
                found_cpi,
            }));
        }

        Ok(*journal.expect("journal expected"))
    }
    pub fn commit<T: Origin, L: AccountLock>(&mut self, state: &T, context: &L) -> Result<()> {
        if let Some(parent) = self.parent.as_mut() {
            parent.commit(state, context)?
        }

        for (address, diffs) in &self.diff {
            for diff in diffs {
                match diff {
                    Diff::NonceChange => {
                        dmsg!("NonceChange");
                        state.inc_nonce(address, context)?;
                    }
                    Diff::CodeChange { code, valids } => {
                        state.set_code(address, code, valids, context)?;
                        dmsg!("contract is deployed");
                    }
                    Diff::StorageChange { key, value } => {
                        state.set_storage(address, key, value, context)?;
                    }
                    Diff::Event { topics, data } => {
                        dmsg!("SetLogs");
                        state.set_logs(address, topics, data)?;
                    }
                    Diff::TransferFrom { balance } => {
                        dmsg!("TransferFrom");
                        state.sub_balance(address, balance, context)?;
                    }
                    Diff::TransferTo { balance } => {
                        dmsg!("TransferTo");
                        state.add_balance(address, balance, context)?;
                    }
                    Diff::TStorageChange { .. } => {
                        dmsg!("TStorageChange");
                    }
                }
            }
        }

        if let Some(invokes) = self.non_evm_ix.take() {
            for ix_ in invokes {
                let (ix, seeds) = ix_.cast();
                dmsg!("InvokeSigned {}", &ix.program_id);
                state.invoke_signed(&ix, seeds, true)?;
            }
        }

        Ok(())
    }

    pub fn alloc_balances<T: Allocate + Origin, L: AccountLock>(
        &self,
        state: &T,
        context: &L,
        syscall_start: u64, // to avoid depending on the emulator's syscall counter
    ) -> Result<bool> {
        if !self
            .parent
            .as_ref()
            .map_or(Ok(true), |parent| parent.alloc_balances(state, context, syscall_start))?
        {
            return Ok(false);
        }

        for (address, diffs) in &self.diff {
            for diff in diffs {
                // the alloc_limit should be enough to allocate the AccountState
                let base = state.base();
                let syscall_cnt = base.pda.syscall.count() - syscall_start;

                if syscall_cnt >= NUMBER_SYSCALLS_PER_ALLOC_ITER || base.alloc_limit() < 500 {
                    return Ok(false);
                }

                match diff {
                    // TODO check allocation limit
                    Diff::NonceChange
                    | Diff::TransferFrom { balance: _ }
                    | Diff::TransferTo { balance: _ } => {
                        state.alloc_balance(address, context)?;
                    }
                    Diff::CodeChange { code, valids } => {
                        if !state.alloc_contract(address, code, valids, context)? {
                            return Ok(false);
                        }
                    }
                    Diff::StorageChange { key, value: _ } => {
                        // just to calculate and cache the hash
                        let (_, _, _) = state.base().slot_to_key(address, key);
                    }
                    _ => {}
                };
            }
        }

        Ok(true)
    }
    pub fn diff_len(&self) -> usize {
        let parent = self.parent.as_ref().map_or(0, |parent| parent.diff_len());

        parent + self.diff.values().fold(0, |s, a| s + a.len())
    }

    pub fn merge_slots<T: Origin>(&self, state: &T) -> Result<BTreeMap<H160, FastSet<U256>>> {
        let parent = self
            .parent
            .as_ref()
            .map_or(Ok(BTreeMap::new()), |parent| parent.merge_slots(state))?;

        let mut new = BTreeMap::new();
        for (address, diffs) in self.diff.iter() {
            let mut slots = FastSet::default();

            for diff in diffs.iter() {
                if let Diff::StorageChange { key: slot, value: _} = diff {
                    if state.storage(address, slot)?.is_none() {
                        slots.insert(*slot);
                    }
                }
            }
            new.insert(address.clone(), slots);
        }

        // merge from parent
        parent.into_iter().for_each(|(address, set)| {
            if let Some(new_set) = new.get_mut(&address) {
                new_set.extend(set);
            } else {
                new.insert(address, set);
            }
        });

        Ok(new)
    }

    pub fn selfdestruct(&mut self, address: &H160) {
        if let Some(parent) = self.parent.as_mut() {
            parent.selfdestruct(address)
        }

        if let Some(diffs) = self.diff.get_mut(address) {
            *diffs = diffs
                .iter()
                .filter(|a| {
                    matches!(
                        a,
                        Diff::TransferFrom { balance: _ } | Diff::TransferTo { balance: _ }
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
        }
    }

    pub fn found_storage(&self) -> bool {
        let found = self.diff.iter().any(|(_, diffs)| {
            diffs
                .iter()
                .any(|diff| matches!(diff, Diff::StorageChange { key: _, value: _ }))
        });

        if found {
            return true;
        }

        if let Some(parent) = self.parent.as_ref() {
            parent.found_storage()
        } else {
            false
        }
    }

    pub fn revert_page(&mut self, page: u64) -> Self {
        if let Some(mut parent) = self.parent.take() {
            if parent.page < page {
                *parent
            } else {
                parent.revert_page(page)
            }
        } else {
            Journal::new()
        }
    }
    pub fn found_cpi(&self, page: u64) -> bool {
        if self.found_cpi {
            return true
        }

        if let Some(parent) = self.parent.as_ref() {
            if parent.page < page {
                false
            } else {
                parent.found_cpi(page)
            }
        } else {
            false
        }
    }

    pub fn merge_page(&mut self) {
        self.page = self.page.checked_sub(1).expect("fm vault");
    }
}
