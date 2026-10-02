use {
    solana_program::instruction::Instruction,
    crate::{
        origin::Origin, error::{Result, RomeProgramError::*,}, handler::CallInterrupt,
        non_evm::{
            Program, aux::*, NonEvmCall,
        },
        state::NonEvmState, H160,
    },
    borsh::{BorshDeserialize,},
    super::aspl_cached_ix::{ASplCachedReader, ASplCachedWriter, },
    spl_associated_token_account_interface::instruction::{
        AssociatedTokenAccountInstruction as ATAI,
    },
};

//      invokes
//  create_ata()                        0xb6d336ed          create_ata()
//  create_ata(bytes32)                 0x81972e35          create_ata(mint)
//  create_ata(address)                 0x5a7c3259          create_ata(user)
//  create_ata(address,bytes32)         0x3de2251a          create_ata(user,mint)
const CREATE_ATA: &[u8] = &[0xb6, 0xd3, 0x36, 0xed];
const CREATE_ATA_MINT: &[u8] = &[0x81, 0x97, 0x2e, 0x35];
const CREATE_ATA_ADDRESS: &[u8] = &[0x5a, 0x7c, 0x32, 0x59];
const CREATE_ATA_ADDRESS_MINT: &[u8] = &[0x3d, 0xe2, 0x25, 0x1a];

pub struct ASplCached<'a, T: Origin> {
    pub state: &'a T,
}
impl<'a, T: Origin> ASplCached<'a, T> {
    pub const ADDRESS: H160 = H160([
        0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x06,
    ]);
    pub fn new(state: &'a T) -> Self {
        Self {
            state
        }
    }
}

impl<'a, T: Origin> Program for ASplCached<'a, T> {
    fn from_abi<'b>(&self, _: &'b CallInterrupt) -> Result<NonEvmCall<'b>> {
        unreachable!()
    }
    fn from_abi_cached<'b>(&self, params: &'b CallInterrupt, nes: Option<&NonEvmState>) -> Result<NonEvmCall<'b>> {
        not_payable(params)?;
        let (a, b) = params.input.split_at(4);
        let reader = ASplCachedReader::new(self.state, nes);

        let call = match a {
            CREATE_ATA => {
                let ix = reader.create_ata(params)?;
                NonEvmCall::Invoke(ix, vec![])
            },
            CREATE_ATA_MINT => {
                let ix = reader.create_ata_mint(b, params)?;
                NonEvmCall::Invoke(ix, vec![])
            },
            CREATE_ATA_ADDRESS => {
                let ix = reader.create_ata_address(b)?;
                NonEvmCall::Invoke(ix, vec![])
            },
            CREATE_ATA_ADDRESS_MINT => {
                let ix = reader.create_ata_address_mint(b)?;
                NonEvmCall::Invoke(ix, vec![])
            },
            _ => return Err(Unimplemented(format!("method is not supported by ASplProgram {}", hex::encode(a))))
        };

        Ok(call)
    }
    fn emulate(&self, ix: &Instruction, nes: &mut NonEvmState) -> Result<()>  {
        let writer = ASplCachedWriter::new(self.state, nes);
        
        match ATAI::try_from_slice(&ix.data)? {
            ATAI::CreateIdempotent => writer.create_idempotent(ix),
            _ => Err(Unimplemented("instruction is not supported by ASplProgram".to_string())),
        }
    }
    fn eth_call(&self, _func: &[u8], _: &[u8]) -> Result<Vec<u8>> {
        unimplemented!()
    }
    fn cross_state_call(&self, _: &[u8], _:&[u8]) -> Result<Vec<u8>> {
        unimplemented!()
    }
    fn precompile(&self) -> bool {
        false
    }
    // every create_ata leg is operator-funded and returns no seeds, so nothing signs as the caller
    fn delegatecall_exempt(&self, selector: &[u8]) -> bool {
        [
            CREATE_ATA,
            CREATE_ATA_MINT,
            CREATE_ATA_ADDRESS,
            CREATE_ATA_ADDRESS_MINT,
        ]
        .contains(&selector)
    }
    fn cached(&self) -> bool {
        true
    }
}
