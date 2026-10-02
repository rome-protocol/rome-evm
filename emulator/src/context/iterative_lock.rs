use {
    crate::context::ContextIt,
    rome_evm::{
        accounts::{Data, Lock},
        context::AccountLock,
        error::{Result, RomeProgramError::*},
    },
    solana_program::{
        account_info::{AccountInfo, IntoAccountInfo}, pubkey::Pubkey,
    },
};

pub fn add_rw_lock(info: &AccountInfo, state_holder: &Pubkey) -> Result<()> {
    let mut lock = Lock::from_account_mut(info)?;

    if let Some(lock) = lock.get()? {
        return Err(AccountLocked(*info.key, Some(lock)));
    }
    // add rw-lock
    lock.rw_lock(&state_holder, false)
}

impl AccountLock for ContextIt<'_, '_> {
    fn lock(&self) -> Result<()> {
        let accs = self.state.accounts.borrow().clone();

        for (key, item) in accs.iter() {
            let mut bind = (*key, item.account.clone());
            let info = bind.into_account_info();

            if Lock::is_managed(&info, self.state.program_id)? {
                // TODO: enable ro-lock after the ALT is implemented
                add_rw_lock(&info, &self.state_holder_key)?;
                // all accounts must be writable
                self.state.update(bind);
            }
        }

        Ok(())
    }
    fn update_lock(&self) -> Result<()> {
        // during transaction emulation accounts are not locked
        Ok(())
    }
    fn unlock(&self) -> Result<()> {
        // it doesn't make sense for emulation
        Ok(())
    }
    fn lock_new_one(&self, _info: &AccountInfo) -> Result<()> {
        unreachable!()
    }
    fn check_writable(&self, _info: &AccountInfo) -> Result<()> {
        Ok(())
    }
}
