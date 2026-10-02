// The one place mint bytes are interpreted.
//
// A mint's owner and TLV are the only truth about how a token operation must
// behave, so they are read here and nowhere else; consumers take the result.
//
// ARMED, not PRESENT: an extension can be present and inert. A TransferHook
// with a zero program_id makes the processor's get_program_id() return None, so
// no hook CPI fires, and a TransferFeeConfig at zero bps skips the fee branch.
// Such a mint transfers correctly on a plain transfer_checked, so every
// decision keys on the armed state — refusing on presence would refuse most
// real Token-2022 mints.
//
// Parsing uses the vendored spl-token-2022 crate, the same code the cached
// track executes, so the interpreter and the processor cannot disagree.

use {
    crate::error::{Result, RomeProgramError::InvalidSplMintAccount},
    solana_program::pubkey::Pubkey,
    spl_token_2022::{
        extension::{
            transfer_fee::TransferFeeConfig, transfer_hook::TransferHook,
            BaseStateWithExtensions, ExtensionType, StateWithExtensions,
        },
        state::{Account as SplAccount, Mint},
    },
    spl_token_2022_interface::extension::account_len,
    solana_program::program_pack::Pack,
};

/// Everything a consumer needs to know about a mint. Facts travel whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintProfile {
    /// The owning token program — the routing truth for every instruction.
    pub program: Pubkey,
    pub decimals: u8,
    /// The transfer-hook program, only when ARMED. `None` covers both "no hook
    /// extension" and "hook present with a zero program_id".
    pub hook_program: Option<Pubkey>,
    /// Transfer fee in basis points for the current epoch, only when ARMED.
    /// The greater of the two scheduled fees, so a fee arming at the epoch
    /// boundary is never under-reported.
    pub fee_bps: Option<u16>,
    /// Bit N set ⇔ `ExtensionType` discriminant N is present. Generic on
    /// purpose: a caller decides which families it refuses, and adding an
    /// extension type upstream needs no change here.
    pub extensions: u32,
}

impl MintProfile {
    /// ABI encoding for `mint_info` — a fixed 5-word tuple
    /// `(bytes32, uint8, bytes32, uint16, uint32)`, so it is built directly
    /// rather than through the `#[repr(C, packed)]`-cast idiom the wider
    /// multi-field readers use; there is no dynamic tail to splice.
    ///
    /// Unarmed reads as zero on purpose: a caller checking `hookProgram != 0`
    /// is asking the right question, and one checking presence is not.
    pub fn to_abi(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 * 5);
        let word = |v: u128| {
            let mut w = [0u8; 32];
            w[16..32].copy_from_slice(&v.to_be_bytes());
            w
        };
        out.extend_from_slice(self.program.as_ref());
        out.extend_from_slice(&word(self.decimals as u128));
        out.extend_from_slice(self.hook_program.unwrap_or_default().as_ref());
        out.extend_from_slice(&word(self.fee_bps.unwrap_or(0) as u128));
        out.extend_from_slice(&word(self.extensions as u128));
        out
    }
}

/// The mint's facts. The one place mint bytes are read, so callers that need
/// only the program and decimals project from this rather than having a second,
/// cheaper reader to keep in step (S1: facts travel whole). Probing extensions
/// on a legacy mint short-circuits on an empty TLV, so the routing-only callers
/// pay nothing for facts they ignore.
pub fn profile(owner: &Pubkey, data: &[u8]) -> Result<MintProfile> {
    let state = unpack(owner, data)?;
    let mut p = MintProfile {
        program: *owner,
        decimals: state.base.decimals,
        hook_program: None,
        fee_bps: None,
        extensions: 0,
    };

    // A legacy mint has no TLV region at all — nothing to enumerate.
    if *owner == spl_token_interface::ID {
        return Ok(p);
    }

    let Ok(types) = state.get_extension_types() else {
        return Err(InvalidSplMintAccount(*owner));
    };

    for t in types {
        let bit = t as u32;
        if bit < u32::BITS {
            p.extensions |= 1 << bit;
        }
        match t {
            ExtensionType::TransferHook => {
                if let Ok(h) = state.get_extension::<TransferHook>() {
                    p.hook_program = Option::<Pubkey>::from(h.program_id);
                }
            }
            ExtensionType::TransferFeeConfig => {
                if let Ok(cfg) = state.get_extension::<TransferFeeConfig>() {
                    let newer: u16 = cfg.newer_transfer_fee.transfer_fee_basis_points.into();
                    let older: u16 = cfg.older_transfer_fee.transfer_fee_basis_points.into();
                    let bps = newer.max(older);
                    p.fee_bps = (bps > 0).then_some(bps);
                }
            }
            _ => {}
        }
    }

    Ok(p)
}

/// The account length the Associated Token Account program will create for this
/// mint, which is what the cached overlay must reproduce (I7).
///
/// `try_calculate_account_len_from_mint_data` accumulates the mint-required
/// init-account extensions plus the additional list passed here, deduped —
/// the same composition the token program's `GetAccountDataSize` arm
/// performs. `[ImmutableOwner]` is the extra the ATA program always adds,
/// which is why a plain Token-2022 ATA is 170 bytes and not 165.
///
/// Note SPL's own `get_account_len` helper is deliberately NOT used: it CPIs
/// `GetAccountDataSize` and reads the answer with `get_return_data()`, and Rome
/// does not surface CPI return data.
///
/// Sizing is the one place **presence** is the right question rather than armed
/// state — an unarmed hook still forces `TransferHookAccount` onto every account
/// of that mint.
pub fn ata_len(owner: &Pubkey, data: &[u8]) -> Result<usize> {
    // Owner-program + mint-shape validation only; account_len below reads data directly.
    let _ = unpack(owner, data)?;

    // A legacy mint has no TLV region, and its accounts are the bare 165 bytes.
    if *owner == spl_token_interface::ID {
        return Ok(SplAccount::LEN);
    }

    account_len::try_calculate_account_len_from_mint_data(data, &[ExtensionType::ImmutableOwner])
        .map_err(|_| InvalidSplMintAccount(*owner))
}

fn unpack<'a>(owner: &Pubkey, data: &'a [u8]) -> Result<StateWithExtensions<'a, Mint>> {
    if *owner != spl_token_interface::ID && *owner != spl_token_2022_interface::ID {
        return Err(InvalidSplMintAccount(*owner));
    }
    StateWithExtensions::<Mint>::unpack(data).map_err(|_| InvalidSplMintAccount(*owner))
}
