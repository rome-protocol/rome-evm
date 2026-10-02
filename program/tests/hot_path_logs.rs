//! Source-shape guard for the program's per-transaction text logs.
//!
//! Every `msg!` line on the transaction hot path costs a `sol_log` syscall
//! (~100 CU) plus its formatting — measured under Mollusk (2026-09-09):
//! a fixed string 103 CU, `from {hex}` 1,982, `Call: from {hex}, to {hex}`
//! 3,758, `InvokeSigned {pubkey}` 1,796 (base58), `allocate slots {pubkey}`
//! 1,834. A warm SPL transfer emitted 14 such lines (~8.7k CU, 5% of the tx);
//! a warm UniswapV2 swap 46 (~43k CU). Nothing off-chain parses those
//! diagnostic lines, so they are `dmsg!` (compiled out unless the
//! `verbose-logs` feature is on).
//!
//! What MUST stay a plain `msg!` because something consumes it:
//! - `Heap <used>` (entrypoint) — rome-sdk `heap_usage` reads it from
//!   emulation logs to decide atomic vs iterative;
//! - `Instruction: ...` entry lines (api/) — human-readable and the family
//!   `parse_batch_trace` keys on;
//! - `Commit` / `Exit` — `parse_batch_trace` end markers;
//! - failure-path diagnostics (`Revert:`, `TxFailed`, `UnnecessaryIteration`,
//!   `non-evm call error`, `Error:`) — only paid when the tx already failed.
//!
//! The test reads the source files themselves, so re-adding a per-transfer
//! `msg!("Execute")` (or promoting a `dmsg!` back to `msg!`) fails here.

use std::{collections::BTreeSet, fs, path::PathBuf};

fn src(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Literal first arguments of every `msg!(` (not `dmsg!(`) call in `text`,
/// in order of appearance; formatted lines keep their format string.
fn msg_literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("msg!(") {
        let before = &rest[..i];
        let is_dmsg = before.ends_with('d');
        let after = &rest[i + "msg!(".len()..];
        if !is_dmsg {
            let lit = after
                .trim_start()
                .strip_prefix('"')
                .and_then(|s| s.split('"').next())
                .unwrap_or("<non-literal>")
                .to_string();
            out.push(lit);
        }
        rest = after;
    }
    out
}

fn allowed(file: &str) -> BTreeSet<&'static str> {
    let list: &[&str] = match file {
        // `Error` only fires on a failed atomic tx.
        "vm/vm_atomic.rs" => &["Commit", "Error", "Exit"],
        "vm/vm_iterative.rs" => &["Commit", "UnnecessaryIteration: {}", "reason: {:?}", "TxFailed: {}", "UnlockFailedTx"],
        "vm/vm_eth_call.rs" => &[],
        // Revert diagnostics only fire on a reverted frame.
        "vm/vm.rs" => &["Revert: {:?}", "Revert panic: {:?})"],
        "state/journal.rs" => &[],
        "state/allocate.rs" => &[],
        // Error path only.
        "state/handler_non_evm.rs" => &["non-evm call error: {}"],
        _ => panic!("no allow-list for {file}"),
    };
    list.iter().copied().collect()
}

#[test]
fn hot_path_files_log_only_the_consumed_or_failure_lines() {
    for file in [
        "vm/vm.rs",
        "vm/vm_atomic.rs",
        "vm/vm_iterative.rs",
        "vm/vm_eth_call.rs",
        "state/journal.rs",
        "state/allocate.rs",
        "state/handler_non_evm.rs",
    ] {
        // Production code only — tests inside these files may log freely.
        let text = src(file);
        let prod = text.split("#[cfg(test)]").next().unwrap_or("");
        let found: BTreeSet<String> = msg_literals(prod).into_iter().collect();
        let allow = allowed(file);
        let extra: Vec<&String> = found.iter().filter(|l| !allow.contains(l.as_str())).collect();
        assert!(
            extra.is_empty(),
            "{file}: plain msg! on the hot path that nothing consumes — make it dmsg! (verbose-logs): {extra:?}"
        );
    }
}

#[test]
fn the_consumed_markers_are_still_emitted() {
    // Removing these would silently break rome-sdk heap sizing and the
    // batch-trace parser; they are cheap fixed strings.
    assert!(src("entrypoint.rs").contains("HEAP_USAGE_LOG"), "Heap usage log must stay");
    assert!(src("vm/vm_atomic.rs").contains("msg!(\"Commit\")"), "atomic Commit marker must stay");
    assert!(src("vm/vm_atomic.rs").contains("msg!(\"Exit\")"), "atomic Exit marker must stay");
    assert!(src("vm/vm_iterative.rs").contains("msg!(\"Commit\")"), "iterative Commit marker must stay");
    assert!(src("api/do_tx.rs").contains("msg!(\"Instruction: Atomic transaction\")"), "Instruction entry line must stay");
}

#[test]
fn every_diagnostic_line_is_gated() {
    // The lines measured as the expensive ones must all be dmsg! now.
    let vm = src("vm/vm.rs");
    assert!(vm.contains("dmsg!(\n            \"Call: from {}, to {}\"") || vm.contains("dmsg!(\"Call: from {}, to {}\""), "Call frame log must be dmsg!");
    assert!(vm.contains("dmsg!(\"from {}\""), "sender log must be dmsg!");
    let journal = src("state/journal.rs");
    assert!(journal.contains("dmsg!(\"InvokeSigned {}\""), "InvokeSigned log must be dmsg!");
    let alloc = src("state/allocate.rs");
    assert_eq!(alloc.matches("dmsg!(\"allocate slots {}, slots {}\"").count(), 2, "both allocate logs must be dmsg!");
    let atomic = src("vm/vm_atomic.rs");
    for l in ["Lock", "Init", "Execute"] {
        assert!(atomic.contains(&format!("dmsg!(\"{l}\")")), "atomic {l} must be dmsg!");
    }
}
