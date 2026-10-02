// FIND-024 (Halborn, DISCLOSE — no production code). Pins the current,
// unchanged behavior behind README.md's "EVM Semantics Divergences" section:
// Rome has no gas schedule, so the gas operand of CALL/CALLCODE/DELEGATECALL/
// STATICCALL/CREATE bounds nothing, and the 2300-gas reentrancy stipend does
// not exist. A future gasometer PR must flip these tests deliberately, not
// silently.

mod common;

use {
    common::{install_clock_stub, DummyState},
    evm::{Capture, Context, Handler, H160, U256},
    rome_evm::state::JournaledState,
    solana_program::pubkey::Pubkey,
};

/// `Handler::call`'s gas operand is `_: Option<u64>` in `handler.rs` — discarded
/// at the signature, never read, and `CallInterrupt` (the struct actually
/// threaded onward to `trap_call`) carries no gas field at all. A call made
/// with a tiny gas value (the historical `transfer()`/`send()` 2300 stipend)
/// proceeds to exactly the same `Capture::Trap` as one made with `u64::MAX`
/// or no gas operand at all — nothing about the request is throttled by it.
#[test]
fn call_gas_operand_never_gates_the_call() {
    install_clock_stub();
    let program_id = Pubkey::new_unique();
    let state = DummyState::new(&program_id);
    let mut handler = JournaledState::new(&state).unwrap();

    let ctx = Context { address: H160::zero(), caller: H160::zero(), apparent_value: U256::zero() };
    let stipend_call = handler.call(H160::repeat_byte(1), None, vec![], Some(2300), false, ctx.clone());
    let unbounded_call = handler.call(H160::repeat_byte(1), None, vec![], Some(u64::MAX), false, ctx.clone());
    let no_gas_operand_call = handler.call(H160::repeat_byte(1), None, vec![], None, false, ctx);

    for capture in [stipend_call, unbounded_call, no_gas_operand_call] {
        match capture {
            Capture::Trap(interrupt) => {
                assert_eq!(interrupt.code_address, H160::repeat_byte(1));
                assert!(!interrupt.is_static, "gas value must not force static mode");
            }
            Capture::Exit(_) => panic!("gas operand must never itself cause an early exit"),
        }
    }
}

/// `gas_left()` (`handler.rs`) returns the transaction's `gas_limit` verbatim
/// (or `U256::MAX` with no fee recipient) — never a decrementing per-opcode
/// counter. `GAS`/`gasleft()` in Solidity reads this.
#[test]
fn gas_left_returns_tx_gas_limit_not_a_decrementing_counter() {
    install_clock_stub();
    let program_id = Pubkey::new_unique();
    let state = DummyState::new(&program_id);
    let mut handler = JournaledState::new(&state).unwrap();
    handler.gas_recipient = Some(H160::zero());
    handler.gas_limit = Some(U256::from(30_000_000_u64));

    assert_eq!(Handler::gas_left(&handler), U256::from(30_000_000_u64));
    // No opcode execution happened between construction and this read —
    // there is no mechanism that would have decremented it if it had.
}
