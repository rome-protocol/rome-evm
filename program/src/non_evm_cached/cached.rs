use {
    crate::{
        error::Result, RomeProgramError::*, origin::Origin, state::{Account, base::Base,},
        state::NonEvmState,
    },
    solana_program::pubkey::Pubkey,
};

#[derive(Clone, )]
pub struct CachedState<'a, T: Origin> {
    state: &'a T,
    ne_state: Option<&'a NonEvmState>,
}
impl <'a, T: Origin> CachedState <'a, T> {
    pub fn new(state: &'a T, ne_state: Option<&'a NonEvmState>) -> Self {
        Self {
            state,
            ne_state,
        }
    }
    /// The mint's facts, through the overlay. The one mint read on this track —
    /// callers that need only the program and decimals project from it, so there
    /// is never a second reader to keep in step (S1: facts travel whole).
    pub fn mint_profile(&self, mint: &Pubkey) -> Result<crate::state::mint_profile::MintProfile> {
        let acc = self.account(mint)?;

        if acc.lamports == 0 {
            return Err(InvalidSplMintAccount(*mint));
        }
        crate::state::mint_profile::profile(&acc.owner, &acc.data)
            .map_err(|_| InvalidSplMintAccount(*mint))
    }
    pub fn account(&self, key: &Pubkey) -> Result<Account> {
        if self.ne_state.is_none() {
            return self.state.account(&key)
        }

        if let Some(acc ) = self.ne_state.unwrap().get(&key) {
            Ok(acc.clone())
        } else {
            self.state.account(&key)
        }
    }
    pub fn owner(&self, key: &Pubkey) -> Result<Pubkey> {
        if self.ne_state.is_none() {
            return self.state.owner(&key)
        }

        if let Some(acc ) = self.ne_state.unwrap().get(&key) {
            Ok(acc.owner)
        } else {
            self.state.owner(&key)
        }
    }
    pub fn signer(&self) -> Pubkey {
        self.state.signer()
    }
    pub fn base (&self) -> &Base<'_> {
        self.state.base()
    }
    pub fn mint(&self) -> Result<Pubkey> {
        self
            .base()
            .owner_info()
            .mint
            .ok_or(NonEvmCallError("solidify function is only applicable for a rollup based on SPL".to_string()))
    }
}