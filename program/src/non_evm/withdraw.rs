use {
    crate::{
        error::{Result, RomeProgramError::*}, origin::Origin, handler::CallInterrupt, 
        non_evm::not_payable,
    },
    super::{
        Program, NonEvmCall, withdraw_ix::Withdraw,
    },
};

// withdrawal(bytes32)      0x4d8b0ea4      payable
const WITHDRAWAL_ID: &[u8] = &[0x4d, 0x8b, 0x0e, 0xa4];
// withdraw_to_pda(uint256) 0x7f3124a0
const WITHDRAW_TO_PDA: &[u8] = &[0x7f, 0x31, 0x24, 0xa0];
// withdraw_to_ata(uint256) 0x8059abc0
const WITHDRAW_TO_ATA: &[u8] = &[0x80, 0x59, 0xab, 0xc0];

impl<'a, T: Origin> Program for Withdraw<'a, T> {
    fn from_abi<'b>(&self, params: &'b CallInterrupt) -> Result<NonEvmCall<'b>> {
        let (a, b) = params.input.split_at(4);

        match a {
            WITHDRAWAL_ID => {
                let list = self.withdraw(b, params)?;
                Ok(NonEvmCall::Composed(list))
            },
            WITHDRAW_TO_PDA => {
                not_payable(params)?;
                let list = self.withdraw_to_pda(b, params)?;
                Ok(NonEvmCall::Composed(list))
            }
            WITHDRAW_TO_ATA => {
                not_payable(params)?;
                let list = self.withdraw_to_ata(b, params)?;
                Ok(NonEvmCall::Composed(list))
            }
            _ => Err(Unimplemented(format!("method is not supported by WithdrawProgram {}", hex::encode(a))))
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
}


