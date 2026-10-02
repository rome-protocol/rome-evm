use {
    macros::elf, solana_program::account_info::MAX_PERMITTED_DATA_INCREASE,
};

pub const ACCOUNT_SEED: &[u8] = b"ACCOUN_SEED";
pub const EVENT_LOG: &[u8] = b"EVENT_LOG";
pub const EXIT_REASON: &[u8] = b"EXIT_REASON";
pub const REVERT_PANIC: &[u8] = &[0x4e, 0x48, 0x7b, 0x71];
pub const REVERT_ERROR: &[u8] = &[0x08, 0xc3, 0x79, 0xa0]; // Signature for "Error(string)"
// A session that keeps iterating holds its locked accounts for as long as it
// runs, so its lifetime is bounded: past this many iterations the session is
// released and marked completed instead of continuing. The client's own leg
// ceiling is 128 (rome-sdk iterative::MAX_LEGS); the margin covers the
// non-execute steps and re-serialize retries.
pub const MAX_SESSION_ITERATIONS: u64 = 160;
pub const LOCK_DURATION: i64 = 3; // each iteration blocks accounts for this number of seconds
pub const LOCK_DURATION_ALT: i64 = 4; // lock duration in case ALT usage
pub const TX_HOLDER_SEED: &[u8] = b"TX_HOLDER_SEED";
pub const STATE_HOLDER_SEED: &[u8] = b"STATE_HOLDER_SEED";
pub const NUMBER_OPCODES_PER_TX: u64 = 500;
pub const NUMBER_OPCODES_TO_ESTIMATE_TX_FLOW: u64 = 500;
pub const MAX_OPCODES_PER_EMULATION: u64 = 10_000_000;

// Precompile work-pricing constants, in the same unit as
// interpreter opcode steps (`Vm::native_work`, priced at `trap_call` dispatch,
// bounded together with `steps_executed` against MAX_OPCODES_PER_EMULATION;
// see program/src/vm/vm.rs and precompile::step_price).
//
// MEASURED 2026-09-05 on a testnet deployment of this program, via on-chain
// `computeUnitsConsumed` read back from a Solana RPC node.
// Every figure is a delta between two probe sizes so the transaction
// envelope cancels; n=3 per point, spread <1% on the large bodies.
//   price = ceil(body_CU / CU_per_opcode),  CU_per_opcode = 79.5
// The denominator is CU per INDIVIDUAL EVM OPCODE, not per loop iteration:
// `native_work` is summed with `steps_executed`, which counts opcodes. Two
// independent calibrations agree — a 15-opcode loop body at 1192.5 CU/iteration
// (79.5) and a 4-opcode delta between two probes at 315 CU (78.75).
// Re-measure all of these together if the interpreter's per-opcode cost moves;
// a stale denominator silently rescales every constant below.
//
// ECADD measured at ~0 CU (below a 32-byte identity memcpy) and is floored to 1
// so that no precompile is free. HASH_* serve sha256, ripemd160 and identity
// from one pair of constants, so the per-word figure is the MAX of the three:
// ripemd160 is ~8x sha256 (2179 vs 257 CU/word), which overprices the other two.
// That direction is deliberate — over-pricing refuses early, under-pricing does
// not — but it means HASH_PER_WORD cannot be lowered by measuring sha256 alone.
pub const PRECOMPILE_STEPS_PAIRING_BASE: u64 = 278;
pub const PRECOMPILE_STEPS_PAIRING_PER_PAIR: u64 = 161;
pub const PRECOMPILE_STEPS_BLAKE2F_BASE: u64 = 19;
pub const PRECOMPILE_STEPS_BLAKE2F_PER_ROUND: u64 = 6;
pub const PRECOMPILE_STEPS_ECMUL: u64 = 53;
pub const PRECOMPILE_STEPS_ECADD: u64 = 1;
pub const PRECOMPILE_STEPS_ECRECOVER: u64 = 323;
pub const PRECOMPILE_STEPS_HASH_BASE: u64 = 29;
pub const PRECOMPILE_STEPS_HASH_PER_WORD: u64 = 28;
pub const SIG_VERIFY_COST: u64 = 5000;
// Sized to fill v1's 4096B packet. Real worst-case leg (bare batch, 4 accounts)
// = 3954B at 3600; 3800 overflows (4154B) — don't bump without re-measuring.
// TODO(#477): fold into single-sourced limits module.
pub const TRANSMIT_CHUNK_SIZE: u64 = 3600;
pub const GAS_VALUE: &[u8] = b"GAS_VALUE";
pub const GAS_PRICE: &[u8] = b"GAS_PRICE";
pub const GAS_RECIPIENT: &[u8] = b"GAS_RECIPIENT";
pub const OWNER_INFO: &[u8] = b"OWNER_INFO";
pub const NUMBER_SYSCALLS_PER_ALLOC_ITER: u64 = 50; // mut be <= 64  (max_instruction_trace_length)
pub const STORAGE_LEN: usize = 256; // must be <= u8::MAX+1
pub const CONTRACT_SOL_WALLET: &[u8] = b"CONTRACT_SOL_WALLET";
pub const RSOL_DECIMALS: u8 = 18;
pub const SOL_DECIMALS: u8 = 9;  // decimals of SOL
pub const TREASURE_SEED: &[u8] = b"TREASURE_SEED";
pub const TREASURE_NUMBER: u64 = 64;
pub const TREASURE_LAMPORTS: u64 = 5000;
pub const TX_HOLDER_MAX_SIZE: usize = 80000;
pub const CHAIN_ID: &[u8] = b"CHAIN_ID";
pub const SLOT: &[u8] = b"SLOT";
pub const TIMESTAMP: &[u8] = b"TIMESTAMP";
pub const STATE_HOLDER_ALLOC_LEN: usize = MAX_PERMITTED_DATA_INCREASE * 10;
pub const MAX_KEYS_WITHOUT_ALT: usize = 28;
pub const EXTERNAL_AUTHORITY: &[u8] = b"EXTERNAL_AUTHORITY";
pub const BRIDGE_PROCESSED_SEED: &[u8] = b"BRIDGE_PROCESSED";
pub const STARTER_UNWRAP_DONE_SEED: &[u8] = b"STARTER_UNWRAP_DONE";
pub const HEAP_USAGE_LOG: &str = "Heap";
pub const GAS_ESTIMATE_HOLDER: u64 = 99999;
pub const GAS_ESTIMATE_SESSION: u64 = 1;
pub const ESTIMATE_AUTHORITY: &[u8] = b"ESTIMATE_AUTHORITY";
pub const DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT: u32 = 200_000;
pub const MICRO_LAMPORTS_PER_LAMPORT: u64 = 1_000_000;
pub const BINCODE_CAP: u64 = 10_000;
pub const OPCODE_LIMIT_MIN: u64 = 500;
/// secp256k1 group order / 2 — EIP-2 low-s bound (s > this ⇒ malleated).
/// Shared by the tx signature check (tx::recovery_from) and the bridge
/// settle path (api::settle_inbound_bridge_v2).
pub const SECP256K1_N_HALF: [u8; 32] = [
    0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4, 0x50, 0x1d, 0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b, 0x20, 0xa0,
];

// Readable form of the Ed25519 authorisation message for unsigned EIP-1559
// bodies (DoTxUnsigned / external V1): this prefix followed by the lowercase
// hex of keccak256(0x02 ‖ rlp). The raw 32-byte hash stays accepted. The
// prefix is Rome-specific ASCII so it is not a Solana transaction (v1 0x81 /
// legacy signature count), an off-chain message (\xffsolana offchain) or a
// SIWS sign-in.
pub const ED25519_AUTH_MESSAGE_PREFIX: &[u8] = b"Rome authorization\n0x";



// STARTER_UNWRAP_MIN/MAX bounds removed: settle_inbound_bridge
// (slot 15) replaces the bounded grant_starter_unwrap. The new
// instruction settles the full bridged_amount, capped only by the
// user's actual SPL ATA balance — no operator-tunable bounds. The
// `STARTER_UNWRAP_DONE_SEED` constant above is kept for
// compatibility with the `StarterUnwrapDone` account-type variant
// (a backwards-compatibility slot in `AccountType::StarterUnwrapDone`),
// but no instruction allocates new ones after this rewrite.
// (v1 itself retired #480; v2 at slot 21.)

elf!(CARGO_PKG_VERSION, env!("CARGO_PKG_VERSION"));
elf!(GIT_HASH, env!("GIT_HASH"));
elf!(RUSTC_VERSION, env!("RUSTC_VERSION"));
elf!(COMPILE_DATETIME, env!("COMPILE_DATETIME"));
elf!(CARGO_CFG_FEATURE, env!("CARGO_CFG_FEATURE"));

// Single self-describing build-info blob, magic-delimited and contiguous in
// `.rodata`. SBPFv3 ELFs are symbol-stripped, so the 5 statics above become
// unaddressable on v3 (their bytes survive but the symbol table that located
// them does not). This blob carries the same fields between fixed markers so
// the reader (rome-sdk rome-evm-client/src/elf.rs) can recover them by scanning
// raw bytes — no symbol table needed. Field order is locked to that reader:
// version, git_hash, rustc_version, compile_datetime, cargo_cfg_feature.
// NB: no `#[used]` (the macro deliberately dropped it in #416) — re-adding it
// would set SHF_GNU_RETAIN on `.rodata` and the v3 loader rejects that.
elf!(
    ROME_BUILD_INFO,
    concat!(
        "<<ROME_BUILD_INFO>>\n",
        env!("CARGO_PKG_VERSION"),
        "\n",
        env!("GIT_HASH"),
        "\n",
        env!("RUSTC_VERSION"),
        "\n",
        env!("COMPILE_DATETIME"),
        "\n",
        env!("CARGO_CFG_FEATURE"),
        "\n<<END_ROME_BUILD_INFO>>"
    )
);


#[cfg(not(feature = "no-logs"))]
#[macro_export]
macro_rules! msg {
    ($msg:expr) => {
        solana_program::msg!($msg)
    };
    ($($arg:tt)*) => (solana_program::msg!(&format!($($arg)*)));
}

#[cfg(feature = "no-logs")]
#[macro_export]
macro_rules! msg {
    ($msg:expr) => {};
    ($($arg:tt)*) => {};
}

/// Diagnostic log: a per-transaction line nothing off-chain consumes (frame
/// trace, per-diff commit lines, InvokeSigned, allocate slots, phase names).
/// Compiled out unless the `verbose-logs` feature is on — each such line is a
/// `sol_log` syscall (~100 CU) plus formatting (hex 20 B ≈ 1.9k CU, a base58
/// pubkey ≈ 1.8k CU), measured under Mollusk 2026-09-09. Lines something DOES
/// read stay `msg!`: `Heap <n>` (rome-sdk heap sizing), `Instruction: …` and
/// `Commit`/`Exit` (batch-trace markers), failure diagnostics.
#[cfg(all(feature = "verbose-logs", not(feature = "no-logs")))]
#[macro_export]
macro_rules! dmsg {
    ($msg:expr) => {
        $crate::msg!($msg)
    };
    ($($arg:tt)*) => ($crate::msg!($($arg)*));
}

#[cfg(not(all(feature = "verbose-logs", not(feature = "no-logs"))))]
#[macro_export]
macro_rules! dmsg {
    ($msg:expr) => {};
    ($($arg:tt)*) => {};
}

#[cfg(all(test, not(feature = "verbose-logs")))]
mod dmsg_tests {
    /// Not `Display`, not `Debug`: this only compiles if `dmsg!` discards its
    /// arguments. Promoting `dmsg!` back to a real log fails the build here.
    struct Opaque;

    #[test]
    fn dmsg_is_inert_without_verbose_logs() {
        let opaque = Opaque;
        dmsg!("{}", opaque);
        dmsg!("fixed");
        let _still_usable = opaque; // nothing moved it into a format call
    }
}


#[cfg(feature = "ci")]
pub mod registration_key {
    solana_program::declare_id!("9u1wj9K3o9KiDFEppSshGdNhopPBKMbzHgtJr5BMhVjD");
}
#[cfg(feature = "mainnet")]
pub mod registration_key {
    solana_program::declare_id!("RMUhpWCmqm9wzb727XFrSPVjDntPYg8TUmb3wAD1oQE");
}
#[cfg(feature = "testnet")]
pub mod registration_key {
    solana_program::declare_id!("RTRxXgJDFccQNxy976KWhrHr1UzF1gYBqeJnH1dvdNQ");
}

