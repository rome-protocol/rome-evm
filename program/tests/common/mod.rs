// Shared test double for handler-level tests. Only `base()` is real; every
// other Origin/Allocate method is unreachable from the scenarios that use it.

use {
    evm::{H160, U256},
    rome_evm::{
        context::AccountLock,
        error::Result as CrateResult,
        state::{base::Base, origin::Origin, pda::Seed, Allocate},
        Account,
    },
    solana_program::{account_info::AccountInfo, instruction::Instruction, pubkey::Pubkey},
};

pub struct DummyState<'a> {
    base: Base<'a>,
}

impl<'a> DummyState<'a> {
    pub fn new(program_id: &'a Pubkey) -> Self {
        Self { base: Base::new(program_id, 1, None) }
    }
}

impl Origin for DummyState<'_> {
    fn nonce(&self, _: &H160) -> CrateResult<Option<u64>> { unimplemented!() }
    fn balance(&self, _: &H160) -> CrateResult<Option<U256>> { unimplemented!() }
    fn code(&self, _: &H160) -> CrateResult<Option<Vec<u8>>> { unimplemented!() }
    fn valids(&self, _: &H160) -> CrateResult<Option<Vec<u8>>> { unimplemented!() }
    fn storage(&self, _: &H160, _: &U256) -> CrateResult<Option<U256>> { unimplemented!() }
    fn inc_nonce<L: AccountLock>(&self, _: &H160, _: &L) -> CrateResult<()> { unimplemented!() }
    fn add_balance<L: AccountLock>(&self, _: &H160, _: &U256, _: &L) -> CrateResult<()> { unimplemented!() }
    fn sub_balance<L: AccountLock>(&self, _: &H160, _: &U256, _: &L) -> CrateResult<()> { unimplemented!() }
    fn set_code<L: AccountLock>(&self, _: &H160, _: &[u8], _: &[u8], _: &L) -> CrateResult<()> { unimplemented!() }
    fn set_storage<L: AccountLock>(&self, _: &H160, _: &U256, _: &U256, _: &L) -> CrateResult<()> { unimplemented!() }
    fn base(&self) -> &Base<'_> { &self.base }
    fn account(&self, _: &Pubkey) -> CrateResult<Account> { unimplemented!() }
    fn with_account_info<F, R>(&self, _: &Pubkey, _: F) -> CrateResult<R>
    where F: FnOnce(&AccountInfo) -> CrateResult<R> { unimplemented!() }
    fn invoke_signed(&self, _: &Instruction, _: Vec<Seed>, _: bool) -> CrateResult<()> { unimplemented!() }
    fn invoke_signed_unchecked(&self, _: &Instruction, _: Vec<Seed>) -> CrateResult<()> { unimplemented!() }
    fn signer(&self) -> Pubkey { unimplemented!() }
    fn wallet(&self) -> CrateResult<Pubkey> { unimplemented!() }
    fn treasure(&self, _: u64) -> CrateResult<Pubkey> { unimplemented!() }
    fn owner(&self, _: &Pubkey) -> CrateResult<Pubkey> { unimplemented!() }
    fn ed25519_data(&self) -> CrateResult<Vec<u8>> { unimplemented!() }
}

impl Allocate for DummyState<'_> {
    fn alloc_balance<L: AccountLock>(&self, _: &H160, _: &L) -> CrateResult<()> { unimplemented!() }
    fn alloc_slots<L: AccountLock>(&self, _: &Pubkey, _: &Seed, _: usize, _: &L, _: &H160) -> CrateResult<bool> { unimplemented!() }
    fn alloc_slots_unchecked(&self, _: &Pubkey, _: &Seed, _: usize, _: &H160) -> CrateResult<()> { unimplemented!() }
    fn alloc_contract<L: AccountLock>(&self, _: &H160, _: &[u8], _: &[u8], _: &L) -> CrateResult<bool> { unimplemented!() }
}

pub fn install_clock_stub() {
    let clock_bytes = bincode::serde::encode_to_vec(
        solana_program::clock::Clock::default(),
        bincode::config::legacy(),
    )
    .expect("Clock encodes");
    solana_get_sysvar::set_sysvar_bytes(solana_program::sysvar::clock::ID, clock_bytes);
}
