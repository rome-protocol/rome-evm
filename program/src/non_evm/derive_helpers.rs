// PDA derivation shortcut precompiles.
//
// Two originally-proposed selectors (see STATUS below — only
// pdas_batch_derive ended up wired here):
//   - `derive_user_ata(evm_user, mint) → bytes32`
//        Combines Rome's EXTERNAL_AUTHORITY user-PDA derivation with the
//        standard Associated Token Account derivation in one syscall.
//        Replaces 2× findPda calls (~80–90k each) with one combined
//        Rust-side computation. Saves ~80k CU per call.
//
//   - `pdas_batch_derive(seed_groups[][], program_id) → (bytes32,uint8)[]`
//        Derives N PDAs against a single program in one selector
//        dispatch. Amortizes the EVM-side dispatch overhead (~30k per
//        call) across N derivations. Saves ~50–80k per PDA when N ≥ 3.
//
// Both are read-only (CrossStateEthCall in NonEvmCall taxonomy).
//
// Both were left out of the initial shortcut set. The case for shipping them as a
// follow-up:
//
//   `derive_user_ata` saves ~80k CU per wrapper.balanceOf and per
//   wrapper.transfer (called from `UserPda.ata` inside both flows).
//   Hits the same Romeswap operations that the initial set's two primitives
//   already optimize, compounding to ~80k extra savings per balanceOf
//   read (4 of those per pair.burn = 320k extra) and ~80k per
//   wrapper.transfer (2 per pair.burn = 160k extra). The downside —
//   coupling to Rome's `EXTERNAL_AUTHORITY` seed convention + SPL ATA
//   derivation — is bounded: both are immutable parts of Rome's design
//   today and Token-2022's ATA derivation uses the same shape (different
//   token program, but same seed structure).
//
//   `pdas_batch_derive` is most valuable for cardo adapters that
//   derive 3+ PDAs per call. Drift positions, Mango groups, Meteora
//   pool tokens — each cardo adapter today does multiple findPda calls
//   sequentially, paying the EVM-side ABI roundtrip per call. Batching
//   amortizes that. Generic (works for any program, any seed shape),
//   so it doesn't introduce spec coupling.
//
// SPEC: docs/CPI_PRECOMPILE_SHORTCUTS_V2.md
// STATUS: pdas_batch_derive is wired in the CpiProgram dispatch (const
// PDAS_BATCH_DERIVE, selector 0x944336f8). derive_user_ata was never
// given a dispatch arm here — the capability it targeted now lives on
// HelperProgram as `ata(address,bytes32)` (0xfeb1c647). pdas_batch_derive
// verified on-chain against Marcus 121301 (program romedpkFK…)
// 2026-05-11 — returned PDAs match PublicKey.findProgramAddressSync
// byte-for-byte.
//
// Measured saving for derive_user_ata vs two-hop (3-sample avg on
// Marcus 121301, 2026-05-11): 281K → 129K Solana CU — ~152K saved
// per call, 54% reduction. The PR-body figure of "~80K saved" in
// CPI_PRECOMPILE_SHORTCUTS_V2.md row 5 was ~2× low — leave the
// design doc untouched as a historical record, but treat ~150K as
// the operationally correct number.

use {
    crate::{
        error::{Result, RomeProgramError::*},
        origin::Origin,
    },
    super::aux::len_ge,
    solana_program::pubkey::Pubkey,
};


// =====================================================================
// pdas_batch_derive(bytes[][] seed_groups, bytes32 program_id)
//                  → (bytes32 pda, uint8 bump)[]
// =====================================================================
//
// Calldata layout (head + dynamic):
//   word 0 (32): offset to seed_groups outer array
//   word 1 (32): program_id
// seed_groups dynamic head:
//   word 0:    N (outer length — number of PDAs to derive)
//   word 1..N: offsets to each inner seeds[][] (relative to seed_groups
//              head start)
// Each inner seeds[][]:
//   word 0:    M (inner length — number of seeds in this group)
//   word 1..M: offsets to each `bytes` seed (relative to inner head start)
// Each `bytes`:
//   word 0: length L
//   payload: L bytes, padded to 32-byte boundary
//
// Returns: ABI-encoded array of (bytes32 pda, uint8 bump) tuples,
// length N. Each tuple is 64 bytes ABI (32 for pda, 32 for bump padded).
//
// Use when caller has 2+ PDAs to derive against the same program and
// the derivations are independent. Common consumer: cardo adapters
// (Drift position + vault, Mango account + group, Meteora pool +
// LP-mint). Different programs need separate calls — keeps the
// per-call CU bounded.
//
// Bounds:
//   - Outer N ≤ 16 (caps the loop CU; chain calls if more needed)
//   - Inner M ≤ 8 (Solana's own seed-count cap)
//   - Each seed ≤ 32 bytes (Solana's max-seed-length constant)

pub fn pdas_batch_derive<T: Origin>(state: &T, abi: &[u8]) -> Result<Vec<u8>> {
    let _ = state;  // unused for pure derivation; kept for symmetry
    // Minimum: 64 bytes head (offset + program_id) + 32 bytes (length=0)
    len_ge!(abi, 96);

    // Parse program_id (head word 1).
    let program_id = Pubkey::try_from(&abi[32..64])
        .map_err(|_| InvalidNonEvmInstructionData)?;

    // The seed_groups outer array starts at the offset given in head[0].
    let seed_groups_offset = u32_from_abi_word(&abi[0..32])? as usize;
    if seed_groups_offset + 32 > abi.len() {
        return Err(InvalidNonEvmInstructionData);
    }

    let n = u32_from_abi_word(&abi[seed_groups_offset..seed_groups_offset + 32])?
        as usize;
    if n == 0 {
        // Empty result. ABI-encode as length=0 array.
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(&abi_word_uint256(0x20));
        out.extend_from_slice(&abi_word_uint256(0));
        return Ok(out);
    }

    // Bound N to keep CU bounded.
    if n > 16 {
        return Err(NonEvmCallError(
            format!("pdas_batch_derive: N={} exceeds max 16", n)
        ));
    }

    let mut results: Vec<(Pubkey, u8)> = Vec::with_capacity(n);
    let inner_offsets_base = seed_groups_offset + 32;
    for i in 0..n {
        let inner_offset_word_start = inner_offsets_base + i * 32;
        if inner_offset_word_start + 32 > abi.len() {
            return Err(InvalidNonEvmInstructionData);
        }
        let inner_offset = inner_offsets_base
            + u32_from_abi_word(
                &abi[inner_offset_word_start..inner_offset_word_start + 32],
            )? as usize;
        let seeds = decode_inner_seed_array(abi, inner_offset)?;

        let seed_refs: Vec<&[u8]> = seeds.iter().map(|v| v.as_slice()).collect();
        let (pda, bump) = state.base().pda.find_program_address(&seed_refs, &program_id);
        results.push((pda, bump));
    }

    // ABI-encode the result array of (bytes32, uint8) tuples.
    let mut out = Vec::with_capacity(64 + results.len() * 64);
    out.extend_from_slice(&abi_word_uint256(0x20));
    out.extend_from_slice(&abi_word_uint256(results.len() as u128));
    for (pda, bump) in results {
        out.extend_from_slice(pda.as_ref());
        let mut bump_word = [0u8; 32];
        bump_word[31] = bump;
        out.extend_from_slice(&bump_word);
    }
    Ok(out)
}

// =====================================================================
// helpers
// =====================================================================

fn decode_inner_seed_array(abi: &[u8], offset: usize) -> Result<Vec<Vec<u8>>> {
    if offset + 32 > abi.len() {
        return Err(InvalidNonEvmInstructionData);
    }
    let m = u32_from_abi_word(&abi[offset..offset + 32])? as usize;
    if m > 8 {
        return Err(NonEvmCallError(
            format!("pdas_batch_derive: inner seed count {} exceeds max 8", m)
        ));
    }

    let mut seeds = Vec::with_capacity(m);
    let inner_base = offset + 32;
    for j in 0..m {
        let pointer_word = inner_base + j * 32;
        if pointer_word + 32 > abi.len() {
            return Err(InvalidNonEvmInstructionData);
        }
        let bytes_at = inner_base
            + u32_from_abi_word(&abi[pointer_word..pointer_word + 32])? as usize;
        if bytes_at + 32 > abi.len() {
            return Err(InvalidNonEvmInstructionData);
        }
        let len = u32_from_abi_word(&abi[bytes_at..bytes_at + 32])? as usize;
        if len > 32 {
            return Err(NonEvmCallError(
                format!("pdas_batch_derive: seed length {} exceeds Solana's 32", len)
            ));
        }
        let payload_start = bytes_at + 32;
        if payload_start + len > abi.len() {
            return Err(InvalidNonEvmInstructionData);
        }
        seeds.push(abi[payload_start..payload_start + len].to_vec());
    }
    Ok(seeds)
}

fn u32_from_abi_word(word: &[u8]) -> Result<u32> {
    if word.len() != 32 {
        return Err(InvalidNonEvmInstructionData);
    }
    if word[..28].iter().any(|&b| b != 0) {
        return Err(NonEvmCallError(
            "uint32 ABI word has high-order non-zero bytes".to_string()
        ));
    }
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&word[28..32]);
    Ok(u32::from_be_bytes(buf))
}

fn abi_word_uint256(v: u128) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[16..32].copy_from_slice(&v.to_be_bytes());
    out
}

