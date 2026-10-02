// Pins the behavior behind README.md's "EVM Semantics Divergences" section:
// BLOCKHASH is keccak256 of the slot number, a pure function of its argument.
// A change that consults real slot hashes must flip this test deliberately.

mod common;

use {
    common::{install_clock_stub, DummyState},
    evm::{Handler, H256, U256},
    rome_evm::state::JournaledState,
    solana_program::{keccak, pubkey::Pubkey},
};

fn keccak_of_number(n: u64) -> H256 {
    let mut be = [0_u8; 32];
    U256::from(n).to_big_endian(&mut be);
    H256::from(keccak::hash(&be).to_bytes())
}

#[test]
fn block_hash_is_keccak_of_the_slot_number_within_the_window() {
    install_clock_stub();
    let program_id = Pubkey::new_unique();
    let state = DummyState::new(&program_id);
    let mut handler = JournaledState::new(&state).unwrap();
    handler.slot = 1_000;

    assert_eq!(Handler::block_hash(&handler, U256::from(999_u64)), keccak_of_number(999));
    assert_eq!(Handler::block_hash(&handler, U256::from(744_u64)), keccak_of_number(744), "distance 256 is in-window");

    assert_eq!(Handler::block_hash(&handler, U256::from(1_000_u64)), H256::zero(), "current slot");
    assert_eq!(Handler::block_hash(&handler, U256::from(1_001_u64)), H256::zero(), "future slot");
    assert_eq!(Handler::block_hash(&handler, U256::from(743_u64)), H256::zero(), "distance 257 is out of window");
}

#[test]
fn block_hash_does_not_depend_on_which_slot_asks() {
    install_clock_stub();
    let program_id = Pubkey::new_unique();
    let state = DummyState::new(&program_id);
    let mut handler = JournaledState::new(&state).unwrap();

    handler.slot = 1_000;
    let seen_from_1000 = Handler::block_hash(&handler, U256::from(900_u64));
    handler.slot = 1_100;
    let seen_from_1100 = Handler::block_hash(&handler, U256::from(900_u64));

    assert_eq!(seen_from_1000, seen_from_1100);
    assert_eq!(seen_from_1000, keccak_of_number(900));
}
