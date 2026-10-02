use {
    crate::{
        state::State, ix_store,
    },
    mollusk::Mollusk,
    rome_evm::{
        context::AccountLock,
        error::{Result, RomeProgramError::*},
        info::Info,
        origin::Origin,
        Base, Code, Data, Account, H160, U256, pda::Seed,
    },
    solana_program::{
        account_info::{AccountInfo, IntoAccountInfo}, instruction::Instruction, pubkey::Pubkey,
    },
    mollusk_svm_result::types::ProgramResult,
    solana_system_interface::program as system_program,
    std::{
        cmp::Ordering::{Greater, Less},
    },
};

impl Info for State<'_> {}

impl Origin for State<'_> {
    fn nonce(&self, address: &H160) -> Result<Option<u64>> {
        self
            .info_addr_opt(address)?
            .map_or(
                Ok(None),
                |mut bind| {
                    let info = bind.into_account_info();
                    Info::nonce(self, &info).map(|nonce| Some(nonce))
                }
            )
    }
    fn balance(&self, address: &H160) -> Result<Option<U256>> {
        self
            .info_addr_opt(address)?
            .map_or(
                Ok(None),
                |mut bind| {
                    let info = bind.into_account_info();
                    Info::balance(self, &info).map(|balance| Some(balance))
                }
            )
    }
    fn code(&self, address: &H160) -> Result<Option<Vec<u8>>> {
        self
            .info_addr_opt(address)?
            .map_or(
                Ok(None),
                |mut bind| {
                    let info = bind.into_account_info();
                    Info::code(self, &info).map(|code| Some(code))
                }
            )
    }
    fn valids(&self, address: &H160) -> Result<Option<Vec<u8>>> {
        self
            .info_addr_opt(address)?
            .map_or(
                Ok(None),
                |mut bind| {
                    let info = bind.into_account_info();
                    Info::valids(self, &info).map(|valids| Some(valids))
                }
            )
    }
    fn storage(&self, address: &H160, slot: &U256) -> Result<Option<U256>> {
        let (bind, sub_ix) = self.info_slot_opt(address, slot)?;
        bind
            .map_or(
                Ok(None),
                |mut bind| {
                    let info = bind.into_account_info();
                    Info::storage(self, &info, sub_ix)
                }
            )
    }
    fn inc_nonce<L: AccountLock>(&self, address: &H160, context: &L) -> Result<()> {
        let mut bind = self.info_addr(address, true)?;
        let info = bind.into_account_info();
        Info::inc_nonce(self, &info, context)?;
        self.update(bind);
        Ok(())
    }
    fn add_balance<L: AccountLock>(
        &self,
        address: &H160,
        balance: &U256,
        context: &L,
    ) -> Result<()> {
        let mut bind = self.info_addr(address, true)?;
        let info = bind.into_account_info();
        Info::add_balance(self, &info, balance, context)?;
        self.update(bind);
        Ok(())
    }
    fn sub_balance<L: AccountLock>(
        &self,
        address: &H160,
        balance: &U256,
        context: &L,
    ) -> Result<()> {
        let mut bind = self.info_addr(address, true)?;
        let info = bind.into_account_info();
        Info::sub_balance(self, &info, balance, address, context)?;
        self.update(bind);
        Ok(())
    }
    fn set_code<L: AccountLock>(
        &self,
        address: &H160,
        code: &[u8],
        valids: &[u8],
        context: &L,
    ) -> Result<()> {
        let mut bind = self.info_addr(address, true)?;

        let len = bind.1.data.len();
        let offset = Code::offset(&bind.into_account_info());
        let required = offset + code.len() + valids.len();

        match len.cmp(&required) {
            Less => {
                self.realloc(&bind.0, required, true)?;
            }
            Greater => {
                // TODO: implement deallocation of the unused contract space
                return Err(Unimplemented(
                    format!(
                        "the contract deployment space must be deallocated according to size of the contract \
                        pubkey: {}, available: {}, required: {}",
                        bind.0,
                        len,
                        required
                    ))
                );
            }
            _ => {}
        }

        let mut bind = self.info_addr(address, false)?;
        let info = bind.into_account_info();
        Info::set_code(self, &info, code, valids, address, context)?;
        self.update(bind);
        Ok(())
    }
    fn set_storage<L: AccountLock>(
        &self,
        address: &H160,
        slot: &U256,
        value: &U256,
        context: &L,
    ) -> Result<()> {
        let (mut bind, sub_ix) = self.info_slot(address, slot, true)?;
        let info = bind.into_account_info();
        Info::set_storage(self, &info, sub_ix, value, context)?;
        self.update(bind);
        Ok(())
    }
    fn base(&self) -> &Base<'_> {
        &self.base
    }

    fn account(&self, key: &Pubkey) -> Result<Account> {
        let bind = self.info_external(key, false)?;
        Ok(bind.1)
    }

    fn with_account_info<F, R>(&self, _key: &Pubkey, _f: F) -> Result<R>
    where
        F: FnOnce(&AccountInfo) -> Result<R>,
    {
        // Emulator does not surface raw `AccountInfo` references: its account-binding model
        // constructs `AccountInfo` on demand from RPC-fetched data and they don't outlive the
        // binding. Non-EVM precompile paths that need this method (e.g. Sysvar::Instructions
        // introspection in `IEd25519`) short-circuit BEFORE the on-chain primitive runs (see
        // `2026-05-19-rome-ed25519-precompile.md` §8.3), so the closure is never invoked in
        // emulator simulation. Surfacing as `Unimplemented` keeps the contract explicit:
        // any future emulator path that DOES reach this method will fail loudly rather than
        // silently produce wrong results.
        Err(Unimplemented(
            "emulator does not surface raw AccountInfo for non-evm precompiles".to_string(),
        ))
    }

    fn invoke_signed(&self, ix: &Instruction, _: Vec<Seed>, refund_to_signer: bool) -> Result<()> {
        let keys = self.fetch_accs(ix)?;
        let signer = &self.signer.unwrap();

        let store = {
            let accs = self.accounts.borrow();
            ix_store(ix, &accs)
        };
        let elf = &self.upgradeable_elf.borrow();
        let mollusk = Mollusk::new(store, elf)?;

        let len_old = self.data_len(&keys)?;
        let lmp_old = self.lamports(&signer)?;

        let res = mollusk.execute_with_sysvar(ix, None)?;
        if !Mollusk::is_success(&res) {
            return Err(SimulateTransactionError(decode_cpi_failure(
                &ix.program_id,
                &res.program_result,
            )));
        }

        self.update_accs(res.resulting_accounts);

        let len_new = self.data_len(&keys)?;
        let lmp_new = self.lamports(&signer)?;

        if refund_to_signer {
            if lmp_old > lmp_new {
                self.add_fee(lmp_old - lmp_new)?;
            } else {
                self.add_refund(lmp_new - lmp_old)?;
            }
        }

        let alloc = len_diff(&len_new, &len_old);
        let dealloc = len_diff(&len_old, &len_new);
        let refund = refund_to_signer && (lmp_new < lmp_old);
        self.inc_space_counter(alloc, dealloc, refund)?;
        self.syscall.inc();

        Ok(())
    }
    fn invoke_signed_unchecked(&self, ix: &Instruction, _: Vec<Seed>) -> Result<()> {
        let _ = self.info_program(&ix.program_id)?;
        for meta in ix.accounts.iter() {
            let _ = self.info_external(&meta.pubkey, meta.is_writable)?;
        }

        self.syscall.inc();
        Ok(())
    }

    fn signer(&self) -> Pubkey {
        self.signer.unwrap()
    }
    fn wallet(&self) -> Result<Pubkey> {
        let bind = self.info_sol_wallet(false)?;
        Ok(bind.0)
    }
    fn treasure(&self, id: u64) -> Result<Pubkey> {
        let bind = self.info_treasure(id, true)?; // TODO: set false
        Ok(bind.0)
    }
    fn owner(&self, key: &Pubkey) -> Result<Pubkey> {
        let bind = self.info_external(key, false)?;

        Ok(bind.1.owner)
    }
    fn ed25519_data(&self) -> Result<Vec<u8>> {
        let data = self
            .ed25519_ix
            .clone()
            .ok_or(Ed25519IxNotFound)?
            .data;
        Ok(data)
    }
}

fn len_diff(a: &Vec<usize>, b: &Vec<usize>) -> usize{
    a
        .iter()
        .zip(b)
        .map(|(&x, &y)| x.saturating_sub(y))
        .collect::<Vec<usize>>()
        .iter()
        .sum()
}

fn decode_cpi_failure(program_id: &Pubkey, result: &ProgramResult) -> String {
    match program_name(program_id) {
        Some(name) => format!("mollusk error: {result:?} [program {program_id} ({name})]"),
        None => format!("mollusk error: {result:?} [program {program_id}]"),
    }
}

/// Canonical name for the well-known programs a Rome CPI can target. Pure pubkey
/// lookup — factual, never a guess.
fn program_name(program_id: &Pubkey) -> Option<&'static str> {
    if *program_id == system_program::ID {
        Some("System Program")
    } else if *program_id == spl_token_interface::ID {
        Some("SPL Token")
    } else if *program_id == spl_token_2022_interface::ID {
        Some("Token-2022")
    } else if *program_id == spl_associated_token_account_interface::program::ID {
        Some("Associated Token Account")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use {
        super::decode_cpi_failure,
        mollusk_svm_result::types::ProgramResult,
        solana_program::{program_error::ProgramError, pubkey::Pubkey},
        solana_system_interface::program as system_program,
    };

    // The message must IDENTIFY which program's CPI failed (System vs SPL vs
    // rome-evm-Custom(0)) — the real disambiguation — while preserving the raw
    // `Failure(Custom(n))`. It must NOT assert a cause: a Custom(1) from System
    // or SPL can be the user's own direct CPI (Rome exposes System/SPL via the
    // Solidity precompile interface), not a proxy/payer issue.
    const CAUSAL_PHRASES: [&str; 4] =
        ["ResultWithNegativeLamports", "InsufficientFunds", "payer pool", "blind spot"];

    #[test]
    fn system_failure_names_the_program_and_keeps_raw_code() {
        let result = ProgramResult::Failure(ProgramError::Custom(1));
        let msg = decode_cpi_failure(&system_program::ID, &result);
        assert!(msg.contains("Failure(Custom(1))"), "raw code must be preserved: {msg}");
        assert!(msg.contains(&system_program::ID.to_string()), "must name the program id: {msg}");
        assert!(msg.contains("System Program"), "factual program name: {msg}");
        for p in CAUSAL_PHRASES {
            assert!(!msg.contains(p), "must not assert a cause ({p}): {msg}");
        }
    }

    #[test]
    fn spl_token_failure_names_the_program_and_keeps_raw_code() {
        let result = ProgramResult::Failure(ProgramError::Custom(1));
        let msg = decode_cpi_failure(&spl_token_interface::ID, &result);
        assert!(msg.contains("Failure(Custom(1))"), "raw code must be preserved: {msg}");
        assert!(msg.contains(&spl_token_interface::ID.to_string()), "must name the program id: {msg}");
        assert!(msg.contains("SPL Token"), "factual program name: {msg}");
        for p in CAUSAL_PHRASES {
            assert!(!msg.contains(p), "must not assert a cause ({p}): {msg}");
        }
    }

    // Unknown program: still append its id (the useful fact), no name, no cause.
    #[test]
    fn unknown_program_appends_id_without_name_or_cause() {
        let id = Pubkey::new_unique();
        let result = ProgramResult::Failure(ProgramError::Custom(7));
        let msg = decode_cpi_failure(&id, &result);
        assert!(msg.contains("Failure(Custom(7))"), "raw code preserved: {msg}");
        assert!(msg.contains(&id.to_string()), "unknown program id still appended: {msg}");
        assert_eq!(msg, format!("mollusk error: {result:?} [program {id}]"));
    }
}


