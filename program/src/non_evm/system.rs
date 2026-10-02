use {
    crate::{
        error::{Result, RomeProgramError::*}, origin::Origin, handler::CallInterrupt,
    },
    super::{
        Program, System, NonEvmCall, aux::not_payable,
    },
};

//      eth_calls
//  0xb76fd45b       rome_evm_program_id()
//  0xfa2b1a5f       bytes32_to_base58(bytes32)
//  0x5df01b72       base58_to_bytes32(bytes)
//  0x27e3edda       find_program_address(bytes32,(bytes)[])
//  0x570ca735       operator()
//  0x77764881       program_id()
//  0xb99f29e1       create_program_address(bytes32,(bytes)[],uint8)
//  0xe132a122       mint_id()

const FIND_PDA_ID: &[u8] = &[0x27, 0xe3, 0xed, 0xda];
const ROME_EVM_PROGRAM_ID_ID: &[u8] = &[0xb7, 0x6f, 0xd4, 0x5b];
const BYTES32_TO_BASE58_ID: &[u8] = &[0xfa, 0x2b, 0x1a, 0x5f];
const BASE58_TO_BYTES32_ID: &[u8] = &[0x5d, 0xf0, 0x1b, 0x72];
const OPERATOR : &[u8] = &[0x57, 0x0c, 0xa7, 0x35];
const PROGRAM_ID_ID: &[u8] = &[0x77, 0x76, 0x48, 0x81];
const CREATE_PROGRAM_ADDRESS_ID: &[u8] = &[0xb9, 0x9f, 0x29, 0xe1];
const MINT_ID: &[u8] = &[0xe1, 0x32, 0xa1, 0x22];

impl<'a, T: Origin> Program for System<'a, T> {
    fn from_abi<'b>(&self, params: &'b CallInterrupt) ->Result<NonEvmCall<'b>> {
        not_payable(params)?;
        let (a, b) = params.input.split_at(4);

        let call = match a {
            FIND_PDA_ID
            | ROME_EVM_PROGRAM_ID_ID
            | BYTES32_TO_BASE58_ID
            | BASE58_TO_BYTES32_ID
            | OPERATOR
            | PROGRAM_ID_ID
            | CREATE_PROGRAM_ADDRESS_ID
            | MINT_ID => NonEvmCall::EthCall(a, b),
            _ =>  return Err(Unimplemented(format!("method is not supported by SystemProgram {}", hex::encode(a)))),
        };

        Ok(call)
    }
    fn eth_call(&self, a: &[u8], b: &[u8]) -> Result<Vec<u8>> {
        match a {
            FIND_PDA_ID => self.find_pda(b),
            ROME_EVM_PROGRAM_ID_ID => Ok(self.state.base().program_id.to_bytes().to_vec()),
            BYTES32_TO_BASE58_ID => System::<'a, T>::bytes32_to_base58(b),
            BASE58_TO_BYTES32_ID => System::<'a, T>::base58_to_bytes32(b),
            OPERATOR => Ok(self.state.signer().to_bytes().to_vec()),
            PROGRAM_ID_ID => Ok(System::<'a, T>::program_id()),
            CREATE_PROGRAM_ADDRESS_ID => System::<'a, T>::create_program_address(b),
            MINT_ID => self.mint_id(),
            _ => unreachable!()
        }
    }
    fn cross_state_call(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
        unimplemented!()
    }
    fn precompile(&self) -> bool {
        false
    }
}


