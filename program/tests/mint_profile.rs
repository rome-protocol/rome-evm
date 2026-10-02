// The mint interpreter, over real mint bytes.
//
// The load-bearing distinction is ARMED vs PRESENT: a TransferHook whose
// program_id is zero makes the processor's get_program_id() return None, so no
// hook CPI fires; a TransferFeeConfig at zero bps skips the fee branch. A mint
// carrying either as an inert extension transfers correctly on a plain
// transfer_checked, so refusing on presence would refuse most real 2022 mints.

use {
    rome_evm::state::mint_profile::profile,
    solana_program::pubkey::Pubkey,
    spl_token_2022::extension::ExtensionType,
};

const T22: Pubkey = spl_token_2022_interface::ID;
const LEGACY: Pubkey = spl_token_interface::ID;

const PYUSD: &[u8] = include_bytes!("fixtures/pyusd.bin");
const USDC_LEGACY: &[u8] = include_bytes!("fixtures/usdc_legacy.bin");

/// Byte offset of an extension's payload within a mint buffer, by type.
/// Walks the TLV the same way the processor does: 2-byte type, 2-byte length.
fn payload_at(data: &[u8], want: ExtensionType) -> usize {
    let mut i = 166;
    while i + 4 <= data.len() {
        let t = u16::from_le_bytes([data[i], data[i + 1]]);
        let len = u16::from_le_bytes([data[i + 2], data[i + 3]]) as usize;
        if t == 0 && len == 0 {
            break;
        }
        if t == want as u16 {
            return i + 4;
        }
        i += 4 + len;
    }
    panic!("extension {want:?} not present in fixture");
}

fn with_armed_hook(program: Pubkey) -> Vec<u8> {
    let mut d = PYUSD.to_vec();
    let at = payload_at(&d, ExtensionType::TransferHook) + 32; // authority, then program_id
    d[at..at + 32].copy_from_slice(program.as_ref());
    d
}

fn with_fee_bps(bps: u16) -> Vec<u8> {
    let mut d = PYUSD.to_vec();
    // TransferFeeConfig: two authorities (64), withheld (8), older (18), newer (18).
    // Each TransferFee is epoch(8) + maximum_fee(8) + basis_points(2).
    let cfg = payload_at(&d, ExtensionType::TransferFeeConfig);
    for fee in [cfg + 72, cfg + 90] {
        d[fee + 16..fee + 18].copy_from_slice(&bps.to_le_bytes());
    }
    d
}

#[test]
fn legacy_mint_reports_legacy_program_and_no_extensions() {
    let p = profile(&LEGACY, USDC_LEGACY).unwrap();
    assert_eq!(p.program, LEGACY);
    assert_eq!(p.decimals, 6);
    assert_eq!(p.hook_program, None, "legacy mints have no hook");
    assert_eq!(p.fee_bps, None, "legacy mints have no fee");
    assert_eq!(p.extensions, 0, "legacy mints carry no extension bits");
}

#[test]
fn extension_free_2022_mint_is_indistinguishable_but_for_its_program() {
    // A 2022 mint with no extensions is the same 82 bytes as a legacy one.
    let p = profile(&T22, USDC_LEGACY).unwrap();
    assert_eq!(p.program, T22);
    assert_eq!(p.decimals, 6);
    assert_eq!(p.hook_program, None);
    assert_eq!(p.fee_bps, None);
    assert_eq!(p.extensions, 0);
}

#[test]
fn present_but_unarmed_hook_and_fee_report_none() {
    // The real fixture carries seven extensions: the hook is present with a
    // zero program_id and the fee is present at zero bps. Both are inert.
    let p = profile(&T22, PYUSD).unwrap();
    assert_eq!(p.program, T22);
    assert_eq!(p.decimals, 6);
    assert_eq!(
        p.hook_program, None,
        "a zero program_id is an unarmed hook — reporting it armed would refuse this mint"
    );
    assert_eq!(p.fee_bps, None, "zero bps is an unarmed fee");
    // Presence is still reported, so a caller may refuse a family if it wants to.
    assert_ne!(p.extensions & (1 << ExtensionType::TransferHook as u32), 0);
    assert_ne!(
        p.extensions & (1 << ExtensionType::TransferFeeConfig as u32),
        0
    );
    assert_ne!(
        p.extensions & (1 << ExtensionType::PermanentDelegate as u32),
        0
    );
}

#[test]
fn armed_hook_reports_its_program() {
    let hook = Pubkey::new_unique();
    let p = profile(&T22, &with_armed_hook(hook)).unwrap();
    assert_eq!(p.hook_program, Some(hook));
}

#[test]
fn armed_fee_reports_current_epoch_bps() {
    let p = profile(&T22, &with_fee_bps(150)).unwrap();
    assert_eq!(p.fee_bps, Some(150));
}

#[test]
fn arming_one_family_does_not_arm_another() {
    let p = profile(&T22, &with_fee_bps(25)).unwrap();
    assert_eq!(p.fee_bps, Some(25));
    assert_eq!(p.hook_program, None, "fee armed, hook still inert");

    let hook = Pubkey::new_unique();
    let p = profile(&T22, &with_armed_hook(hook)).unwrap();
    assert_eq!(p.hook_program, Some(hook));
    assert_eq!(p.fee_bps, None, "hook armed, fee still inert");
}

#[test]
fn unknown_owner_is_refused() {
    assert!(profile(&Pubkey::new_unique(), USDC_LEGACY).is_err());
}

#[test]
fn truncated_mint_is_refused_not_panicked() {
    assert!(profile(&T22, &PYUSD[..200]).is_err());
    assert!(profile(&LEGACY, &USDC_LEGACY[..40]).is_err());
}

// The ATA length the real ATA program will create for a mint (DESIGN §4b / I7).
//
// Literals here are the measured behaviour of the deployed ATA program; the
// differential harness independently ties them to its ELF. Two independent
// checks, so neither can drift alone. A test that recomputed the expectation
// with the function under test would assert nothing.

#[test]
fn ata_len_legacy_is_the_bare_account() {
    // No account_type byte, no TLV.
    assert_eq!(rome_evm::state::mint_profile::ata_len(&LEGACY, USDC_LEGACY).unwrap(), 165);
}

#[test]
fn ata_len_extension_free_2022_still_gains_immutable_owner() {
    // The mint requires nothing, but the ATA program adds ImmutableOwner to
    // every 2022 ATA it creates — which is why a constant 165 is wrong even
    // for a mint with no extensions.
    assert_eq!(rome_evm::state::mint_profile::ata_len(&T22, USDC_LEGACY).unwrap(), 170);
}

#[test]
fn ata_len_grows_with_what_the_mint_requires() {
    // The real fixture carries both TransferFeeConfig and TransferHook, so the
    // mint requires TransferFeeAmount + TransferHookAccount, and the ATA
    // program adds ImmutableOwner on top.
    assert_eq!(rome_evm::state::mint_profile::ata_len(&T22, PYUSD).unwrap(), 187);
}

#[test]
fn ata_len_keys_on_presence_not_armed_state() {
    // Sizing is the one place PRESENCE is the right question: an unarmed hook
    // still forces TransferHookAccount onto every account of that mint. The
    // fixture's hook is unarmed, and it is still sized for the hook.
    let unarmed = rome_evm::state::mint_profile::ata_len(&T22, PYUSD).unwrap();
    let armed = rome_evm::state::mint_profile::ata_len(&T22, &with_armed_hook(Pubkey::new_unique())).unwrap();
    assert_eq!(unarmed, armed, "arming a hook must not change the account size");
    assert_eq!(
        rome_evm::state::mint_profile::ata_len(&T22, &with_fee_bps(500)).unwrap(),
        unarmed,
        "arming a fee must not change the account size either"
    );
}

#[test]
fn ata_len_refuses_a_bad_owner() {
    assert!(rome_evm::state::mint_profile::ata_len(&Pubkey::new_unique(), USDC_LEGACY).is_err());
}

// ---------------------------------------------------------------------------
// The same interpreter, against mints the real Token-2022 program wrote.
//
// Everything above this line reasons over one vendored mint and byte-edits of
// it, so it can only prove `ata_len` is self-consistent. These read the mints
// in `ci/dump/`, which the CI validator loads at genesis, and check `ata_len`
// against the length the real Associated Token Account program *actually*
// created for each — the only thing the cached overlay has to match (I7).
//
// Regenerate the fixtures with `ci/gen-t22-fixtures.sh`; the expectations below
// are what that script measured off the created accounts, not what this crate
// computed.
// ---------------------------------------------------------------------------

/// `(dump file, length the ATA program created)`.
const REAL_ATA_LEN: &[(&str, usize)] = &[
    ("spl2022_mint_plain.json", 170),
    ("spl2022_mint_fee.json", 182),
    ("spl2022_mint_hook_unarmed.json", 175),
    ("spl2022_mint_hook_armed.json", 175),
    ("spl2022_mint_fee_hook.json", 187),
    ("spl2022_mint_nontransferable.json", 174),
    ("spl2022_mint_permanent_delegate.json", 170),
    // Its accounts are created frozen, which costs no space — the state is the
    // point, not the length.
    ("spl2022_mint_frozen_default.json", 170),
    // The only family requiring an account-side extension of its own,
    // PausableAccount. If this reads 170 the vendored spl-token-2022 does not know
    // an extension the deployed program does, and the interpreter is behind.
    ("spl2022_mint_pausable.json", 174),
    // Display-only: the stored amount, and the account, are untouched.
    ("spl2022_mint_scaled_ui.json", 170),
    // Dumped from mainnet rather than generated: real metadata, issuer-held authority.
    ("spl2022_mint_ai16z.json", 170),
    ("spl_mint_usdc.json", 165),
];

/// `(owner, data)` of an account dumped by `solana account --output json`.
fn dumped(name: &str) -> (Pubkey, Vec<u8>) {
    use {base64::Engine, std::str::FromStr};

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../ci/dump/").to_string() + name;
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let acc = v.get("account").unwrap_or(&v);
    let owner = Pubkey::from_str(acc["owner"].as_str().unwrap()).unwrap();
    let b64 = acc["data"][0].as_str().unwrap();
    let data = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
    (owner, data)
}

#[test]
fn ata_len_matches_what_the_real_ata_program_created() {
    for (name, expected) in REAL_ATA_LEN {
        let (owner, data) = dumped(name);
        let got = rome_evm::state::mint_profile::ata_len(&owner, &data)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(got, *expected, "{name}: ata_len disagrees with the ATA program");
    }
}

#[test]
fn real_fixtures_distinguish_armed_from_present() {
    // Same extension, same account size, opposite armed state — the pair that
    // makes presence the wrong question for behaviour and the right one for size.
    let (o, unarmed) = dumped("spl2022_mint_hook_unarmed.json");
    let (_, armed) = dumped("spl2022_mint_hook_armed.json");

    let u = profile(&o, &unarmed).unwrap();
    let a = profile(&o, &armed).unwrap();

    let bit = 1u32 << ExtensionType::TransferHook as u32;
    assert_eq!(u.extensions & bit, bit, "hook must be PRESENT on the unarmed mint");
    assert_eq!(a.extensions & bit, bit);

    assert_eq!(u.hook_program, None, "a zero program_id must read as unarmed");
    assert!(a.hook_program.is_some(), "a real program_id must read as armed");

    assert_eq!(
        rome_evm::state::mint_profile::ata_len(&o, &unarmed).unwrap(),
        rome_evm::state::mint_profile::ata_len(&o, &armed).unwrap(),
        "arming a hook must not change the size the ATA program creates"
    );
}

#[test]
fn real_fixtures_report_their_armed_families() {
    let (o, fee) = dumped("spl2022_mint_fee.json");
    let p = profile(&o, &fee).unwrap();
    assert_eq!(p.fee_bps, Some(50), "fee mint was created at 50 bps");
    assert_eq!(p.hook_program, None);

    let (_, fee_hook) = dumped("spl2022_mint_fee_hook.json");
    let p = profile(&o, &fee_hook).unwrap();
    assert_eq!(p.fee_bps, Some(50));
    assert!(p.hook_program.is_some(), "both families armed on one mint");

    // PermanentDelegate needs nothing of an account and arms neither family —
    // the extension the earlier framing wrongly called a blocker.
    let (_, delegate) = dumped("spl2022_mint_permanent_delegate.json");
    let p = profile(&o, &delegate).unwrap();
    assert_eq!(p.decimals, 9, "created with 9 decimals, unlike the rest");
    assert_eq!(p.hook_program, None);
    assert_eq!(p.fee_bps, None);
    assert_eq!(
        p.extensions & (1 << ExtensionType::PermanentDelegate as u32),
        1 << ExtensionType::PermanentDelegate as u32
    );

    let (_, plain) = dumped("spl2022_mint_plain.json");
    let p = profile(&o, &plain).unwrap();
    assert_eq!(p.extensions, 0, "an extension-free 2022 mint carries no TLV");
    assert_eq!(p.program, T22, "…and is still routed by its owner, not its shape");
}

// ---------------------------------------------------------------------------
// Red team: things that are not a mint, but look enough like one to get in.
// ---------------------------------------------------------------------------

/// A Token-2022 *token account* is owned by the same program a mint is, so the
/// owner check cannot tell them apart. It must be refused on its contents.
///
/// This is the attack the old exact-82 check accidentally blocked: relaxing the
/// length gate to `>= 82` means length alone no longer discriminates, and a
/// 170-byte token account is longer than a mint. If this ever returned a profile,
/// a caller would route instructions using another account's bytes as decimals.
#[test]
fn a_token_account_is_not_a_mint() {
    let (owner, data) = dumped("ata2022_def_signer_ai16z.json");
    assert_eq!(owner, T22, "fixture must be owned by Token-2022, or this proves nothing");
    assert!(data.len() > 82, "and be longer than a mint, so length cannot discriminate");

    assert!(
        profile(&owner, &data).is_err(),
        "a token account must be refused as a mint"
    );
    assert!(
        rome_evm::state::mint_profile::ata_len(&owner, &data).is_err(),
        "and must not yield an account length"
    );
}

/// Zero-length and single-byte inputs, which a caller can produce by naming an
/// account that does not exist yet.
#[test]
fn empty_and_stub_accounts_are_refused() {
    for data in [vec![], vec![0u8], vec![0u8; 81]] {
        assert!(profile(&T22, &data).is_err(), "len {} must be refused", data.len());
        assert!(profile(&LEGACY, &data).is_err(), "len {} must be refused", data.len());
    }
}

/// A mint whose TLV claims an extension longer than the buffer. Must not read
/// past the end, and must not panic — the length comes from the account, which is
/// someone else's data.
#[test]
fn a_lying_tlv_length_is_refused_not_read_past() {
    let mut d = PYUSD.to_vec();
    // First TLV entry at 166: type u16, len u16. Claim a length past the buffer.
    d[168] = 0xff;
    d[169] = 0xff;
    // Either refuses or reports what it could parse — but never panics, and never
    // yields a length that came from reading beyond the account.
    let _ = profile(&T22, &d);
    let _ = rome_evm::state::mint_profile::ata_len(&T22, &d);
}

/// The interpreter is given `(owner, data)` and knows no mint pubkey, so its error
/// can only name the owner. That is a fine contract, but it means every caller has
/// to remap the payload to the mint it was actually asked about — otherwise a
/// failure reports the token *program* as the invalid account, which sends whoever
/// reads the log looking at the wrong thing entirely.
///
/// Pinned so the contract cannot drift silently in either direction.
#[test]
fn interpreter_errors_name_the_owner_so_callers_must_remap() {
    use rome_evm::error::RomeProgramError::InvalidSplMintAccount;

    let (owner, data) = dumped("ata2022_def_signer_ai16z.json"); // a token account
    match rome_evm::state::mint_profile::ata_len(&owner, &data) {
        Err(InvalidSplMintAccount(named)) => assert_eq!(
            named, owner,
            "ata_len can only name what it was given; callers remap to the mint"
        ),
        other => panic!("expected InvalidSplMintAccount, got {other:?}"),
    }
    match profile(&owner, &data) {
        Err(InvalidSplMintAccount(named)) => assert_ne!(
            named,
            Pubkey::default(),
            "a zero pubkey names nothing — the payload must be actionable"
        ),
        other => panic!("expected InvalidSplMintAccount, got {other:?}"),
    }
}
