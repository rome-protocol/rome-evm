// Slim account-state read primitive — strict subset of `account_info`.
//
// Why: `account_info(bytes32) → (uint64, bytes32, bool, bool, bool, bytes)`
// returns the full 6-tuple regardless of what the caller actually wants.
// Today every in-tree caller (`SplTokenLib::load_mint`,
// `SplTokenLib::load_token_amount`, `SplTokenLib::load_token_account_delegate`)
// destructures via `(,,,,, bytes memory data) = account_info(...)`, throwing
// away 5 of 6 fields. The unused fields cost ~50–100k CU per call in
// ABI-encode + Solidity-side decode overhead.
//
// `account_data_at(pubkey, offset, length) → bytes` is the **single**
// primitive added: it is the generic read-side abstraction over Solana
// account data. Callers ask for the slice they need; the precompile
// returns it. No specialized variants, no typed sugar, no junk-drawer.
//
// Specialized variants (`account_u64_at`, `account_lamports`,
// `account_owner`, `account_amount`) were considered and dropped:
//   - typed sugar can live in Solidity helpers (a Convert.u64le call
//     downstream of `account_data_at(ata, 64, 8)` costs ~50k EVM CU,
//     similar to a hand-coded precompile); it does not justify a frozen
//     ABI commitment per type.
//   - lamports / owner reads are infrequent enough that callers can
//     fall back to the existing full `account_info` for cold paths
//     without measurable impact.
//   - keeping the precompile generic means it survives SPL spec updates
//     (Token-2022 layout extensions, future Anchor IDL changes) without
//     needing parallel siblings.
//
// SPEC: see docs/CPI_PRECOMPILE_SHORTCUTS.md for measured CU baselines
// and per-product impact analysis.
//
// STATUS: stub — implements the call shape; final wire-up in cpi.rs.
//         Mollusk emulator mirror in emulator/src/... must match.

use {
    crate::{
        error::{Result, RomeProgramError::*},
        origin::Origin,
    },
    super::aux::len_eq,
    solana_program::pubkey::Pubkey,
};

// =====================================================================
// account_data_at(bytes32 pubkey, uint16 offset, uint16 length) → bytes
// =====================================================================
//
// Calldata layout: 32 (pubkey) + 32 (offset, ABI-padded uint16) + 32
//                  (length, ABI-padded uint16) = 96 bytes
// Returns: ABI-encoded `bytes` of length `length`.
//
// Errors:
//   - `InvalidNonEvmInstructionData` if calldata != 96 bytes
//   - `OutOfRange` if (offset + length) > account.data.len()
//
// Bounds: `length` is u16-capped at 65535 — sufficient for any SPL/Anchor
// state we read in practice (Token = 165 / 72-actual, Mint = 82, large
// Anchor accounts ~1024). Larger reads should still go through full
// `account_info`.
//
// Generic to any Solana account. Use for:
//   - SPL TokenAccount.amount    (offset 64,  length 8)
//   - SPL TokenAccount.owner     (offset 32,  length 32)
//   - SPL TokenAccount.delegate  (offset 76,  length 32 — within COption)
//   - SPL TokenAccount.delegated_amount (offset 121, length 8)
//   - SPL Mint.supply            (offset 36,  length 8)
//   - SPL Mint.decimals          (offset 44,  length 1)
//   - Pyth/Switchboard u64/i64 fields (varies by feed)
//   - Custom Solana program state at known offsets

pub fn account_data_at<T: Origin>(state: &T, abi: &[u8]) -> Result<Vec<u8>> {
    len_eq!(abi, 96);
    let pubkey = Pubkey::try_from(&abi[0..32])
        .map_err(|_| InvalidNonEvmInstructionData)?;
    let offset = u16_from_abi_word(&abi[32..64])? as usize;
    let length = u16_from_abi_word(&abi[64..96])? as usize;

    let acc = state.account(&pubkey)?;
    let end = offset
        .checked_add(length)
        .ok_or(InvalidNonEvmInstructionData)?;
    if end > acc.data.len() {
        return Err(NonEvmCallError(format!(
            "account_data_at: range {}..{} out of {} bytes",
            offset, end, acc.data.len()
        )));
    }

    // ABI-encode `bytes` return value:
    //   word 0: offset = 0x20
    //   word 1: length
    //   word 2..: data, padded to 32-byte boundary
    let pad = (32 - (length % 32)) % 32;
    let mut out = Vec::with_capacity(64 + length + pad);
    out.extend_from_slice(&abi_word_uint256(0x20));
    out.extend_from_slice(&abi_word_uint256(length as u128));
    out.extend_from_slice(&acc.data[offset..end]);
    out.extend(std::iter::repeat(0u8).take(pad));
    Ok(out)
}

// =====================================================================
// internal helpers
// =====================================================================

/// Extract a uint16 from an ABI-encoded 32-byte word. ABI pads uint16 in
/// the high-order bytes (big-endian); we read the last 2 bytes and assert
/// the rest is zero (catches accidental wider values).
fn u16_from_abi_word(word: &[u8]) -> Result<u16> {
    if word.len() != 32 {
        return Err(InvalidNonEvmInstructionData);
    }
    if word[..30].iter().any(|&b| b != 0) {
        return Err(NonEvmCallError(
            "uint16 ABI word has high-order non-zero bytes".to_string()
        ));
    }
    Ok(u16::from_be_bytes([word[30], word[31]]))
}

/// Encode a uint256 (taking u128 input — sufficient for length/offset).
fn abi_word_uint256(v: u128) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[16..32].copy_from_slice(&v.to_be_bytes());
    out
}

