use {
    crate::{
        context::AccountLock, error::Result, origin::Origin,
        state::{base::Base, pda::Seed, Account, Allocate}, H160, U256,
    },
    solana_program::{account_info::AccountInfo, instruction::Instruction, pubkey::Pubkey, rent::Rent},
    solana_system_interface::program as system_program,
    std::collections::HashMap,
};

pub const USDC_LEGACY: &[u8] = include_bytes!("../../tests/fixtures/usdc_legacy.bin");

/// Origin double over a fixed account map; unknown keys read as empty
/// system-owned accounts, exactly as a never-created PDA does.
pub struct MapState {
    signer: Pubkey,
    accounts: HashMap<Pubkey, Account>,
}

impl MapState {
    pub fn new(signer: Pubkey, signer_lamports: u64) -> Self {
        let mut accounts = HashMap::new();
        accounts.insert(signer, system_account(signer_lamports));
        Self { signer, accounts }
    }
    pub fn with(mut self, key: Pubkey, acc: Account) -> Self {
        self.accounts.insert(key, acc);
        self
    }
}

pub fn system_account(lamports: u64) -> Account {
    Account { lamports, data: vec![], owner: system_program::ID, executable: false, writable: false, signer: false }
}

pub fn legacy_mint() -> Account {
    Account {
        lamports: 1,
        data: USDC_LEGACY.to_vec(),
        owner: spl_token_interface::ID,
        executable: false,
        writable: false,
        signer: false,
    }
}

struct RentStub;
impl solana_sysvar::program_stubs::SyscallStubs for RentStub {
    fn sol_get_rent_sysvar(&self, var_addr: *mut u8) -> u64 {
        unsafe { std::ptr::write(var_addr as *mut Rent, Rent::default()) };
        0
    }
}

/// Serve the default `Rent` to both host sysvar registries: the one
/// `solana_program` 4.x reads and the one the vendored token processor reads.
pub fn install_rent() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let bytes = bincode::serde::encode_to_vec(Rent::default(), bincode::config::legacy()).unwrap();
        solana_get_sysvar::set_sysvar_bytes(solana_program::sysvar::rent::ID, bytes);
        solana_sysvar::program_stubs::set_syscall_stubs(Box::new(RentStub));
    });
}

impl Origin for MapState {
    fn nonce(&self, _: &H160) -> Result<Option<u64>> { unimplemented!() }
    fn balance(&self, _: &H160) -> Result<Option<U256>> { unimplemented!() }
    fn code(&self, _: &H160) -> Result<Option<Vec<u8>>> { unimplemented!() }
    fn valids(&self, _: &H160) -> Result<Option<Vec<u8>>> { unimplemented!() }
    fn storage(&self, _: &H160, _: &U256) -> Result<Option<U256>> { unimplemented!() }
    fn inc_nonce<L: AccountLock>(&self, _: &H160, _: &L) -> Result<()> { unimplemented!() }
    fn add_balance<L: AccountLock>(&self, _: &H160, _: &U256, _: &L) -> Result<()> { unimplemented!() }
    fn sub_balance<L: AccountLock>(&self, _: &H160, _: &U256, _: &L) -> Result<()> { unimplemented!() }
    fn set_code<L: AccountLock>(&self, _: &H160, _: &[u8], _: &[u8], _: &L) -> Result<()> { unimplemented!() }
    fn set_storage<L: AccountLock>(&self, _: &H160, _: &U256, _: &U256, _: &L) -> Result<()> { unimplemented!() }
    fn base(&self) -> &Base<'_> { unimplemented!() }
    fn account(&self, key: &Pubkey) -> Result<Account> {
        Ok(self.accounts.get(key).cloned().unwrap_or_else(|| system_account(0)))
    }
    fn with_account_info<F, R>(&self, _: &Pubkey, _: F) -> Result<R>
    where
        F: FnOnce(&AccountInfo) -> Result<R>,
    {
        unimplemented!()
    }
    fn invoke_signed(&self, _: &Instruction, _: Vec<Seed>, _: bool) -> Result<()> { unimplemented!() }
    fn invoke_signed_unchecked(&self, _: &Instruction, _: Vec<Seed>) -> Result<()> { unimplemented!() }
    fn signer(&self) -> Pubkey { self.signer }
    fn wallet(&self) -> Result<Pubkey> { unimplemented!() }
    fn treasure(&self, _: u64) -> Result<Pubkey> { unimplemented!() }
    fn owner(&self, key: &Pubkey) -> Result<Pubkey> { Ok(self.account(key)?.owner) }
    fn ed25519_data(&self) -> Result<Vec<u8>> { unimplemented!() }
}

impl Allocate for MapState {
    fn alloc_balance<L: AccountLock>(&self, _: &H160, _: &L) -> Result<()> { unimplemented!() }
    fn alloc_slots<L: AccountLock>(&self, _: &Pubkey, _: &Seed, _: usize, _: &L, _: &H160) -> Result<bool> { unimplemented!() }
    fn alloc_slots_unchecked(&self, _: &Pubkey, _: &Seed, _: usize, _: &H160) -> Result<()> { unimplemented!() }
    fn alloc_contract<L: AccountLock>(&self, _: &H160, _: &[u8], _: &[u8], _: &L) -> Result<bool> { unimplemented!() }
}
