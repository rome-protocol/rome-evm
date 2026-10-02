//! The gas-pool ATA a chain registration creates must be created idempotently:
//! ATA creation is permissionless and the destination is publicly derivable, so
//! a pre-created ATA must not be able to make `RegOwner` fail for that mint.

use {
    mollusk_svm::{program as mprog, Mollusk},
    rome_evm::{
        api::reg_owner::create_spl_wallet,
        context::AccountLock,
        error::Result,
        origin::Origin,
        state::{base::Base, pda::Seed, Account as RomeAccount},
        H160, U256,
    },
    solana_account::Account,
    solana_program::{
        instruction::Instruction, program_option::COption, program_pack::Pack, pubkey::Pubkey,
    },
    spl_associated_token_account_interface::address::get_associated_token_address_with_program_id,
    spl_token::state::{Account as TokenAccount, AccountState, Mint},
    std::{cell::RefCell, collections::HashMap},
};

const TOKENKEG: Pubkey = spl_token_interface::ID;
const ATA_PROG: Pubkey = spl_associated_token_account_interface::program::ID;

/// Origin double that answers the two reads `create_spl_wallet` makes and
/// records the instruction it hands to `invoke_signed`.
struct Recorder {
    signer: Pubkey,
    recorded: RefCell<Option<Instruction>>,
}

impl Origin for Recorder {
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
    fn account(&self, _: &Pubkey) -> Result<RomeAccount> { unimplemented!() }
    fn with_account_info<F, R>(&self, _: &Pubkey, _: F) -> Result<R>
    where
        F: FnOnce(&solana_program::account_info::AccountInfo) -> Result<R>,
    {
        unimplemented!()
    }
    fn invoke_signed(&self, ix: &Instruction, _: Vec<Seed>, _: bool) -> Result<()> {
        *self.recorded.borrow_mut() = Some(ix.clone());
        Ok(())
    }
    fn invoke_signed_unchecked(&self, _: &Instruction, _: Vec<Seed>) -> Result<()> { unimplemented!() }
    fn signer(&self) -> Pubkey { self.signer }
    fn wallet(&self) -> Result<Pubkey> { unimplemented!() }
    fn treasure(&self, _: u64) -> Result<Pubkey> { unimplemented!() }
    fn owner(&self, _: &Pubkey) -> Result<Pubkey> { Ok(TOKENKEG) }
    fn ed25519_data(&self) -> Result<Vec<u8>> { unimplemented!() }
}

fn elf(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/elf/{name}.so", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn mollusk() -> Mollusk {
    let mut m = Mollusk::default();
    m.add_program_with_loader_and_elf(&TOKENKEG, &solana_sdk_ids::bpf_loader_upgradeable::id(), &elf("tokenkeg"));
    m.add_program_with_loader_and_elf(&ATA_PROG, &solana_sdk_ids::bpf_loader::id(), &elf("ata"));
    m
}

fn system_account(lamports: u64) -> Account {
    Account { lamports, data: vec![], owner: solana_sdk_ids::system_program::ID, executable: false, rent_epoch: 0 }
}

fn mint_account(m: &Mollusk) -> Account {
    let mut data = vec![0u8; Mint::LEN];
    Mint {
        mint_authority: COption::Some(Pubkey::new_unique()),
        supply: 0,
        decimals: 6,
        is_initialized: true,
        freeze_authority: COption::None,
    }
    .pack_into_slice(&mut data);
    Account { lamports: m.sysvars.rent.minimum_balance(data.len()), data, owner: TOKENKEG, executable: false, rent_epoch: 0 }
}

/// An ATA for (`wallet`, `mint`) that a third party already created.
fn initialized_ata(m: &Mollusk, wallet: &Pubkey, mint: &Pubkey) -> Account {
    let mut data = vec![0u8; TokenAccount::LEN];
    TokenAccount {
        mint: *mint,
        owner: *wallet,
        amount: 0,
        delegate: COption::None,
        state: AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    }
    .pack_into_slice(&mut data);
    Account { lamports: m.sysvars.rent.minimum_balance(data.len()), data, owner: TOKENKEG, executable: false, rent_epoch: 0 }
}

struct World {
    m: Mollusk,
    payer: Pubkey,
    wallet: Pubkey,
    mint: Pubkey,
    ata: Pubkey,
    pre: HashMap<Pubkey, Account>,
}

fn world() -> World {
    let m = mollusk();
    let (payer, wallet, mint) = (Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique());
    let ata = get_associated_token_address_with_program_id(&wallet, &mint, &TOKENKEG);
    let mut pre = HashMap::new();
    pre.insert(payer, system_account(1_000_000_000));
    pre.insert(wallet, system_account(1_000_000));
    pre.insert(mint, mint_account(&m));
    let (sk, sa) = mprog::keyed_account_for_system_program();
    pre.insert(sk, sa);
    pre.insert(TOKENKEG, mprog::create_program_account_loader_v3(&TOKENKEG));
    World { m, payer, wallet, mint, ata, pre }
}

impl World {
    /// Emit the wallet-creation instruction exactly as `reg_owner` does and
    /// run it through the deployed ATA program over this world's pre-state.
    fn register(&self) -> mollusk_svm::result::InstructionResult {
        let state = Recorder { signer: self.payer, recorded: RefCell::new(None) };
        create_spl_wallet(&state, Some(self.mint), &self.wallet).unwrap();
        let ix = state.recorded.into_inner().expect("reg_owner must create the gas-pool ATA");
        assert_eq!(ix.program_id, ATA_PROG);
        assert_eq!(ix.accounts[1].pubkey, self.ata);

        let accs: Vec<(Pubkey, Account)> = ix
            .accounts
            .iter()
            .map(|a| (a.pubkey, self.pre.get(&a.pubkey).cloned().unwrap_or_default()))
            .collect();
        self.m.process_instruction(&ix, &accs)
    }
}

#[test]
fn a_pre_created_gas_pool_ata_does_not_block_registration() {
    let mut w = world();
    let squatted = initialized_ata(&w.m, &w.wallet, &w.mint);
    w.pre.insert(w.ata, squatted.clone());

    let res = w.register();

    assert!(!res.program_result.is_err(), "registration failed on a pre-created ATA: {:?}", res.program_result);
    let after = res.resulting_accounts.iter().find(|(k, _)| *k == w.ata).unwrap().1.clone();
    assert_eq!(after.data, squatted.data, "an existing ATA must be left untouched");
}

#[test]
fn a_fresh_gas_pool_ata_is_created() {
    let w = world();

    let res = w.register();

    assert!(!res.program_result.is_err(), "{:?}", res.program_result);
    let after = res.resulting_accounts.iter().find(|(k, _)| *k == w.ata).unwrap().1.clone();
    assert_eq!(after.owner, TOKENKEG);
    assert_eq!(TokenAccount::unpack(&after.data).unwrap().owner, w.wallet);
}
