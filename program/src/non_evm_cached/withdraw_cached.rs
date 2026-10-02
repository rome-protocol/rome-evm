use {
    crate::{
        error::{Result, RomeProgramError::*},
        non_evm::{
            not_payable, Program, NonEvmCall,
        },
        non_evm_cached::{WithdrawCachedReader, ASplCached, SplCached, SystemCached,},
        origin::Origin, handler::CallInterrupt, NonEvmState, H160,
    },
    solana_program::instruction::Instruction,
    solana_system_interface::program as system_program,
};

// withdrawal(bytes32)              0x4d8b0ea4          payable
// withdraw_to_pda(uint256)         0x7f3124a0
// withdraw_to_ata(uint256)         0x8059abc0
// deposit(uint256)                 0xb6b55f25          inverse of withdrawal (SPL ATA → native gas mint)
const WITHDRAWAL_ID: &[u8] = &[0x4d, 0x8b, 0x0e, 0xa4];
const WITHDRAW_TO_PDA: &[u8] = &[0x7f, 0x31, 0x24, 0xa0];
const WITHDRAW_TO_ATA: &[u8] = &[0x80, 0x59, 0xab, 0xc0];
const DEPOSIT: &[u8] = &[0xb6, 0xb5, 0x5f, 0x25];

pub struct WithdrawCached<'a, T: Origin> {
    pub state: &'a T,
}
impl<'a, T: Origin> WithdrawCached<'a, T> {
    pub const ADDRESS: H160 = H160([
        0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0b,

    ]);
    pub fn new(state: &'a T) -> Self {
        Self {
            state
        }
    }
}
impl<'a, T: Origin> Program for WithdrawCached<'a, T> {
    fn from_abi<'b>(&self, _: &'b CallInterrupt) -> Result<NonEvmCall<'b>> {
        unreachable!()
    }
    fn from_abi_cached<'c>(&self, params: &'c CallInterrupt, nes: Option<&NonEvmState>) -> Result<NonEvmCall<'c>> {
        let (a, b) = params.input.split_at(4);
        let reader = WithdrawCachedReader::new(self.state, nes);

        match a {
            WITHDRAWAL_ID => {
                let list = reader.withdraw(b, params)?;
                Ok(NonEvmCall::Composed(list))
            },
            WITHDRAW_TO_PDA => {
                not_payable(params)?;
                let list = reader.withdraw_to_pda(b, params)?;
                Ok(NonEvmCall::Composed(list))
            }
            WITHDRAW_TO_ATA => {
                not_payable(params)?;
                let list = reader.withdraw_to_ata(b, params)?;
                Ok(NonEvmCall::Composed(list))
            }
            DEPOSIT => {
                not_payable(params)?;
                let list = reader.deposit(b, params)?;
                Ok(NonEvmCall::Composed(list))
            }
            _ => Err(Unimplemented(format!("method is not supported by CachedWithdrawProgram {}", hex::encode(a))))
        }
    }
    fn emulate(&self, ix: &Instruction, nes: &mut NonEvmState) -> Result<()>  {
        match ix.program_id {
            spl_associated_token_account_interface::program::ID =>
                ASplCached::new(self.state).emulate(ix, nes),
            system_program::ID =>
                SystemCached::new(self.state).emulate(ix, nes),
            spl_token_interface::ID | spl_token_2022_interface::ID =>
                SplCached::new(self.state).emulate(ix, nes),
            _ => unreachable!()
        }
    }
    fn eth_call(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
        unreachable!()
    }
    fn cross_state_call(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
        unreachable!()
    }
    fn precompile(&self) -> bool {
        false
    }
    fn cached(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::keccak::hashv;

    fn selector(sig: &str) -> [u8; 4] {
        let h = hashv(&[sig.as_bytes()]).to_bytes();
        let mut s = [0u8; 4];
        s.copy_from_slice(&h[..4]);
        s
    }

    #[test]
    fn deposit_selector_hex_lock() {
        assert_eq!(
            DEPOSIT,
            selector("deposit(uint256)").as_slice(),
        );
    }
}


