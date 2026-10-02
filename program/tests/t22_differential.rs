// Differential harness: vendored token processors (what the cached overlay
// executes in-process) vs the DEPLOYED devnet ELFs (what the commit-time CPI
// executes), byte-compared on the same pre-state. Guards the vendored-crate ↔
// deployed-program seam that gates the fee tier, plus the overlay ATA-sizing
// contract (legacy byte-frozen at 165).
//
// ELF fixtures under tests/fixtures/elf/ are point-in-time devnet snapshots
// (fetched 2026-07-24): token2022.so, tokenkeg.so (p-token), ata.so. They are
// vendored so CI is deterministic; a scheduled re-fetch drift alarm is the
// companion (tracked in the token2022 plan), not live fetches here.
//
// The epoch-skew test pins the one known divergence mode: fee schedules flip
// per-epoch, so an emulation clock lagging the cluster clock stages a
// different fee than the commit charges. The emulator serves Clock from
// chain; the iterative confirm path re-derives sysvars from
// Emulation.original — that seam gets its own case in the emulator crate.

use {
    mollusk_svm::{program as mprog, Mollusk},
    solana_account::Account,
    solana_program::{
        account_info::AccountInfo, instruction::Instruction, program_option::COption,
        program_pack::Pack, pubkey::Pubkey,
    },
    spl_token_2022::{
        extension::{
            transfer_fee::{TransferFee, TransferFeeAmount, TransferFeeConfig},
            BaseStateWithExtensions, BaseStateWithExtensionsMut, ExtensionType,
            PodStateWithExtensions, StateWithExtensionsMut,
        },
        pod::PodAccount,
        state::{Account as TokenAccount, Mint},
    },
    std::collections::HashMap,
};

const T22: Pubkey = spl_token_2022_interface::ID;
const TOKENKEG: Pubkey = spl_token_interface::ID;
const ATA_PROG: Pubkey = Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

// Vendored-processor syscall stubs. Clock::get() runs inside the fee path;
// STUB_EPOCH models the emulation clock, skewable against Mollusk's cluster
// clock. Process-global: only the epoch-skew test depends on the value —
// every other case uses epoch-invariant fee schedules.
static STUB_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static STUBS_ONCE: std::sync::Once = std::sync::Once::new();

struct Stubs;
impl solana_program::program_stubs::SyscallStubs for Stubs {
    fn sol_get_clock_sysvar(&self, var_addr: *mut u8) -> u64 {
        let clock = solana_program::clock::Clock {
            epoch: STUB_EPOCH.load(std::sync::atomic::Ordering::Relaxed),
            ..Default::default()
        };
        unsafe { std::ptr::write(var_addr as *mut solana_program::clock::Clock, clock) };
        0
    }
    fn sol_get_rent_sysvar(&self, var_addr: *mut u8) -> u64 {
        unsafe {
            std::ptr::write(
                var_addr as *mut solana_program::rent::Rent,
                solana_program::rent::Rent::default(),
            )
        };
        0
    }
}

// spl-token-2022 11.0.0 pins its own `solana-sysvar 3.x` (semver-incompatible
// with our `solana-program` 4.x line), so its Clock/Rent reads resolve
// through this separate stub registry, not the one above. Same `Stubs`
// struct, second trait impl, live `STUB_EPOCH` read — no re-install needed.
impl solana_sysvar::program_stubs::SyscallStubs for Stubs {
    fn sol_get_clock_sysvar(&self, var_addr: *mut u8) -> u64 {
        let clock = solana_program::clock::Clock {
            epoch: STUB_EPOCH.load(std::sync::atomic::Ordering::Relaxed),
            ..Default::default()
        };
        unsafe { std::ptr::write(var_addr as *mut solana_program::clock::Clock, clock) };
        0
    }
    fn sol_get_rent_sysvar(&self, var_addr: *mut u8) -> u64 {
        unsafe {
            std::ptr::write(
                var_addr as *mut solana_program::rent::Rent,
                solana_program::rent::Rent::default(),
            )
        };
        0
    }
}

fn install_stubs() {
    STUBS_ONCE.call_once(|| {
        solana_program::program_stubs::set_syscall_stubs(Box::new(Stubs));
        solana_sysvar::program_stubs::set_syscall_stubs(Box::new(Stubs));
    });
    install_stub_sysvars();
}

// Install clock/rent snapshot bytes into the `solana_get_sysvar` host registry
// (the Agave 4.x `GetSysvar` path), which fails closed off-chain otherwise.
// Re-install (not once-only) so `STUB_EPOCH` mutations between the epoch-skew
// test's phases are reflected in the served clock.
fn install_stub_sysvars() {
    let clock = solana_program::clock::Clock {
        epoch: STUB_EPOCH.load(std::sync::atomic::Ordering::Relaxed),
        ..Default::default()
    };
    let clock_bytes = bincode::serde::encode_to_vec(&clock, bincode::config::legacy())
        .expect("Clock encodes");
    solana_get_sysvar::set_sysvar_bytes(solana_program::sysvar::clock::ID, clock_bytes);

    let rent_bytes =
        bincode::serde::encode_to_vec(solana_program::rent::Rent::default(), bincode::config::legacy())
            .expect("Rent encodes");
    solana_get_sysvar::set_sysvar_bytes(solana_program::sysvar::rent::ID, rent_bytes);
}

fn elf(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/elf/{name}.so",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn mollusk() -> Mollusk {
    let mut m = Mollusk::default();
    m.add_program_with_loader_and_elf(
        &T22,
        &solana_sdk_ids::bpf_loader_upgradeable::id(),
        &elf("token2022"),
    );
    m.add_program_with_loader_and_elf(
        &TOKENKEG,
        &solana_sdk_ids::bpf_loader_upgradeable::id(),
        &elf("tokenkeg"),
    );
    m.add_program_with_loader_and_elf(&ATA_PROG, &solana_sdk_ids::bpf_loader::id(), &elf("ata"));
    m
}

struct Store(HashMap<Pubkey, Account>);
impl Store {
    fn get(&self, k: &Pubkey) -> Account {
        self.0.get(k).cloned().unwrap_or_default()
    }
    fn run(&mut self, m: &Mollusk, ix: &Instruction, label: &str) {
        let accs: Vec<(Pubkey, Account)> =
            ix.accounts.iter().map(|a| (a.pubkey, self.get(&a.pubkey))).collect();
        let res = m.process_instruction(ix, &accs);
        assert!(
            !res.program_result.is_err(),
            "{label} failed under deployed ELF: {:?}",
            res.program_result
        );
        for (k, a) in res.resulting_accounts.iter() {
            self.0.insert(*k, a.clone());
        }
    }
}

// Execute the VENDORED processor in-process over copies of the pre-state —
// exactly what the cached overlay does — returning post (lamports, data).
fn run_vendored(
    prog: &Pubkey,
    ix: &Instruction,
    pre: &HashMap<Pubkey, Account>,
) -> HashMap<Pubkey, (u64, Vec<u8>)> {
    // Re-install from the current STUB_EPOCH on every call so a caller that
    // mutated it between phases (the epoch-skew test) sees it reflected.
    install_stub_sysvars();

    struct Slot {
        key: Pubkey,
        lamports: u64,
        data: Vec<u8>,
        owner: Pubkey,
        signer: bool,
        writable: bool,
    }
    let mut slots: Vec<Slot> = ix
        .accounts
        .iter()
        .map(|a| {
            let acc = pre.get(&a.pubkey).cloned().unwrap_or_default();
            Slot {
                key: a.pubkey,
                lamports: acc.lamports,
                data: acc.data,
                owner: acc.owner,
                signer: a.is_signer,
                writable: a.is_writable,
            }
        })
        .collect();
    let infos: Vec<AccountInfo> = slots
        .iter_mut()
        .map(|s| {
            AccountInfo::new(&s.key, s.signer, s.writable, &mut s.lamports, &mut s.data, &s.owner, false)
        })
        .collect();
    let res = if *prog == T22 {
        spl_token_2022::processor::Processor::process(prog, &infos, &ix.data)
    } else {
        spl_token::processor::Processor::process(prog, &infos, &ix.data)
    };
    assert!(res.is_ok(), "vendored processor failed: {res:?}");
    drop(infos);
    slots.into_iter().map(|s| (s.key, (s.lamports, s.data))).collect()
}

fn rent_for(m: &Mollusk, len: usize) -> u64 {
    m.sysvars.rent.minimum_balance(len)
}

fn make_plain_mint(m: &Mollusk, authority: &Pubkey, owner_prog: Pubkey) -> Account {
    let mut data = vec![0u8; Mint::LEN];
    Mint {
        mint_authority: COption::Some(*authority),
        supply: 0,
        decimals: 6,
        is_initialized: true,
        freeze_authority: COption::None,
    }
    .pack_into_slice(&mut data);
    Account { lamports: rent_for(m, data.len()), data, owner: owner_prog, executable: false, rent_epoch: 0 }
}

// Fee-bearing mint with schedule (older_epoch, older_bps) → (newer_epoch, newer_bps).
fn make_fee_mint(
    m: &Mollusk,
    authority: &Pubkey,
    older: (u64, u16),
    newer: (u64, u16),
) -> Account {
    let len =
        ExtensionType::try_calculate_account_len::<Mint>(&[ExtensionType::TransferFeeConfig])
            .unwrap();
    let mut data = vec![0u8; len];
    let mut st = StateWithExtensionsMut::<Mint>::unpack_uninitialized(&mut data).unwrap();
    let ext = st.init_extension::<TransferFeeConfig>(true).unwrap();
    ext.older_transfer_fee = TransferFee {
        epoch: older.0.into(),
        maximum_fee: u64::MAX.into(),
        transfer_fee_basis_points: older.1.into(),
    };
    ext.newer_transfer_fee = TransferFee {
        epoch: newer.0.into(),
        maximum_fee: u64::MAX.into(),
        transfer_fee_basis_points: newer.1.into(),
    };
    st.base = Mint {
        mint_authority: COption::Some(*authority),
        supply: 0,
        decimals: 6,
        is_initialized: true,
        freeze_authority: COption::None,
    };
    st.pack_base();
    st.init_account_type().unwrap();
    Account { lamports: rent_for(m, len), data, owner: T22, executable: false, rent_epoch: 0 }
}

struct Scenario {
    ata_len: usize,
    identical: bool,
    dst_amount: u64,
    dst_withheld: u64,
}

fn withheld_of(d: &[u8]) -> u64 {
    PodStateWithExtensions::<PodAccount>::unpack(d)
        .ok()
        .and_then(|ps| ps.get_extension::<TransferFeeAmount>().ok().map(|e| u64::from(e.withheld_amount)))
        .unwrap_or(0)
}

fn amount_of(d: &[u8]) -> u64 {
    PodStateWithExtensions::<PodAccount>::unpack(d).map(|ps| ps.base.amount.into()).unwrap_or(0)
}

// Create 2 ATAs via the deployed ATA program, fund src via the deployed token
// program, transfer_checked 100_000 twice — deployed ELF and vendored
// processor — from the same pre-state, and byte-compare the results.
fn run_transfer_scenario(m: &Mollusk, token_prog: Pubkey, mint_acc: Account) -> Scenario {
    install_stubs();
    let payer = Pubkey::new_unique();
    let owner1 = Pubkey::new_unique();
    let owner2 = Pubkey::new_unique();
    let mint_auth = mint_acc_authority(&mint_acc);
    let mint = Pubkey::new_unique();

    let mut st = Store(HashMap::new());
    st.0.insert(payer, Account { lamports: 10_000_000_000, ..Account::default() });
    let (sk, sa) = mprog::keyed_account_for_system_program();
    st.0.insert(sk, sa);
    st.0.insert(T22, mprog::create_program_account_loader_v3(&T22));
    st.0.insert(TOKENKEG, mprog::create_program_account_loader_v3(&TOKENKEG));
    st.0.insert(ATA_PROG, mprog::create_program_account_loader_v3(&ATA_PROG));
    st.0.insert(mint, mint_acc);

    let ata_of = |owner: &Pubkey| {
        spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(
            owner, &mint, &token_prog,
        )
    };
    let a1 = ata_of(&owner1);
    let a2 = ata_of(&owner2);
    for owner in [&owner1, &owner2] {
        let ix = spl_associated_token_account_interface::instruction::create_associated_token_account(
            &payer, owner, &mint, &token_prog,
        );
        st.run(m, &ix, "create_ata");
    }
    let ata_len = st.get(&a2).data.len();

    let fund = spl_token_2022_interface::instruction::mint_to_checked(
        &token_prog, &mint, &a1, &mint_auth, &[], 1_000_000, 6,
    )
    .unwrap();
    st.run(m, &fund, "mint_to_checked");

    let xfer = spl_token_2022_interface::instruction::transfer_checked(
        &token_prog, &a1, &mint, &a2, &owner1, &[], 100_000, 6,
    )
    .unwrap();
    let pre: HashMap<Pubkey, Account> =
        xfer.accounts.iter().map(|am| (am.pubkey, st.get(&am.pubkey))).collect();

    st.run(m, &xfer, "transfer_checked");
    let vend = run_vendored(&token_prog, &xfer, &pre);

    let identical = [a1, mint, a2].iter().all(|k| {
        let mol = st.get(k);
        let (vl, vd) = vend.get(k).cloned().unwrap_or((0, vec![]));
        mol.data == vd && mol.lamports == vl
    });

    let dst = st.get(&a2);
    Scenario { ata_len, identical, dst_amount: amount_of(&dst.data), dst_withheld: withheld_of(&dst.data) }
}

fn mint_acc_authority(acc: &Account) -> Pubkey {
    // Both layouts share the base Mint prefix; read the COption<Pubkey>.
    Mint::unpack_from_slice(&acc.data[..Mint::LEN]).unwrap().mint_authority.unwrap()
}

#[test]
fn legacy_transfer_vendored_matches_deployed_and_ata_is_165() {
    let m = mollusk();
    let auth = Pubkey::new_unique();
    let s = run_transfer_scenario(&m, TOKENKEG, make_plain_mint(&m, &auth, TOKENKEG));
    // The overlay stages legacy ATA creates at exactly 165 bytes — byte-frozen
    // (the Δ2 owner-branch contract): the deployed ATA program agrees.
    assert_eq!(s.ata_len, TokenAccount::LEN, "legacy ATA must stay byte-frozen at 165");
    assert!(s.identical, "vendored spl-token must byte-match deployed (p-token) post-state");
    assert_eq!((s.dst_amount, s.dst_withheld), (100_000, 0));
}

#[test]
fn plain_t22_transfer_vendored_matches_deployed_and_ata_is_170() {
    let m = mollusk();
    let auth = Pubkey::new_unique();
    let expect_len =
        ExtensionType::try_calculate_account_len::<TokenAccount>(&[ExtensionType::ImmutableOwner])
            .unwrap();
    let s = run_transfer_scenario(&m, T22, make_plain_mint(&m, &auth, T22));
    assert_eq!(s.ata_len, expect_len, "plain-2022 ATA = base + ImmutableOwner");
    assert!(s.identical, "vendored spl-token-2022 must byte-match deployed post-state");
    assert_eq!((s.dst_amount, s.dst_withheld), (100_000, 0));
}

#[test]
fn fee_t22_transfer_vendored_matches_deployed_with_fee_semantics() {
    let m = mollusk();
    let auth = Pubkey::new_unique();
    // Epoch-invariant schedule (both slots 100 bps) so this case holds under
    // any clock; the skew case below isolates clock sensitivity.
    let mint = make_fee_mint(&m, &auth, (0, 100), (0, 100));
    let expect_len = ExtensionType::try_calculate_account_len::<TokenAccount>(&[
        ExtensionType::ImmutableOwner,
        ExtensionType::TransferFeeAmount,
    ])
    .unwrap();
    let s = run_transfer_scenario(&m, T22, mint);
    assert_eq!(s.ata_len, expect_len, "fee-2022 ATA = base + ImmutableOwner + TransferFeeAmount");
    assert!(s.identical, "vendored fee math must byte-match deployed post-state");
    // 100 bps on 100_000: recipient nets 99_000, 1_000 withheld in the dst ext.
    assert_eq!((s.dst_amount, s.dst_withheld), (99_000, 1_000));
}

#[test]
fn epoch_skew_is_the_divergence_mode_and_sync_restores_identity() {
    install_stubs();
    let mut m = mollusk();
    let auth = Pubkey::new_unique();
    // Fee flips 0bps → 100bps at epoch 1: cluster ahead of a lagging
    // emulation clock stages fee 0 while commit charges 1_000.
    let mint_acc = make_fee_mint(&m, &auth, (0, 0), (1, 100));
    let payer = Pubkey::new_unique();
    let owner1 = Pubkey::new_unique();
    let owner2 = Pubkey::new_unique();
    let mint = Pubkey::new_unique();

    let mut st = Store(HashMap::new());
    st.0.insert(payer, Account { lamports: 10_000_000_000, ..Account::default() });
    let (sk, sa) = mprog::keyed_account_for_system_program();
    st.0.insert(sk, sa);
    st.0.insert(T22, mprog::create_program_account_loader_v3(&T22));
    st.0.insert(ATA_PROG, mprog::create_program_account_loader_v3(&ATA_PROG));
    st.0.insert(mint, mint_acc);

    let a1 = spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(&owner1, &mint, &T22);
    let a2 = spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(&owner2, &mint, &T22);
    for owner in [&owner1, &owner2] {
        let ix = spl_associated_token_account_interface::instruction::create_associated_token_account(&payer, owner, &mint, &T22);
        st.run(&m, &ix, "create_ata");
    }
    st.run(
        &m,
        &spl_token_2022_interface::instruction::mint_to_checked(&T22, &mint, &a1, &auth, &[], 1_000_000, 6).unwrap(),
        "fund",
    );

    // Cluster at epoch 1: the 100 bps schedule is live for the deployed leg.
    m.sysvars.clock.epoch = 1;
    let xfer = spl_token_2022_interface::instruction::transfer_checked(&T22, &a1, &mint, &a2, &owner1, &[], 100_000, 6).unwrap();
    let pre: HashMap<Pubkey, Account> =
        xfer.accounts.iter().map(|am| (am.pubkey, st.get(&am.pubkey))).collect();
    st.run(&m, &xfer, "transfer_checked @cluster-epoch=1");
    let deployed_withheld = withheld_of(&st.get(&a2).data);
    assert_eq!(deployed_withheld, 1_000, "cluster at epoch 1 charges the new fee");

    // Lagging emulation clock (epoch 0) → stages fee 0 → DIVERGENT.
    STUB_EPOCH.store(0, std::sync::atomic::Ordering::Relaxed);
    let vend = run_vendored(&T22, &xfer, &pre);
    let lagging = withheld_of(&vend.get(&a2).map(|(_, d)| d.clone()).unwrap_or_default());
    assert_ne!(lagging, deployed_withheld, "epoch skew must surface as divergence");
    assert_eq!(lagging, 0);

    // Synced clock → byte-identical again.
    STUB_EPOCH.store(1, std::sync::atomic::Ordering::Relaxed);
    let vend = run_vendored(&T22, &xfer, &pre);
    let synced = withheld_of(&vend.get(&a2).map(|(_, d)| d.clone()).unwrap_or_default());
    assert_eq!(synced, deployed_withheld, "synced clocks must restore identity");
    STUB_EPOCH.store(0, std::sync::atomic::Ordering::Relaxed);
}
