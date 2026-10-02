use {
    crate::{
        error::Result, RomeProgramError::*, origin::Origin, state::Account, state::NonEvmState,
    },
    solana_program::{
        instruction::{Instruction},
        pubkey::Pubkey,
        account_info::{AccountInfo, IntoAccountInfo},
    },
};

pub type Bind<'a> = (&'a Pubkey, &'a mut Account);

pub struct CachedMut<'a,  T: Origin> {
    pub nes: &'a mut NonEvmState,
    pub state: &'a T,
}
impl <'a, T: Origin> CachedMut<'a, T> {
    pub fn new(state: &'a T, nes: &'a mut NonEvmState) -> Self {
        Self {
            nes: nes,
            state,
        }
    }
    pub fn get_mut(&mut self, key: &Pubkey) -> Result<&mut Account> {
        self.add(key)?;
        self
            .nes
            .iter_mut()
            .find(|(key_, _)| **key_ == *key)
            .map(|(_, acc)| acc)
            .ok_or(InconsistentAccountList)
    }
    pub fn get_ref(&mut self, key: &Pubkey) -> Result<&Account> {
        self.add(key)?;
        self
            .nes
            .iter()
            .find(|(key_, _)| **key_ == *key)
            .map(|(_, acc)| acc)
            .ok_or(InconsistentAccountList)
    }
    pub fn infos(&mut self, ix: &Instruction) -> Result<Vec<AccountInfo<'_>>> {
        self.add_all(ix)?;
        let iter_mut = self.nes.iter_mut();
        let mut binds = Self::filter(iter_mut, ix)?;

        let mut infos: Vec<AccountInfo> = vec![];

        for key in ix.accounts.iter().map(|x| x.pubkey) {
            let info = if let Some(info) = infos.iter().find(|&x| *x.key == key) {
                info.clone()
            } else {
                let pos = binds
                    .iter()
                    .position(|(key_, _)| **key_ == key)
                    .ok_or(InconsistentAccountList)?;
                binds
                    .swap_remove(pos)
                    .into_account_info()
            };
            infos.push(info);
        }

        Ok(infos)
    }
    fn add(&mut self, key: &Pubkey) -> Result<()> {
        if !self.nes.contains_key(key) {
            let acc = self.state.account(key)?;
            self.nes.insert(*key, acc);
        };
        Ok(())
    }
    fn add_all(&mut self, ix: &Instruction) -> Result<()> {
        ix
            .accounts
            .iter()
            .map(|m| self.add(&m.pubkey))
            .collect::<Result<Vec<_>>>()?;

        Ok(())
    }
    fn filter<'c, I: Iterator<Item = Bind<'c>>>(
        iter_mut: I,
        ix: &Instruction
    ) -> Result<Vec<Bind<'c>>> {
        let vec = iter_mut
            .filter(|(&key, _)|
                ix.accounts.iter().any(|m| m.pubkey == key)
            )
            .collect::<Vec<_>>();

        Ok(vec)
    }
}

