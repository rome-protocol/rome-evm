use {
    crate::{
        accounts::{AccountState, Data},
        context::AccountLock,
        error::{Result, RomeProgramError::*},
        info::Info,
        state::{base::Base, State},
        Code, Account, EVENT_LOG, pda::Seed,
        MICRO_LAMPORTS_PER_LAMPORT,
    },
    evm::{H160, H256, U256},
    solana_program::{
        account_info::AccountInfo,
        instruction::Instruction,
        log::sol_log_data, program::{
            invoke_signed, invoke,
        },
        pubkey::Pubkey,
    },
    solana_program::{
        sysvar::instructions::load_instruction_at_checked
    },
    std::cmp::Ordering::*,
};

pub trait Origin {
    fn nonce(&self, address: &H160) -> Result<Option<u64>>;
    fn balance(&self, address: &H160) -> Result<Option<U256>>;
    fn code(&self, address: &H160) -> Result<Option<Vec<u8>>>;
    fn valids(&self, address: &H160) -> Result<Option<Vec<u8>>>;
    fn storage(&self, address: &H160, slot: &U256) -> Result<Option<U256>>;
    fn inc_nonce<L: AccountLock>(&self, address: &H160, context: &L) -> Result<()>;
    fn add_balance<L: AccountLock>(
        &self,
        address: &H160,
        balance: &U256,
        context: &L,
    ) -> Result<()>;
    fn sub_balance<L: AccountLock>(
        &self,
        address: &H160,
        balance: &U256,
        context: &L,
    ) -> Result<()>;
    fn set_code<L: AccountLock>(
        &self,
        address: &H160,
        code: &[u8],
        valids: &[u8],
        context: &L,
    ) -> Result<()>;
    fn set_storage<L: AccountLock>(
        &self,
        address: &H160,
        slot: &U256,
        value: &U256,
        context: &L,
    ) -> Result<()>;
    fn set_logs(&self, address: &H160, topics: &[H256], data: &[u8]) -> Result<()> {
        match topics.len() {
            0 => sol_log_data(&[EVENT_LOG, address.as_bytes(), &[0_u8], data]),
            1 => sol_log_data(&[
                EVENT_LOG,
                address.as_bytes(),
                &[1_u8],
                topics[0].as_bytes(),
                data,
            ]),
            2 => sol_log_data(&[
                EVENT_LOG,
                address.as_bytes(),
                &[2_u8],
                topics[0].as_bytes(),
                topics[1].as_bytes(),
                data,
            ]),
            3 => sol_log_data(&[
                EVENT_LOG,
                address.as_bytes(),
                &[3_u8],
                topics[0].as_bytes(),
                topics[1].as_bytes(),
                topics[2].as_bytes(),
                data,
            ]),
            4 => sol_log_data(&[
                EVENT_LOG,
                address.as_bytes(),
                &[4_u8],
                topics[0].as_bytes(),
                topics[1].as_bytes(),
                topics[2].as_bytes(),
                topics[3].as_bytes(),
                data,
            ]),
            _ => panic!("vm fault, event logs topics.len > 4"),
        }

        Ok(())
    }
    fn base(&self) -> &Base<'_>;
    fn account(&self, key: &Pubkey) -> Result<Account>;
    /// Scoped borrow of a Solana `AccountInfo` for the given key, executed inside the
    /// caller-supplied closure. Avoids returning a reference because `AccountInfo<'a>`
    /// is invariant in `'a` — see `program/src/state/origin.rs` review notes 2026-05-19.
    ///
    /// State (on-chain) delegates to `State::info_any` and forwards the AccountInfo to `f`.
    /// Emulator returns `Err(Unsupported(...))` without invoking `f` — emulator precompile
    /// paths short-circuit before reaching this method (see `2026-05-19-rome-ed25519-precompile.md` §8.3).
    fn with_account_info<F, R>(&self, key: &Pubkey, f: F) -> Result<R>
    where
        F: FnOnce(&AccountInfo) -> Result<R>;
    fn invoke_signed(&self, ix: &Instruction, seeds: Vec<Seed>, refund_to_signer: bool) -> Result<()>;
    fn invoke_signed_unchecked(&self, ix: &Instruction, seeds: Vec<Seed>) -> Result<()>;
    fn signer(&self) -> Pubkey;
    fn wallet(&self) -> Result<Pubkey>;
    fn treasure(&self, id: u64) -> Result<Pubkey>;
    fn owner(&self, key: &Pubkey) -> Result<Pubkey>;
    fn ed25519_data(&self) -> Result<Vec<u8>>;
}

impl Origin for State<'_> {
    fn nonce(&self, address: &H160) -> Result<Option<u64>> {
        self
            .info_addr_opt(address)?
            .map_or(
                Ok(None),
                |info| Info::nonce(self, info).map(|nonce| Some(nonce))
            )
    }

    fn balance(&self, address: &H160) -> Result<Option<U256>> {
        self
            .info_addr_opt(address)?
            .map_or(
                Ok(None),
                |info| Info::balance(self, info).map(|balance| Some(balance))
            )
    }

    fn code(&self, address: &H160) -> Result<Option<Vec<u8>>> {
        self
            .info_addr_opt(address)?
            .map_or(
                Ok(None),
                |info| Info::code(self, info).map(|code| Some(code))
            )
    }
    fn valids(&self, address: &H160) -> Result<Option<Vec<u8>>> {
        self
            .info_addr_opt(address)?
            .map_or(
                Ok(None),
                |info| Info::valids(self, info).map(|valids| Some(valids))
            )
    }

    fn storage(&self, address: &H160, slot: &U256) -> Result<Option<U256>> {
        let (info, sub_ix) = self.info_slot_opt(address, slot)?;
        info
            .map_or(
                Ok(None),
                |info| Info::storage(self, info, sub_ix)
            )
    }

    fn inc_nonce<L: AccountLock>(&self, address: &H160, context: &L) -> Result<()> {
        let info = self.info_addr(address, true)?;
        Info::inc_nonce(self, info, context)
    }

    fn add_balance<L: AccountLock>(
        &self,
        address: &H160,
        balance: &U256,
        context: &L,
    ) -> Result<()> {
        let info = self.info_addr(address, true)?;
        Info::add_balance(self, info, balance, context)
    }

    fn sub_balance<L: AccountLock>(
        &self,
        address: &H160,
        balance: &U256,
        context: &L,
    ) -> Result<()> {
        let info = self.info_addr(address, true)?;
        Info::sub_balance(self, info, balance, address, context)
    }

    fn set_code<L: AccountLock>(
        &self,
        address: &H160,
        code: &[u8],
        valids: &[u8],
        context: &L,
    ) -> Result<()> {
        let info = self.info_addr(address, true)?;
        AccountState::check_no_contract(info, address)?;

        let len = info.data_len();
        let offset = Code::offset(info);
        let required = offset + code.len() + valids.len();

        // TODO move allocation to trait for use in the emulator
        match len.cmp(&required) {
            Less => {
                self.realloc(info, required)?;
            }
            Greater => {
                // TODO: implement deallocation of the unused contract space
                return Err(Unimplemented(
                    format!(
                        "the contract deployment space must be deallocated according to size of the contract \
                        pubkey: {}, available: {}, required: {}",
                        info.key,
                        len,
                        required
                    ))
                );
            }
            _ => {}
        }

        Info::set_code(self, info, code, valids, address, context)
    }

    fn set_storage<L: AccountLock>(
        &self,
        address: &H160,
        slot: &U256,
        value: &U256,
        context: &L,
    ) -> Result<()> {
        let (info, sub_ix) = self.info_slot(address, slot, true)?;
        Info::set_storage(self, info, sub_ix, value, context)
    }

    fn base(&self) -> &Base<'_> {
        &self.base
    }

    fn with_account_info<F, R>(&self, key: &Pubkey, f: F) -> Result<R>
    where
        F: FnOnce(&AccountInfo) -> Result<R>,
    {
        let info = self.info_any(key)?;
        f(info)
    }

    fn account(&self, key: &Pubkey) -> Result<Account> {
        let info = self.info_any(key)?;
        let acc = if info.executable {
            Account {
                lamports: info.lamports(),
                data: vec![],
                owner: *info.owner,
                executable: info.executable,
                writable: info.is_writable,
                signer: info.is_signer,
            }
        } else {
            Account::from_account_info(info)
        };

        Ok(acc)
    }

    fn invoke_signed(&self, ix: &Instruction, seeds: Vec<Seed>, refund_to_signer: bool) -> Result<()> {
        let infos = self.ix_infos(ix)?;
        self.syscall.inc();

        if refund_to_signer {
            let init = self.signer.lamports();
            cpi(ix, seeds, infos)?;
            let actual = self.signer.lamports();

            if init > actual {
                self.add_fee(init - actual)
            } else if actual > init {
                self.add_refund(actual - init)
            } else {
                Ok(())
            }
        } else {
            cpi(ix, seeds, infos)
        }
    }
    fn invoke_signed_unchecked(&self, ix: &Instruction, seeds: Vec<Seed>) -> Result<()> {
        let infos = self.ix_infos(ix)?;
        self.syscall.inc();
        cpi(ix, seeds, infos)
    }
    fn signer(&self) -> Pubkey {
        *self.signer.key
    }
    fn wallet(&self) -> Result<Pubkey> {
        let info = self.info_sol_wallet(false)?;
        Ok(*info.key)
    }
    fn treasure(&self, id: u64) -> Result<Pubkey> {
        let info = self.info_treasure(id, true)?; // TODO switch to false
        Ok(*info.key)
    }
    fn owner(&self, key: &Pubkey) -> Result<Pubkey> {
        self.info_any(key).map(|a| *a.owner)
    }
    fn ed25519_data(&self) -> Result<Vec<u8>> {
        let key = solana_program::sysvar::instructions::id();
        let info = self.info_any(&key)?;
        let ix = load_instruction_at_checked(0, info)
            .map_err(|e| Custom(format!("Ed25519 ix not found: {:?}", e)))?;
        if ix.program_id != solana_program::ed25519_program::ID {
            return Err(Ed25519IxNotFound)
        }
        Ok(ix.data)
    }
}

fn cpi(ix: &Instruction, seeds: Vec<Seed>, infos: Vec<AccountInfo>) -> Result<()>{
    if seeds.is_empty() {
        invoke(ix, infos.as_slice())?;
    } else {
        let owned = seeds
            .iter()
            .map(|a| a.cast())
            .collect::<Vec<_>>();
        let slices = owned.iter().map(|a| a.as_slice()).collect::<Vec<_>>();

        invoke_signed(ix, infos.as_slice(), slices.as_slice())?;
    }

    Ok(())
}

pub fn priority_lamports(cu: u32, price: u64) -> Result<u64> {
    let micro_lamports: u128 = (cu as u128).saturating_mul(price as u128);

    micro_lamports
        .saturating_add(u128::from(MICRO_LAMPORTS_PER_LAMPORT - 1))
        .checked_div(u128::from(MICRO_LAMPORTS_PER_LAMPORT))
        .and_then(|fee| u64::try_from(fee).ok())
        .ok_or(CalculationOverflow)
}