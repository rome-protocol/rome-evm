use {
    super::{AccountLock, ContextIt},
    crate::{
        error::{Result, RomeProgramError::*, },
        Data, Lock, LockType, msg,
    },
    solana_program::{
        account_info::AccountInfo, pubkey::Pubkey,
    },
};

impl AccountLock for ContextIt<'_, '_> {
    fn lock(&self) -> Result<()> {
        // TODO: there may be allocation and deallocation together (ro_lock)
        for info in self.state.all().values() {
            if Lock::is_managed(info, self.state.program_id)? {
                let mut lock = Lock::from_account_mut(info)?;
                // writable lock is required
                if let Some(lock) = lock.get()? {
                    return Err(AccountLocked(*info.key, Some(lock)));
                }
                // add rw-lock
                lock.rw_lock(self.state_holder.key, self.alt)?;
            }
        }
        Ok(())
    }
    fn update_lock(&self) -> Result<()> {
        for info in self.state.all().values() {
            if Lock::is_managed(info, self.state.program_id)? {
                let mut lock = Lock::from_account_mut(info)?;
                match lock.get()? {
                    None => return Err(AccountLockNotFound(*info.key, None)),
                    Some(LockType::Rw(holder)) => {
                        if holder != self.state_holder.key.to_bytes() {
                            return Err(AccountLockNotFound(*info.key, Some(LockType::Rw(holder))))
                        }
                    }
                }
                lock.update(self.alt)?;
            }
        }

        Ok(())
    }
    fn unlock(&self) -> Result<()> {
        for info in self.state.all().values() {
            if Lock::is_managed(info, self.state.program_id)? {
                let mut lock = Lock::from_account_mut(info)?;

                match lock.get()? {
                    Some(LockType::Rw(holder)) => {
                        // before Unlock iteration the account lock might be expired
                        // and account might be locked by another transaction
                        if holder == self.state_holder.key.to_bytes() {
                            lock.unlock()
                        } else {
                            let key = Pubkey::from(holder);
                            msg!(
                                "account {} is rw-locked by another holder {}",
                                info.key,
                                key
                            );
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }
    fn lock_new_one(&self, info: &AccountInfo) -> Result<()> {
        if Lock::is_managed(info, self.state.program_id)? {
            let mut lock = Lock::from_account_mut(info)?;
            if lock.is_new_one() {
                lock.rw_lock(self.state_holder.key, self.alt)?;
            }
        }

        Ok(())
    }
    /// this is not lock checking (lock can be expired).
    /// It is just checking the type of lock, that was checked earlier.
    fn check_writable(&self, _info: &AccountInfo) -> Result<()> {
        // read-only lock is not implemented
        Ok(())
    }
}
