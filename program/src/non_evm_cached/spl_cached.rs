use {
    solana_program::instruction::Instruction,
    crate::{
        origin::Origin, error::{Result, RomeProgramError::*,}, handler::CallInterrupt,
        non_evm::{
            Program, aux::*, NonEvmCall,
        },
        state::NonEvmState, H160,
    },
    spl_token_2022_interface::{
        instruction::TokenInstruction::{self, TransferChecked, InitializeAccount3, ApproveChecked, MintToChecked},
    },
    super::spl_cached_ix::{SplCachedReader, SplCachedWriter, },
};
//      invokes
//  transfer(address,uint256)                                       //  0xa9059cbb      transfer(to,amount)
//  transfer(bytes32,uint256)                                       //  0x6a467394      transfer(to_pda,amount)
//  transfer(address,uint256,bytes32)                               //  0x57cfeeee      transfer(to,amount,mint)
//  transfer(bytes32,uint256,bytes32)                               //  0x7db527f9      transfer(to_pda,amount,mint)
//  transferFrom(address,address,uint256,bytes32)                   //  0x401e3367      transferFrom(from,to,amount,mint)
//  approve(address,uint256,bytes32)                                //  0x8180f2fc      approve(spender,amount,mint)
//  mint(address,uint256,bytes32)                                   //  0x1e458bee      mint(to,amount,mint)

//  init(bytes32,bytes32,bytes32)                                   //  0x0b0ad508      init(ata,mint,owner)

//      cross-state eth-calls
//  account(address)                                                //  0x73b9aa91      account(msg.sender)
//  account(bytes32)                                                //  0x882358ae      account(ata)

const TRANSFER: &[u8] = &[0xa9, 0x05, 0x9c, 0xbb];
const TRANSFER_B32: &[u8] = &[0x6a, 0x46, 0x73, 0x94];
const TRANSFER_MINT: &[u8] = &[0x57, 0xcf, 0xee, 0xee];
const TRANSFER_MINT_B32: &[u8] = &[0x7d, 0xb5, 0x27, 0xf9];
const TRANSFER_FROM: &[u8] = &[0x40, 0x1e, 0x33, 0x67];
const APPROVE: &[u8] = &[0x81, 0x80, 0xf2, 0xfc];
const MINT: &[u8] = &[0x1e, 0x45, 0x8b, 0xee];
const INIT: &[u8] = &[0x0b, 0x0a, 0xd5, 0x08];
const ACCOUNT: &[u8] = &[0x73, 0xb9, 0xaa, 0x91];
const ACCOUNT_B32: &[u8] = &[0x88, 0x23, 0x58, 0xae];
const ACCOUNT_MINT: &[u8] = &[0xf9, 0x82, 0x72, 0x27];

// mint_info(bytes32)                    0xe24bf5d4
const MINT_INFO: &[u8] = &[0xe2, 0x4b, 0xf5, 0xd4];

pub struct SplCached<'a, T: Origin> {
    pub state: &'a T,
}

impl<'a, T: Origin> SplCached<'a, T> {
    pub const ADDRESS: H160 = H160([
        0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x05,
    ]);
    pub fn new(state: &'a T) -> Self {
        Self {
            state
        }
    }
}

impl <'a, T: Origin>Program for SplCached<'a, T> {
    fn from_abi<'b>(&self, _: &'b CallInterrupt) -> Result<NonEvmCall<'b>> {
        unreachable!()
    }
    fn from_abi_cached<'c>(&self, params: &'c CallInterrupt, nes: Option<&NonEvmState>) -> Result<NonEvmCall<'c>> {
        not_payable(params)?;
        let (a, b) = params.input.split_at(4);
        let reader = SplCachedReader::new(self.state, nes);

        let call = match a {
            TRANSFER => {
                let (ix, seeds) = reader.transfer(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            TRANSFER_B32 => {
                let (ix, seeds) = reader.transfer_b32(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            TRANSFER_MINT => {
                let (ix, seeds) = reader.transfer_mint(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            TRANSFER_MINT_B32 => {
                let (ix, seeds) = reader.transfer_mint_b32(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            TRANSFER_FROM => {
                let (ix, seeds) = reader.transfer_from(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            APPROVE => {
                let (ix, seeds) = reader.approve(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            MINT => {
                let (ix, seeds) = reader.mint(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            INIT => {
                let (ix, seeds) = reader.init(b)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            ACCOUNT => NonEvmCall::CrossStateEthCall(a, b),
            ACCOUNT_B32 => NonEvmCall::CrossStateEthCall(a, b),
            ACCOUNT_MINT => NonEvmCall::CrossStateEthCall(a, b),
            MINT_INFO => NonEvmCall::CrossStateEthCall(a, b),
            _ => return Err(Unimplemented(format!("method is not supported by SplCached {}", hex::encode(a))))
        };

        Ok(call)
    }
    fn emulate(&self, ix: &Instruction, nes: &mut NonEvmState) -> Result<()>  {
        let mut writer = SplCachedWriter::new(self.state, nes);

        match TokenInstruction::unpack(&ix.data)? {
            TransferChecked { amount, decimals } => {
                match ix.program_id {
                    spl_token_interface::ID =>
                        writer.transfer_checked(ix, amount, decimals),
                    spl_token_2022_interface::ID =>
                        writer.transfer2022_checked(ix,),
                    _ => Err(Unimplemented(format!("invalid spl_program id {}", ix.program_id))),
                }
            }
            ApproveChecked { amount, decimals } => {
                match ix.program_id {
                    spl_token_interface::ID =>
                        writer.approve_checked(ix, amount, decimals),
                    spl_token_2022_interface::ID =>
                        writer.approve2022_checked(ix,),
                    _ => Err(Unimplemented(format!("invalid spl_program id {}", ix.program_id))),
                }
            }
            MintToChecked { amount, decimals } => {
                match ix.program_id {
                    spl_token_interface::ID =>
                        writer.mint_to_checked(ix, amount, decimals),
                    spl_token_2022_interface::ID =>
                        writer.mint_to2022_checked(ix,),
                    _ => Err(Unimplemented(format!("invalid spl_program id {}", ix.program_id))),
                }
            }
            InitializeAccount3 { owner } => {
                match ix.program_id {
                    spl_token_interface::ID =>
                        writer.init(ix, owner),
                    spl_token_2022_interface::ID =>
                        writer.init2022(ix,&owner),
                    _ => Err(Unimplemented(format!("invalid spl_program id {}", ix.program_id))),
                }
            },
            TokenInstruction::InitializeImmutableOwner => {
                match ix.program_id {
                    spl_token_2022_interface::ID =>
                        writer.init_immutable_owner2022(ix),
                    // Legacy accounts are exactly Account::LEN with no TLV
                    // region, so the ATA program creates no such extension and
                    // the overlay is never asked to stage one.
                    _ => Err(Unimplemented(format!("InitializeImmutableOwner is Token-2022 only, got {}", ix.program_id))),
                }
            },
            _ => Err(Unimplemented("instruction is not supported by SplCached".to_string())),
        }
    }
    fn eth_call(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
        unreachable!()
    }
    fn cross_state_call(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
        unreachable!()
    }
    fn cross_state_call_cached(&self, a: &[u8], b: &[u8], nes: Option<&NonEvmState>) -> Result<Vec<u8>> {
        let reader = SplCachedReader::new(self.state, nes);
        match a {
            ACCOUNT => reader.account(b),
            ACCOUNT_B32 => reader.account_b32(b),
            ACCOUNT_MINT => reader.account_mint(b),
            MINT_INFO => reader.mint_info(b),
            _ => unreachable!()
        }
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
    fn transfer_from_selector_hex_lock() {
        assert_eq!(
            TRANSFER_FROM,
            selector("transferFrom(address,address,uint256,bytes32)").as_slice(),
        );
    }

    #[test]
    fn approve_selector_hex_lock() {
        assert_eq!(
            APPROVE,
            selector("approve(address,uint256,bytes32)").as_slice(),
        );
    }

    #[test]
    fn mint_selector_hex_lock() {
        assert_eq!(
            MINT,
            selector("mint(address,uint256,bytes32)").as_slice(),
        );
    }

    #[test]
    fn mint_info_selector_hex_lock() {
        assert_eq!(MINT_INFO, selector("mint_info(bytes32)").as_slice());
    }

    #[test]
    fn mint_info_selector_matches_the_legacy_home() {
        // One capability, two dispatch homes: the selector must be identical so
        // a contract writes the same call whichever track it is on.
        assert_eq!(MINT_INFO, &[0xe2, 0x4b, 0xf5, 0xd4]);
    }

    #[test]
    fn account_mint_selector_hex_lock() {
        assert_eq!(
            ACCOUNT_MINT,
            selector("account(address,bytes32)").as_slice(),
        );
    }
}
