// Attacker-chosen calldata reaching a non-EVM program must fail the frame
// with a typed error, never abort the Solana instruction.

mod common;

use {
    common::{install_clock_stub, DummyState},
    evm::{Context, ExitReason, ExitRevert, ExitSucceed, H160, U256},
    rome_evm::{
        error::RomeProgramError::InvalidNonEvmInstructionData,
        non_evm::{CpiProgram, HelperProgram, System, Withdraw},
        non_evm_cached::{ASplCached, SplCached, SystemCached, WithdrawCached},
        precompile::{ecrecover::Ecrecover, identity::Identity, sha2_256::Sha2},
        state::{handler::CallInterrupt, JournaledState},
    },
    solana_program::pubkey::Pubkey,
};

type State<'a> = DummyState<'a>;

fn call(code_address: H160, input: Vec<u8>) -> CallInterrupt {
    CallInterrupt {
        code_address,
        transfer: None,
        input,
        is_static: false,
        context: Context { address: H160::zero(), caller: H160::zero(), apparent_value: U256::zero() },
    }
}

fn dispatch(handler: &mut JournaledState<'_, State<'_>>, code_address: H160, input: Vec<u8>) -> (ExitReason, Vec<u8>) {
    let program = handler.non_evm_program(&code_address).expect("known non-EVM address");
    handler.handle_non_evm_call(program, call(code_address, input), true)
}

const REVERTED: ExitReason = ExitReason::Revert(ExitRevert::Reverted);
const RETURNED: ExitReason = ExitReason::Succeed(ExitSucceed::Returned);

#[test]
fn dispatcher_calldata_shorter_than_a_selector_reverts() {
    install_clock_stub();
    let program_id = Pubkey::new_unique();
    let state = State::new(&program_id);
    let mut handler = JournaledState::new(&state).unwrap();

    let dispatchers = [
        CpiProgram::<State>::ADDRESS,
        System::<State>::ADDRESS,
        Withdraw::<State>::ADDRESS,
        HelperProgram::<State>::ADDRESS,
        SplCached::<State>::ADDRESS,
        ASplCached::<State>::ADDRESS,
        SystemCached::<State>::ADDRESS,
        WithdrawCached::<State>::ADDRESS,
    ];

    for address in dispatchers {
        for len in 0..4 {
            let (reason, _) = dispatch(&mut handler, address, vec![0xab; len]);
            assert_eq!(reason, REVERTED, "{address:?} with {len} bytes of calldata");
        }
    }
}

#[test]
fn precompiles_still_accept_input_shorter_than_a_selector() {
    install_clock_stub();
    let program_id = Pubkey::new_unique();
    let state = State::new(&program_id);
    let mut handler = JournaledState::new(&state).unwrap();

    let (reason, out) = dispatch(&mut handler, Sha2::ADDRESS, vec![]);
    assert_eq!(reason, RETURNED);
    assert_eq!(
        out,
        hex::decode("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855").unwrap(),
        "sha256 of the empty string"
    );

    let (reason, out) = dispatch(&mut handler, Identity::ADDRESS, vec![0x42]);
    assert_eq!(reason, RETURNED);
    assert_eq!(out, vec![0x42]);

    let (reason, out) = dispatch(&mut handler, Ecrecover::ADDRESS, vec![1, 2, 3]);
    assert_eq!(reason, RETURNED);
    assert!(out.is_empty(), "zero-padded v is neither 27 nor 28, so ecrecover returns empty");
}

#[test]
fn account_info_with_a_short_pubkey_word_reverts() {
    install_clock_stub();
    let program_id = Pubkey::new_unique();
    let state = State::new(&program_id);
    let mut handler = JournaledState::new(&state).unwrap();

    // account_info(bytes32) selector followed by 31 bytes instead of 32.
    let mut input = vec![0xc1, 0x34, 0x65, 0xd9];
    input.extend_from_slice(&[0x11; 31]);

    let (reason, _) = dispatch(&mut handler, CpiProgram::<State>::ADDRESS, input);
    assert_eq!(reason, REVERTED);
}

/// ABI tail for `(bytes32 program, bytes[] seeds)` as `System::find_pda` reads it.
fn encode_find_pda_args(program: &Pubkey, seeds: &[&[u8]]) -> Vec<u8> {
    fn word(n: usize) -> [u8; 32] {
        let mut w = [0_u8; 32];
        w[24..].copy_from_slice(&(n as u64).to_be_bytes());
        w
    }

    let mut out = Vec::new();
    out.extend_from_slice(&program.to_bytes());
    out.extend_from_slice(&word(64));
    out.extend_from_slice(&word(seeds.len()));

    let mut heads = Vec::new();
    let mut tails = Vec::new();
    let mut cursor = 32 * seeds.len();
    for seed in seeds {
        heads.extend_from_slice(&word(cursor));
        tails.extend_from_slice(&word(32));
        tails.extend_from_slice(&word(seed.len()));
        let padded = seed.len().div_ceil(32) * 32;
        let mut data = vec![0_u8; padded];
        data[..seed.len()].copy_from_slice(seed);
        tails.extend_from_slice(&data);
        cursor += 64 + padded;
    }
    out.extend_from_slice(&heads);
    out.extend_from_slice(&tails);
    out
}

#[test]
fn find_pda_rejects_seed_shapes_the_runtime_refuses() {
    let program_id = Pubkey::new_unique();
    let state = State::new(&program_id);
    let system = System::new(&state);
    let target = Pubkey::new_unique();

    let seventeen: Vec<Vec<u8>> = (0..17_u8).map(|i| vec![i]).collect();
    let seventeen: Vec<&[u8]> = seventeen.iter().map(Vec::as_slice).collect();
    let err = system.find_pda(&encode_find_pda_args(&target, &seventeen)).unwrap_err();
    assert!(matches!(err, InvalidNonEvmInstructionData), "17 seeds: {err:?}");

    let long = [0x5a_u8; 33];
    let err = system.find_pda(&encode_find_pda_args(&target, &[&long])).unwrap_err();
    assert!(matches!(err, InvalidNonEvmInstructionData), "33-byte seed: {err:?}");
}

/// The reject cases above are also what the host-target fallback returns, so they cannot
/// tell the explicit bound from its absence. These accept cases can: they pin it at the
/// runtime's own limits, so tightening it rejects a shape the syscall derives fine.
/// Fifteen is the most seeds a caller can pass, because the bump occupies the sixteenth.
#[test]
fn find_pda_accepts_the_largest_seed_shape_the_runtime_allows() {
    let program_id = Pubkey::new_unique();
    let state = State::new(&program_id);
    let system = System::new(&state);
    let target = Pubkey::new_unique();

    let fifteen: Vec<Vec<u8>> = (0..15_u8).map(|i| vec![i]).collect();
    let fifteen: Vec<&[u8]> = fifteen.iter().map(Vec::as_slice).collect();
    let val = system
        .find_pda(&encode_find_pda_args(&target, &fifteen))
        .expect("15 seeds plus the bump is the runtime maximum");
    assert_eq!(val.len(), 64);

    let full = [0x5a_u8; 32];
    let val = system
        .find_pda(&encode_find_pda_args(&target, &[&full]))
        .expect("a 32-byte seed is the runtime maximum");
    assert_eq!(val.len(), 64);
}

#[test]
fn find_pda_matches_the_runtime_derivation() {
    let program_id = Pubkey::new_unique();
    let state = State::new(&program_id);
    let system = System::new(&state);
    let target = Pubkey::new_unique();
    let seeds: &[&[u8]] = &[b"rome", b"find_pda", &[0xde, 0xad]];

    let (key, bump) = Pubkey::find_program_address(seeds, &target);
    let out = system.find_pda(&encode_find_pda_args(&target, seeds)).unwrap();

    assert_eq!(out.len(), 64);
    assert_eq!(&out[..32], &key.to_bytes());
    assert_eq!(&out[32..63], &[0_u8; 31]);
    assert_eq!(out[63], bump);
}
