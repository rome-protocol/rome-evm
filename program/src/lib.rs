#![cfg_attr(feature = "no-logs", allow(unused_variables))]

pub mod accounts;
mod alloc;
pub mod api;
pub mod assert;
mod config;
pub mod context;
mod entrypoint;
pub mod error;
pub mod precompile;
pub mod state;
pub mod tx;
pub mod vm;
pub mod non_evm;
pub mod non_evm_cached;
pub mod api_offchain;

pub use accounts::*;
pub use assert::*;
pub use config::*;
pub use evm::{ExitReason, Valids as EvmValids, H160, H256, U256, Context};
pub use state::*;

use api::*;
use api_offchain::*;

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
    DoTxUnsigned => do_tx_unsigned,
    ActivateAta => activate_ata,
    DoCall => do_call,                          // is prohibited to execute on-chain
    DoCallIterative => do_call_iterative,       // is prohibited to execute on-chain
    SettleInboundBridgeV2 => settle_inbound_bridge_v2,
}


use solana_program::{account_info::AccountInfo, pubkey::Pubkey};
use error::{Result, RomeProgramError};

#[inline(never)]
pub fn unsupported<'a>(
    _: &'a Pubkey,
    _: &'a [AccountInfo<'a>],
    _: &'a [u8],
) -> Result<()> {
    Err(RomeProgramError::Custom(
        "instruction retired: see lib.rs unsupported() for the slot map"
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `DoTxBatch` (slot 16) is retired to `unsupported` — VmBatch,
    /// tx/batch.rs, vm/vm_batch.rs and api/do_tx_batch.rs are all deleted.
    /// Feeding a `0x7d` batch envelope at slot 16 must hit the `unsupported`
    /// stub (fails cleanly), NOT attempt batch parsing/execution.
    #[test]
    fn do_tx_batch_slot_is_unsupported() {
        let program_id = Pubkey::new_unique();
        let accounts: Vec<AccountInfo> = vec![];
        let mut data = vec![Instruction::DoTxBatch as u8];
        data.extend_from_slice(&[0x7d, 0xc0]); // minimal 0x7d-prefixed payload

        let result = instruction::dispatch(&program_id, &accounts, &data);
        match result {
            Err(RomeProgramError::Custom(msg)) => {
                assert!(
                    msg.contains("instruction retired"),
                    "expected the unsupported-stub message, got: {msg}"
                );
            }
            other => panic!(
                "expected Err(Custom(\"instruction retired...\")), got {:?}",
                other
            ),
        }
    }

    /// Relocated from `tx/batch.rs::dispatch_slot_assignments` (deleted along
    /// with VmBatch removal). Guards against accidental instruction
    /// reordering — byte position IS the dispatch index; a mid-list insert
    /// would shift every later slot and break deployed rome-sdk clients that
    /// hard-code byte values. New slots are appended at the end only.
    #[test]
    fn dispatch_slot_assignments() {
        // Existing slots — must not shift.
        assert_eq!(Instruction::DoTx as u8, 0);
        assert_eq!(Instruction::Deposit as u8, 1);
        assert_eq!(Instruction::TransmitTx as u8, 2);
        assert_eq!(Instruction::DoTxHolder as u8, 3);
        assert_eq!(Instruction::DoTxIterative as u8, 4);
        assert_eq!(Instruction::DoTxHolderIterative as u8, 5);
        assert_eq!(Instruction::AltAlloc as u8, 7);
        assert_eq!(Instruction::AltDealloc as u8, 8);
        assert_eq!(Instruction::SettleInboundBridge as u8, 15);
        assert_eq!(Instruction::DoTxBatch as u8, 16);
    }

    /// ALT/v0 removal: `AltAlloc`/`AltDealloc` (slots 7/8) retire to
    /// `unsupported`, kept in place so an old proxy sending those slots
    /// fails clean instead of misrouting.
    #[test]
    fn alt_slots_are_unsupported() {
        assert_retired(Instruction::AltAlloc as u8);
        assert_retired(Instruction::AltDealloc as u8);
    }

    /// Free function, not a loop body: `dispatch`'s single `'a` unifies
    /// `program_id`/`accounts`/`data`, so a shared `accounts` vec declared
    /// once outside a loop can't be reused against a per-iteration `data`.
    fn assert_retired(tag: u8) {
        let program_id = Pubkey::new_unique();
        let accounts: Vec<AccountInfo> = vec![];
        let data = vec![tag, 0xde, 0xad];

        let result = instruction::dispatch(&program_id, &accounts, &data);
        match result {
            Err(RomeProgramError::Custom(msg)) => {
                assert!(
                    msg.contains("instruction retired"),
                    "expected the unsupported-stub message for tag {tag}, got: {msg}"
                );
            }
            other => panic!(
                "expected Err(Custom(\"instruction retired...\")) for tag {tag}, got {:?}",
                other
            ),
        }
    }
}
