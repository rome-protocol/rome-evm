mod allocate;
#[allow(clippy::let_and_return)]
pub mod api;
mod context;
pub mod entrypoint;
mod origin;
mod state;
pub mod stubs   ;
pub mod account_storage;
pub mod api_offchain;

pub use api::*;
pub use context::*;
pub use state::{Bind, Item, ix_store,};
pub use api_offchain::*;

use {
    solana_program::{
        pubkey::Pubkey, instruction::Instruction as Ix,
    },
    crate::{
        account_storage::AccountStorage as A,
    },
    rome_evm::error::Result,
    std::sync::Arc,
};

fn unsupported<'a>(_: &'a Pubkey, _: &'a [u8], _: &'a Pubkey, _: Arc<dyn A>, _: Option<Ix>) -> Result<Emulation> {
    unimplemented!();
}

entrypoint! {
    DoTx => do_tx,
    Deposit => deposit,
    TransmitTx => transmit_tx,
    DoTxHolder => do_tx_holder,
    DoTxIterative => do_tx_iterative,
    DoTxHolderIterative => do_tx_holder_iterative,
    RegOwner => reg_owner,
    AltAlloc => unsupported,
    AltDealloc => unsupported,
    CreateTreasure => create_treasure,
    JoinTreasure => join_treasure,
    AllocResource => alloc_resource,
    DeallocResource => dealloc_resource,
    MetaHook => unsupported,
    _RetiredSettleInbound => unsupported,
    SettleInboundBridge => unsupported,
    DoTxBatch => unsupported,
    DoTxUnsigned => unsupported,
    ActivateAta => unsupported,
    DoCall => do_call,                          // is prohibited to execute on-chain
    DoCallIterative => do_call_iterative,       // is prohibited to execute on-chain
    SettleInboundBridgeV2 => unsupported,
}

pub const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use crate::Instruction;

    // The program and this emulator each expand their OWN `entrypoint!`
    // invocation into an independent `Instruction` enum — there is no shared
    // type. `dispatch()` on both sides matches the wire byte `d[0]` against
    // `Instruction::$row as u8`, which the compiler assigns from each enum's
    // own textual order. A mid-list insert on either side reshuffles every
    // later `as u8` value on that side only, silently desyncing `eth_call`
    // (this crate) from real on-chain execution (`rome_evm`) — the failure
    // mode `program/src/lib.rs::tests::dispatch_slot_assignments` guards
    // against on the program side alone, with no emulator-side check.
    //
    // This asserts ordinal alignment ONLY, never handler equality: handlers
    // legitimately differ (`DoTxUnsigned`/`ActivateAta`/`SettleInboundBridgeV2`
    // are real here vs `unsupported` in the emulator by design). Both values
    // below are computed live from each crate's current declaration order,
    // not hardcoded literals, so a shift on either side breaks the match.
    #[test]
    fn dispatch_ordinals_match_program() {
        let pairs: &[(&str, u8, u8)] = &[
            ("DoTx", rome_evm::Instruction::DoTx as u8, Instruction::DoTx as u8),
            ("Deposit", rome_evm::Instruction::Deposit as u8, Instruction::Deposit as u8),
            ("TransmitTx", rome_evm::Instruction::TransmitTx as u8, Instruction::TransmitTx as u8),
            ("DoTxHolder", rome_evm::Instruction::DoTxHolder as u8, Instruction::DoTxHolder as u8),
            ("DoTxIterative", rome_evm::Instruction::DoTxIterative as u8, Instruction::DoTxIterative as u8),
            ("DoTxHolderIterative", rome_evm::Instruction::DoTxHolderIterative as u8, Instruction::DoTxHolderIterative as u8),
            ("RegOwner", rome_evm::Instruction::RegOwner as u8, Instruction::RegOwner as u8),
            ("AltAlloc", rome_evm::Instruction::AltAlloc as u8, Instruction::AltAlloc as u8),
            ("AltDealloc", rome_evm::Instruction::AltDealloc as u8, Instruction::AltDealloc as u8),
            ("CreateTreasure", rome_evm::Instruction::CreateTreasure as u8, Instruction::CreateTreasure as u8),
            ("JoinTreasure", rome_evm::Instruction::JoinTreasure as u8, Instruction::JoinTreasure as u8),
            ("AllocResource", rome_evm::Instruction::AllocResource as u8, Instruction::AllocResource as u8),
            ("DeallocResource", rome_evm::Instruction::DeallocResource as u8, Instruction::DeallocResource as u8),
            ("MetaHook", rome_evm::Instruction::MetaHook as u8, Instruction::MetaHook as u8),
            ("_RetiredSettleInbound", rome_evm::Instruction::_RetiredSettleInbound as u8, Instruction::_RetiredSettleInbound as u8),
            ("SettleInboundBridge", rome_evm::Instruction::SettleInboundBridge as u8, Instruction::SettleInboundBridge as u8),
            ("DoTxBatch", rome_evm::Instruction::DoTxBatch as u8, Instruction::DoTxBatch as u8),
            ("DoTxUnsigned", rome_evm::Instruction::DoTxUnsigned as u8, Instruction::DoTxUnsigned as u8),
            ("ActivateAta", rome_evm::Instruction::ActivateAta as u8, Instruction::ActivateAta as u8),
            ("DoCall", rome_evm::Instruction::DoCall as u8, Instruction::DoCall as u8),
            ("DoCallIterative", rome_evm::Instruction::DoCallIterative as u8, Instruction::DoCallIterative as u8),
            ("SettleInboundBridgeV2", rome_evm::Instruction::SettleInboundBridgeV2 as u8, Instruction::SettleInboundBridgeV2 as u8),
        ];

        for (name, program_ordinal, emulator_ordinal) in pairs {
            assert_eq!(
                program_ordinal, emulator_ordinal,
                "slot {name}: program={program_ordinal} emulator={emulator_ordinal} — \
                 dispatch tables disagree on this slot's byte position"
            );
        }
    }

    // The pairwise check above only proves the 22 NAMED slots agree — it can't
    // see a variant appended at the end of just one side's `entrypoint!` list,
    // because an append doesn't shift any existing `as u8` value (only a
    // mid-list insert does). A `len() == 22` assert over this test's own
    // literal array is a tautology, not a check on the enum — it always
    // passes regardless of what either enum actually contains.
    //
    // These two functions close that gap at COMPILE time: an exhaustive
    // `match` with no wildcard arm fails to build the moment either enum
    // gains a variant not listed here, independent of whether the functions
    // are ever called. Ordinal alignment ONLY — the match arms are empty, so
    // this proves nothing about handler behavior (handlers legitimately
    // differ by design).
    #[allow(dead_code)]
    fn assert_program_variants_are_all_listed(v: rome_evm::Instruction) {
        match v {
            rome_evm::Instruction::DoTx => {}
            rome_evm::Instruction::Deposit => {}
            rome_evm::Instruction::TransmitTx => {}
            rome_evm::Instruction::DoTxHolder => {}
            rome_evm::Instruction::DoTxIterative => {}
            rome_evm::Instruction::DoTxHolderIterative => {}
            rome_evm::Instruction::RegOwner => {}
            rome_evm::Instruction::AltAlloc => {}
            rome_evm::Instruction::AltDealloc => {}
            rome_evm::Instruction::CreateTreasure => {}
            rome_evm::Instruction::JoinTreasure => {}
            rome_evm::Instruction::AllocResource => {}
            rome_evm::Instruction::DeallocResource => {}
            rome_evm::Instruction::MetaHook => {}
            rome_evm::Instruction::_RetiredSettleInbound => {}
            rome_evm::Instruction::SettleInboundBridge => {}
            rome_evm::Instruction::DoTxBatch => {}
            rome_evm::Instruction::DoTxUnsigned => {}
            rome_evm::Instruction::ActivateAta => {}
            rome_evm::Instruction::DoCall => {}
            rome_evm::Instruction::DoCallIterative => {}
            rome_evm::Instruction::SettleInboundBridgeV2 => {}
        }
    }

    #[allow(dead_code)]
    fn assert_emulator_variants_are_all_listed(v: Instruction) {
        match v {
            Instruction::DoTx => {}
            Instruction::Deposit => {}
            Instruction::TransmitTx => {}
            Instruction::DoTxHolder => {}
            Instruction::DoTxIterative => {}
            Instruction::DoTxHolderIterative => {}
            Instruction::RegOwner => {}
            Instruction::AltAlloc => {}
            Instruction::AltDealloc => {}
            Instruction::CreateTreasure => {}
            Instruction::JoinTreasure => {}
            Instruction::AllocResource => {}
            Instruction::DeallocResource => {}
            Instruction::MetaHook => {}
            Instruction::_RetiredSettleInbound => {}
            Instruction::SettleInboundBridge => {}
            Instruction::DoTxBatch => {}
            Instruction::DoTxUnsigned => {}
            Instruction::ActivateAta => {}
            Instruction::DoCall => {}
            Instruction::DoCallIterative => {}
            Instruction::SettleInboundBridgeV2 => {}
        }
    }

    // Exercises both exhaustive matches so they're live code, not just
    // compiled-but-unreferenced (the `#[allow(dead_code)]` above covers the
    // case where a reviewer removes this call — either way the exhaustiveness
    // check is a property of the match existing, not of it running).
    #[test]
    fn dispatch_variants_are_exhaustively_listed() {
        assert_program_variants_are_all_listed(rome_evm::Instruction::DoTx);
        assert_emulator_variants_are_all_listed(Instruction::DoTx);
    }
}
