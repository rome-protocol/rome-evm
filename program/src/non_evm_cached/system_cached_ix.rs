use {
    solana_program::{
        pubkey::Pubkey, instruction::Instruction,
        rent::Rent, sysvar::Sysvar,
    },
    solana_system_interface::{
        program as system_program,
        instruction::{
            create_account, allocate, assign, transfer,
        }
    },
    crate::{
        error::{Result, RomeProgramError::*,}, origin::Origin,
        pda::Seed, non_evm::aux::*,
        handler::CallInterrupt,
        non_evm_cached::{CachedMut, CachedState,}, NonEvmState,
    },
    solana_program::account_info::MAX_PERMITTED_DATA_INCREASE,
};

#[derive(Clone)]
pub struct SystemCachedReader<'a, T: Origin> {
    pub cached: CachedState<'a, T>,
}

impl<'a, T: Origin> SystemCachedReader<'a, T> {
    pub fn new(state: &'a T, ne_state: Option<&'a NonEvmState>) -> Self {
        Self {
            cached: CachedState::new(state, ne_state)
        }
    }
    pub fn create_pda(&self, params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (pda, seed) = self
            .cached
            .base()
            .pda
            .external_auth(&params.context.caller);

        let rent = Rent::get()?.minimum_balance(0);
        let ix = create_account(&self.cached.signer(), &pda, rent, 0, &system_program::ID);

        Ok((ix, vec![seed]))
    }
    pub fn create_pda_lamports(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (lamports, _) = get_u64(abi)?;
        let (pda, seed) = self
            .cached
            .base()
            .pda
            .external_auth(&params.context.caller);

        let rent = Rent::get()?.minimum_balance(0);
        if lamports < rent {
            return Err(NonEvmCallError(format!("insufficient lamports to create rent-exempt account {}", lamports)))
        }
        let ix = create_account(&self.cached.signer(), &pda, lamports, 0, &system_program::ID);

        Ok((ix, vec![seed]))
    }
    pub fn create_pda_lamports_salt(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (lamports, salt) = get_u64(abi)?;
        len_eq!(salt, 32);
        let (pda, seed) = self.cached.base().pda.external_auth_with_salt(&params.context.caller, salt);

        let rent = Rent::get()?.minimum_balance(0);
        if lamports < rent {
            return Err(NonEvmCallError(format!("insufficient lamports to create rent-exempt account {}", lamports)))
        }
        let ix = create_account(&self.cached.signer(), &pda, lamports, 0, &system_program::ID);

        Ok((ix, vec![seed]))
    }
    pub fn create_pda_owner_len_salt(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (owner, right) = get_pubkey(abi)?;
        let (len, salt) = get_u64(right)?;
        len_eq!(salt, 32);
        let (pda, seed) = self.cached.base().pda.external_auth_with_salt(&params.context.caller, salt);

        let rent = Rent::get()?.minimum_balance(len as usize);
        let ix = create_account(&self.cached.signer(), &pda, rent, len, &owner);

        Ok((ix, vec![seed]))
    }
    pub fn allocate(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (len, salt) = get_u64(abi)?;
        len_eq!(salt, 32);
        let (pda, seed) = self.cached.base().pda.external_auth_with_salt(&params.context.caller, salt);
        let ix = allocate(&pda, len);

        Ok((ix, vec![seed]))
    }
    pub fn assign(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (owner, salt) = get_pubkey(abi)?;
        len_eq!(salt, 32);
        let (pda, seed) = self.cached.base().pda.external_auth_with_salt(&params.context.caller, salt);
        let ix = assign(&pda, &owner);

        Ok((ix, vec![seed]))
    }
    pub fn transfer(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to_addr, right) = get_h160(abi)?;
        let (lamports, _) = get_u64(right)?;
        let (from, seed) = self.cached.base().pda.external_auth(&params.context.caller);
        let (to, _) = self.cached.base().pda.external_auth(&to_addr);
        let ix = transfer(&from, &to, lamports);

        Ok((ix, vec![seed]))
    }
    pub fn transfer_b32(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to, right) = get_pubkey(abi)?;
        let (lamports, _) = get_u64(right)?;
        let (from, seed) = self.cached.base().pda.external_auth(&params.context.caller);
        let ix = transfer(&from, &to, lamports);

        Ok((ix, vec![seed]))
    }
   pub fn transfer_b32_salt(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (to, right) = get_pubkey(abi)?;
        let (lamports, salt) = get_u64(right)?;
       len_eq!(salt, 32);
       let (from, seed) = self.cached.base().pda.external_auth_with_salt(&params.context.caller, salt);
       let ix = transfer(&from, &to, lamports);

        Ok((ix, vec![seed]))
    }
}

pub struct SystemCachedWriter<'a,  T: Origin> {
    pub cached_mut: CachedMut<'a,  T>
}

impl<'a,  T: Origin> SystemCachedWriter<'a, T> {
    pub fn new(state: &'a T, nes: &'a mut NonEvmState) -> Self {
        Self {
            cached_mut: CachedMut::new(state, nes)
        }
    }
    pub fn from_cached_state(cached_mut: CachedMut<'a, T>) -> Self {
        Self {
            cached_mut
        }
    }
    pub fn create_account(&mut self, ix: &Instruction, lamports: u64, len: u64, owner: &Pubkey) -> Result<()> {
        let iter  = &mut ix.accounts.iter();
        let signer = next(iter)?;
        let new = next(iter)?;

        {
            let acc = self.cached_mut.get_mut(&signer)?;
            acc.lamports = acc
                .lamports
                .checked_sub(lamports)
                .ok_or(InsufficientLamports(signer, lamports))?;
        }

        let new_ = self.cached_mut.get_mut(&new)?;
        if !(new_.lamports == 0 && new_.data.is_empty() && system_program::ID == new_.owner) {
            return Err(AccountAlreadyExists(new))
        }

        new_.lamports = lamports;
        new_.data.resize(len as usize, 0);
        new_.owner = *owner;

        Ok(())
    }
    pub fn allocate(&mut self, ix: &Instruction, len: u64) -> Result<()> {
        let iter  = &mut ix.accounts.iter();
        let key = next(iter)?;
        let acc = self.cached_mut.get_mut(&key)?;
        let diff = len.saturating_sub(acc.data.len() as u64 );
        if diff > MAX_PERMITTED_DATA_INCREASE as u64 {
            return Err(NonEvmCallError(format!("account allocation {} exceeds limit  {}", key, diff)))
        }

        if !acc.data.is_empty() || acc.owner != system_program::ID {
            return Err(AccountAlreadyInUse(key))
        }
        acc.data.resize(len as usize, 0);

        Ok(())
    }
    pub fn assign(&mut self, ix: &Instruction, owner: &Pubkey) -> Result<()> {
        let iter  = &mut ix.accounts.iter();
        let key = next(iter)?;
        let acc = self.cached_mut.get_mut(&key)?;
        let non_zeroed = acc.data.iter().any(|x| *x != 0 );

        if acc.owner != system_program::ID || non_zeroed {
            return Err(AccountAlreadyInUse(key))
        }

        acc.owner = *owner;

        Ok(())
    }
    pub fn transfer(&mut self, ix: &Instruction, lamports: u64) -> Result<()> {
        let iter  = &mut ix.accounts.iter();
        let from = next(iter)?;
        let to = next(iter)?;

        let from_ = self.cached_mut.get_mut(&from)?;
        if from_.owner != system_program::ID {
            return Err(NonEvmCallError(format!("transfer(): account isn't owned by SystemProgram {}", from)))
        }

        if !from_.data.is_empty() {
            return Err(TransferFromAccountWithData(from))
        }

        from_.lamports = from_
            .lamports
            .checked_sub(lamports)
            .ok_or(InsufficientLamports(from, lamports))?;

        let to_ = self.cached_mut.get_mut(&to)?;
        to_.lamports = to_
            .lamports
            .checked_add(lamports)
            .ok_or(CalculationOverflow)?;

        Ok(())
    }
}





