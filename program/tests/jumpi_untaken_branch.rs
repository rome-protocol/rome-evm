//! The program links the interpreter that gives `JUMPI` Ethereum semantics on
//! the untaken branch: a zero condition continues whatever the destination
//! operand holds; the destination is converted and validated only when the
//! jump is taken. Pinned here so a dependency bump cannot silently regress it.

use evm::{Capture, Context, ExitError, ExitReason, ExitSucceed, Machine, Valids, H160, U256};

const STACK_LIMIT: usize = 1024;
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;

fn push32(code: &mut Vec<u8>, val: U256) {
    code.push(0x7f);
    let mut buf = [0u8; 32];
    val.to_big_endian(&mut buf);
    code.extend_from_slice(&buf);
}

fn run(code: Vec<u8>) -> Capture<ExitReason, evm::Trap> {
    let valids = Valids::compute(&code);
    let mut machine = Machine::new(code, valids, Vec::new(), STACK_LIMIT, MEMORY_LIMIT);
    let ok = |_, _: &_| Ok(());
    let ctx = Context { address: H160::zero(), caller: H160::zero(), apparent_value: U256::zero() };
    machine.run(1000, ok, &ctx).1
}

#[test]
fn untaken_jumpi_ignores_destination_above_usize_max() {
    let mut code = Vec::new();
    push32(&mut code, U256::zero()); // condition = 0
    push32(&mut code, U256::max_value()); // dest, unrepresentable as usize
    code.push(0x57); // JUMPI
    push32(&mut code, U256::from(1));
    push32(&mut code, U256::zero());
    code.push(0xf3); // RETURN
    assert_eq!(run(code), Capture::Exit(ExitReason::Succeed(ExitSucceed::Returned)));
}

#[test]
fn taken_jumpi_with_destination_above_usize_max_is_invalid_jump() {
    let mut code = Vec::new();
    push32(&mut code, U256::from(1));
    push32(&mut code, U256::max_value());
    code.push(0x57);
    assert_eq!(run(code), Capture::Exit(ExitReason::Error(ExitError::InvalidJump)));
}
