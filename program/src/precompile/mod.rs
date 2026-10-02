pub mod blake2f;
pub mod ecadd;
pub mod ecmul;
pub mod ecpairing;
pub mod ecrecover;
pub mod identity;
pub mod modexp;
pub mod ripemd_160;
pub mod sha2_256;


macro_rules! impl_contract {
    ($name:ident, $address:expr) => {
        use {
            crate::{
                non_evm::{Program, NonEvmCall}, error::Result,
                handler::CallInterrupt, non_evm::aux::not_payable,
            },
        };

        pub struct $name();

        impl $name {
            pub const ADDRESS: H160 = H160($address);
        }

        impl Program for $name {
            fn from_abi<'b>(&self, params: &'b CallInterrupt) -> Result<NonEvmCall<'b>> {
                not_payable(params)?;
                Ok(NonEvmCall::Precompiled(&params.input))
            }
            fn eth_call(&self, _: &[u8], input: &[u8]) -> Result<Vec<u8>> {
                contract(input)
            }
            fn cross_state_call(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
                unreachable!()
            }
            fn precompile(&self) -> bool {
                true
            }
        }
    };
}

pub(crate) use  impl_contract;

use evm::H160;
use crate::config::{
    PRECOMPILE_STEPS_PAIRING_BASE, PRECOMPILE_STEPS_PAIRING_PER_PAIR,
    PRECOMPILE_STEPS_BLAKE2F_BASE, PRECOMPILE_STEPS_BLAKE2F_PER_ROUND,
    PRECOMPILE_STEPS_ECMUL, PRECOMPILE_STEPS_ECADD, PRECOMPILE_STEPS_ECRECOVER,
    PRECOMPILE_STEPS_HASH_BASE, PRECOMPILE_STEPS_HASH_PER_WORD,
};

/// Prices a precompile call's native work into the same unit as interpreter
/// opcode steps — pure function of address + input, no syscall, no
/// state. Called at `Vm::trap_call` dispatch, BEFORE the body runs, so an
/// oversized input can be refused without executing it. See config.rs for
/// the (unmeasured, ship-gated) constants. Unknown address -> 0.
pub fn step_price(address: &H160, input: &[u8]) -> u64 {
    match *address {
        _ if *address == ecpairing::Ecpairing::ADDRESS => {
            // Price only the pairs the body will actually process: a length that
            // is not a whole number of elements is refused before any syscall,
            // so charging for it would let 19 MB of malformed input exhaust the
            // request budget without doing work.
            let element = solana_bn254::prelude::ALT_BN128_PAIRING_ELEMENT_SIZE;
            let pairs = if input.len() % element == 0 { (input.len() / element) as u64 } else { 0 };
            PRECOMPILE_STEPS_PAIRING_BASE.saturating_add(PRECOMPILE_STEPS_PAIRING_PER_PAIR.saturating_mul(pairs))
        }
        _ if *address == blake2f::Blake2f::ADDRESS => {
            // Clamp to MAX_ROUNDS: the body refuses anything above it without
            // running a round, so an unclamped price lets a 4-byte rounds field
            // charge orders of magnitude more than the whole request budget.
            let rounds = if input.len() == blake2f::BLAKE2_INPUT_LEN {
                let requested = u32::from_be_bytes(input[0..4].try_into().unwrap_or([0; 4]));
                if requested > blake2f::MAX_ROUNDS { 0 } else { requested as u64 }
            } else {
                0
            };
            PRECOMPILE_STEPS_BLAKE2F_BASE.saturating_add(PRECOMPILE_STEPS_BLAKE2F_PER_ROUND.saturating_mul(rounds))
        }
        _ if *address == ecmul::Ecmul::ADDRESS => PRECOMPILE_STEPS_ECMUL,
        _ if *address == ecadd::Ecadd::ADDRESS => PRECOMPILE_STEPS_ECADD,
        _ if *address == ecrecover::Ecrecover::ADDRESS => PRECOMPILE_STEPS_ECRECOVER,
        _ if *address == sha2_256::Sha2::ADDRESS
            || *address == ripemd_160::Ripemd::ADDRESS
            || *address == identity::Identity::ADDRESS => {
            let words = (input.len() as u64).div_ceil(32);
            PRECOMPILE_STEPS_HASH_BASE.saturating_add(PRECOMPILE_STEPS_HASH_PER_WORD.saturating_mul(words))
        }
        _ => 0, // modexp trapped earlier (vm.rs); any other address is not a precompile
    }
}

#[cfg(test)]
mod step_price_tests {
    use super::*;

    #[test]
    fn pairing_price_is_linear_in_pair_count() {
        let price = |pairs: usize| step_price(&ecpairing::Ecpairing::ADDRESS, &vec![0_u8; pairs * 192]);
        for pairs in [1_usize, 4, 100] {
            assert_eq!(
                price(pairs),
                PRECOMPILE_STEPS_PAIRING_BASE + PRECOMPILE_STEPS_PAIRING_PER_PAIR * pairs as u64,
            );
        }
        // baseline (pre-fix): a CALL of this shape adds 0 to any counter —
        // there was no counter. Priced now: strictly increasing in pair count.
        assert!(price(4) > price(1));
        assert!(price(100) > price(4));
    }

    #[test]
    fn blake2f_price_is_linear_in_rounds() {
        for rounds in [12_u32, 1_000, 1_000_000] {
            let mut input = vec![0_u8; 213];
            input[0..4].copy_from_slice(&rounds.to_be_bytes());
            assert_eq!(
                step_price(&blake2f::Blake2f::ADDRESS, &input),
                PRECOMPILE_STEPS_BLAKE2F_BASE + PRECOMPILE_STEPS_BLAKE2F_PER_ROUND * rounds as u64,
            );
        }
    }

    #[test]
    fn fixed_precompiles_price_a_normal_small_call_at_their_constant() {
        // "a normal small precompile call is unaffected" (green_must_keep) —
        // fixed-cost precompiles price identically regardless of input size.
        assert_eq!(step_price(&ecmul::Ecmul::ADDRESS, &[0_u8; 96]), PRECOMPILE_STEPS_ECMUL);
        assert_eq!(step_price(&ecadd::Ecadd::ADDRESS, &[0_u8; 128]), PRECOMPILE_STEPS_ECADD);
        assert_eq!(step_price(&ecrecover::Ecrecover::ADDRESS, &[0_u8; 128]), PRECOMPILE_STEPS_ECRECOVER);
    }

    #[test]
    fn hash_family_price_scales_with_word_count() {
        for (addr, len) in [
            (sha2_256::Sha2::ADDRESS, 32_usize),
            (ripemd_160::Ripemd::ADDRESS, 1024),
            (identity::Identity::ADDRESS, 4096),
        ] {
            let words = (len as u64).div_ceil(32);
            assert_eq!(
                step_price(&addr, &vec![0_u8; len]),
                PRECOMPILE_STEPS_HASH_BASE + PRECOMPILE_STEPS_HASH_PER_WORD * words,
            );
        }
    }

    #[test]
    fn a_64mib_pairing_input_exceeds_the_off_chain_budget_on_its_own() {
        // RED-031c (single oversized call must not run): the specific attack
        // size named in the finding — a 64MiB / ~349k-pair ecPairing input —
        // must price ABOVE MAX_OPCODES_PER_EMULATION by itself, so trap_call
        // can refuse it before the syscall runs. Regression guard on the
        // chosen constants: if they're ever lowered, this test catches that
        // the named attack is no longer bounded. Uses the formula directly
        // (not a real 64MiB alloc) — length is all step_price reads.
        const PAIRS: u64 = 349_000;
        let price = PRECOMPILE_STEPS_PAIRING_BASE + PRECOMPILE_STEPS_PAIRING_PER_PAIR * PAIRS;
        assert!(price > crate::config::MAX_OPCODES_PER_EMULATION,
            "pairing price for {PAIRS} pairs ({price}) must exceed the budget on its own");
    }

    #[test]
    fn a_max_rounds_blake2f_loop_is_bounded_in_aggregate() {
        // RED-031d shape: MAX_ROUNDS-sized blake2f calls must NOT each exceed
        // the budget alone (so the loop runs several before tripping) but the
        // aggregate over enough calls must exceed it.
        const MAX_ROUNDS: u64 = 1_000_000;
        let per_call = PRECOMPILE_STEPS_BLAKE2F_BASE + PRECOMPILE_STEPS_BLAKE2F_PER_ROUND * MAX_ROUNDS;
        assert!(per_call < crate::config::MAX_OPCODES_PER_EMULATION,
            "a single MAX_ROUNDS call must fit under the budget so the loop is bounded, not instantly rejected");
        let calls_to_exhaust = crate::config::MAX_OPCODES_PER_EMULATION / per_call;
        assert!(calls_to_exhaust < 100, "loop must be bounded well before 100 calls, got {calls_to_exhaust}");
    }
}
