mod confirm_tx_iterative;
mod deposit;
pub mod do_tx;
pub mod do_tx_holder;
pub mod do_tx_holder_iterative;
pub mod do_tx_iterative;
mod eth_call;
mod eth_get_balance;
mod eth_get_code;
mod eth_get_storage_at;
mod eth_get_tx_count;
mod get_rollups;
mod reg_owner;
mod transmit_tx;
mod create_treasure;
mod join_treasure;
mod upgrade_auth_key;
mod alloc_resource;
mod dealloc_resource;
mod confirm_tx_iterative_opt;
mod emulate_call;

use std::collections::HashMap;
pub use confirm_tx_iterative::confirm_tx_iterative;
pub use deposit::deposit;
pub use do_tx::do_tx;
pub use do_tx_holder::do_tx_holder;
pub use do_tx_holder_iterative::do_tx_holder_iterative;
pub use do_tx_iterative::do_tx_iterative;
pub use eth_call::eth_call;
pub use eth_get_balance::eth_get_balance;
pub use eth_get_code::eth_get_code;
pub use eth_get_storage_at::eth_get_storage_at;
pub use eth_get_tx_count::eth_get_tx_count;
pub use get_rollups::get_rollups;
pub use reg_owner::reg_owner;
use solana_program::entrypoint::MAX_PERMITTED_DATA_INCREASE;
use solana_program::instruction::AccountMeta;
pub use transmit_tx::transmit_tx;
pub use create_treasure::create_treasure;
pub use join_treasure::join_treasure;
pub use upgrade_auth_key::upgrade_auth_key;
pub use alloc_resource::alloc_resource;
pub use dealloc_resource::dealloc_resource;
pub use confirm_tx_iterative_opt::confirm_tx_iterative_opt;
pub use emulate_call::emulate_call;

use {
    crate::{
        state::{Item, Slots, State,},
    },
    rome_evm::{
        accounts::{AccountState, AccountType, Data},
        error::{Result, RomeProgramError::*},
        ExitReason, H160, NUMBER_OPCODES_TO_ESTIMATE_TX_FLOW,
        state::{pda::Pda, Account,}
    },
    solana_program::{
        account_info::IntoAccountInfo, msg, pubkey::Pubkey,
    },
    std::collections::BTreeMap,
};

#[derive(Debug, Clone)]
pub struct Vm {
    pub exit_reason: ExitReason,
    pub return_value: Option<Vec<u8>>,
    pub steps_executed: u64,
    pub iteration_count: u64,
    pub exec_iteration_count: Option<u64>,
}
#[derive(Debug, Clone)]
pub enum Flow {
    Atomic,
    Iterative,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Emulation {
    pub accounts: BTreeMap<Pubkey, Item>,
    pub original: HashMap<Pubkey, solana_account::Account>,
    pub upgradeable_elf: HashMap<Pubkey, solana_account::Account>,
    pub storage: BTreeMap<H160, Slots>,
    pub vm: Option<Vm>,
    pub alloc: usize,
    pub dealloc: usize,
    pub alloc_payed: usize,
    pub dealloc_payed: usize,
    pub gas: u64,
    pub syscalls: u64,
    pub flow: Flow,
}
impl Emulation {
    #[allow(clippy::too_many_arguments)]
    pub fn with_vm(
        state: &State,
        exit_reason: Option<ExitReason>,
        return_value: Option<Vec<u8>>,
        steps_executed: u64,
        iter_count: u64,
        alloc: usize,
        dealloc: usize,
        alloc_payed: usize,
        dealloc_payed: usize,
        syscalls: u64,
        lamports_fee: u64,
        lamports_refund: u64,
        found_cpi: bool,
        exec_cnt: Option<u64>,
    ) -> Result<Self> {

        // it makes sense only for emulation of the atomic_tx
        let only_iterative = if alloc > MAX_PERMITTED_DATA_INCREASE || syscalls >= 64 {
                true
            } else {
                false
            };

        let flow = if only_iterative {
            if found_cpi {
                return Err(TxCannotBeAtomic)
            }
            Flow::Iterative
        } else {
            if found_cpi {
                Flow::Atomic
            } else {
                // it is necessary to estimate gas an accurate for some of oz tests
                // Most likely, the following  assumption should be correct
                if steps_executed <= NUMBER_OPCODES_TO_ESTIMATE_TX_FLOW {
                    Flow::Atomic
                } else {
                    Flow::Unknown
                }
            }
        };

        msg!(">> emulation results:");
        msg!("steps_executed: {}", steps_executed);
        msg!("number of iterations: {}", iter_count);
        msg!("allocated: {}", alloc);
        msg!("deallocated: {}", dealloc);
        msg!("allocated_payed: {}", alloc_payed);
        msg!("deallocated_payed: {}", dealloc_payed);
        msg!("exit_reason: {:?}", exit_reason);
        msg!("syscalls: {}", syscalls);
        msg!("lamports_fee: {}", lamports_fee);
        msg!("lamports_refund: {}", lamports_refund);
        msg!("gas: {:?}", lamports_fee);
        msg!("flow: {:?}", flow);
        msg!("found_cpi: {}", found_cpi);
        msg!("number of execute iterations: {:?}", exec_cnt);

        Emulation::log_accounts(state)?;

        let vm = Vm {
            exit_reason: exit_reason.ok_or(VmFault("exit_reason expected".to_string()))?,
            return_value,
            steps_executed,
            iteration_count: iter_count,
            exec_iteration_count: exec_cnt,
        };

        Ok(Self {
            accounts: state.accounts.borrow().clone(),
            original: state.original.borrow().clone(),
            storage: state.storage.borrow().clone(),
            upgradeable_elf: state.upgradeable_elf.borrow().clone(),
            vm: Some(vm),
            alloc,
            dealloc,
            alloc_payed,
            dealloc_payed,
            gas: lamports_fee,
            syscalls,
            flow,
        })
    }

    fn log_accounts(state: &State) -> Result<()> {
        msg!("accounts:");
        msg!("Pubkey | is_writable | is_signer | AccountType | data.len() | {address} ");
        for (key, item) in state.accounts.borrow().iter() {
            let mut bind = (*key, item.account.clone());
            let info = bind.into_account_info();
            let is_pda = AccountType::check_owner(&info, state.program_id).is_ok();

            let type_ = if is_pda {
                let typ = AccountType::from_account(&info)?;
                let mut is_contract = "".to_string();

                if *typ == AccountType::Balance && AccountState::from_account(&info)?.is_contract {
                    is_contract = "(contract)".to_string();
                }
                format!("{:?}{}", typ, is_contract)
            } else {
                "System".to_string()
            };

            if let Some(address) = item.address {
                msg!(
                    "{} {} {} {} {} {}",
                    key,
                    item.account.writable,
                    item.account.signer,
                    type_,
                    item.account.data.len(),
                    address,
                )
            } else {
                msg!(
                    "{} {} {} {} {}",
                    key,
                    item.account.writable,
                    item.account.signer,
                    type_,
                    item.account.data.len(),
                )
            }
        }

        Ok(())
    }

    pub fn without_vm(state: &State) -> Result<Self> {
        let alloc = state.alloc();
        let dealloc = state.dealloc();
        let alloc_payed = state.alloc_payed();
        let dealloc_payed = state.dealloc_payed();

        msg!(">> emulation results:");
        msg!("allocated: {}", alloc);
        msg!("deallocated: {}", dealloc);
        msg!("allocated_payed: {}", alloc_payed);
        msg!("deallocated_payed: {}", dealloc_payed);
        Emulation::log_accounts(state)?;

        Ok(Self {
            accounts: state.accounts.borrow().clone(),
            original: state.original.borrow().clone(),
            storage: state.storage.borrow().clone(),
            upgradeable_elf: state.upgradeable_elf.borrow().clone(),
            vm: None,
            alloc,
            dealloc,
            alloc_payed,
            dealloc_payed,
            gas: 0,
            syscalls: state.pda.syscall.count(),
            flow: Flow::Atomic,
        })
    }

    pub fn get_account_metas(&self) -> Vec<AccountMeta> {
        self.accounts
            .iter()
            .map(|(pubkey, item)| AccountMeta {
                pubkey: *pubkey,
                is_signer: false,
                is_writable: item.account.writable,
            })
            .collect::<Vec<AccountMeta>>()
    }

    pub fn add_tx_holder(
        &mut self,
        program_id: &Pubkey,
        chain: u64,
        holder: u64,
        signer: &Pubkey,
    ) {
        let pda = Pda::new_(&program_id, chain);
        let (tx_holder, _) = pda.tx_holder_key(signer, holder);
        let account = Account {
            writable: true,
            ..Default::default()
        };
        self.accounts.insert(tx_holder, Item { account, address: None });
    }
}
