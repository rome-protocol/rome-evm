pub mod system;
pub mod system_ix;
pub mod aux;
mod withdraw;
mod cpi;
pub mod cpi_ix;
// CU-shortcut precompiles — see docs/CPI_PRECOMPILE_SHORTCUTS.md (v1)
//                          + docs/CPI_PRECOMPILE_SHORTCUTS_V2.md (v2)
pub mod account_data;          // v1: account_data_at (generic Solana slice reader)
pub mod account_data_extras;   // v2: account_u64_at + account_lamports
pub mod derive_helpers;
mod helper;
pub mod helper_ix;
mod withdraw_ix;


pub use {
    system_ix::System,
    withdraw_ix::Withdraw,
    aux::*,
    cpi_ix::CpiProgram,
    helper_ix::HelperProgram,
};

use {
    crate::{
        error::Result, state::pda::Seed, Diff, H160, state::handler::CallInterrupt,
        state::NonEvmState,
    },
    solana_program::{instruction::Instruction, },
};

pub enum NonEvmCall<'a> {
    Precompiled(&'a[u8]),
    EthCall(&'a[u8], &'a[u8]),
    CrossStateEthCall(&'a[u8], &'a[u8]),
    Invoke(Instruction, Vec<Seed>),
    Composed(IxList),
}

pub struct IxList {
    pub ixs: Vec<(Instruction, Vec<Seed>)>,
    pub diff: Vec<(H160, Diff)>,
}

pub trait Program {
    fn from_abi<'a>(&self, params: &'a CallInterrupt) -> Result<NonEvmCall<'a>>;
    fn from_abi_cached<'a>(&self, _: &'a CallInterrupt, _: Option<&NonEvmState>) -> Result<NonEvmCall<'a>> {
        unimplemented!()
    }
    fn eth_call(&self, func: &[u8],  input: &[u8]) -> Result<Vec<u8>>;
    fn cross_state_call<'a>(&self, func: &'a[u8], input: &'a[u8]) -> Result<Vec<u8>>;
    fn cross_state_call_cached<'a>(&self, _: &'a[u8], _: &'a[u8], _: Option<&NonEvmState>) -> Result<Vec<u8>> {
        unimplemented!()
    }
    fn precompile(&self) -> bool;
    // selectors safe to reach by DELEGATECALL/CALLCODE; default deny so a new signing selector is gated automatically
    fn delegatecall_exempt(&self, _selector: &[u8]) -> bool {
        false
    }
    fn emulate(&self, _ix: &Instruction, _new: &mut NonEvmState) -> Result<()> {
        unimplemented!()
    }
    fn cached(&self) -> bool {
        false
    }
}

