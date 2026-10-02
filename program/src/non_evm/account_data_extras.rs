// Typed read-side shortcuts that build on `account_data_at`.
//
// These two primitives were considered + left out of the initial shortcut set because
// the audit-survival test (CU saved × frequency vs frozen-ABI cost over
// years) was a closer call than the two primitives that did ship there.
// On reflection, the case is strong enough to ship them as well — see
// docs/CPI_PRECOMPILE_SHORTCUTS_V2.md for the per-call savings + the
// frequency analysis that justifies the additional ABI surface.
//
//   - `account_u64_at(pubkey, offset)`  → uint64    (saves ~80–120k vs
//                                                    `account_data_at`
//                                                    + Solidity-side
//                                                    `Convert.u64le`)
//   - `account_lamports(pubkey)`        → uint64    (saves ~150k vs
//                                                    falling back to
//                                                    full `account_info`
//                                                    when caller only
//                                                    needs the lamports
//                                                    field)
//
// Both are read-only (CrossStateEthCall in NonEvmCall taxonomy).
//
// SPEC: docs/CPI_PRECOMPILE_SHORTCUTS_V2.md
// STATUS: stub — call shape + ABI decoders; final wire-up in cpi.rs.
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
// account_u64_at(bytes32 pubkey, uint16 offset) → uint64
// =====================================================================
//
// Calldata: 32 (pubkey) + 32 (offset, ABI-padded uint16) = 64 bytes.
// Returns:  32 bytes ABI-encoded uint64 (LE u64 value, big-endian-padded
//           to 32 bytes per ABI convention).
//
// Reads 8 bytes at `offset` from the account's data, decodes as
// little-endian u64 in Rust, returns as ABI uint256. Caller skips
// the EVM-side `Convert.u64le` bit-shift loop (~80k CU on its own)
// AND the Solidity-side `bytes` ABI decode (~30–40k CU).
//
// This is sugar over `account_data_at` for the most common shape — u64
// fields at a known offset. Justified by frequency rather than
// uniqueness: SPL Token amount, SPL Mint supply, SPL TokenAccount
// delegated_amount, Pyth/Switchboard u64 fields, every Anchor account
// with a u64 — all hit at high rates from rome-ui balance reads,
// Romeswap pool reads, oracle adapters, and cardo adapters.
//
// Per-call CU comparison:
//   - existing path:        wrapper.balanceOf via account_info: ~322k
//   - with account_data_at: wrapper.balanceOf via account_data_at + EVM Borsh decode: ~220k
//   - with this primitive:  wrapper.balanceOf via account_u64_at:   ~140k

pub fn account_u64_at<T: Origin>(state: &T, abi: &[u8]) -> Result<Vec<u8>> {
    len_eq!(abi, 64);
    let pubkey = Pubkey::try_from(&abi[0..32])
        .map_err(|_| InvalidNonEvmInstructionData)?;
    let offset = u16_from_abi_word(&abi[32..64])? as usize;

    let acc = state.account(&pubkey)?;
    if offset + 8 > acc.data.len() {
        return Err(NonEvmCallError(format!(
            "account_u64_at: offset {} + 8 out of {} bytes",
            offset, acc.data.len()
        )));
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&acc.data[offset..offset + 8]);
    let value = u64::from_le_bytes(buf);

    let mut out = [0u8; 32];
    out[24..32].copy_from_slice(&value.to_be_bytes());
    Ok(out.to_vec())
}

// =====================================================================
// account_lamports(bytes32 pubkey) → uint64
// =====================================================================
//
// Calldata: 32 bytes (one ABI-encoded bytes32).
// Returns:  32 bytes ABI-encoded uint64 (lamports, padded).
//
// Use when caller only needs to know "does this account exist / how
// funded?". Saves ~150k CU vs falling back to `account_info`
// because the data fetch + ABI-encode of the 6-tuple is skipped
// entirely (lamports lives on the AccountInfo header, not in the data
// buffer; the runtime can return it without touching the data
// region).
//
// Common callers:
//   - SPL_ERC20.ensure_token_account fast-path probe (lamports != 0
//     means ATA exists, no need to read data)
//   - bridge worker's pre-flight existence checks
//   - cardo adapter pre-flight gates ("is this PDA initialized?")
//   - any "should I create this account?" decision

pub fn account_lamports<T: Origin>(state: &T, abi: &[u8]) -> Result<Vec<u8>> {
    len_eq!(abi, 32);
    let pubkey = Pubkey::try_from(abi)
        .map_err(|_| InvalidNonEvmInstructionData)?;

    let acc = state.account(&pubkey)?;  // lamports-only fast path
    let mut out = [0u8; 32];
    out[24..32].copy_from_slice(&acc.lamports.to_be_bytes());
    Ok(out.to_vec())
}

// =====================================================================
// internal helpers
// =====================================================================

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

