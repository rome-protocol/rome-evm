use {
    crate::{
        error::Result, origin::Origin,
        error::RomeProgramError::*, handler::CallInterrupt,
    },
    super::{
        Program,
        cpi_ix::CpiProgram, NonEvmCall, aux::not_payable,
    },
};
//      invokes
//  0x7480cb86      invoke(bytes32,(bytes32,bool,bool)[],bytes)
//  0xb94f3733      invoke_signed(bytes32,(bytes32,bool,bool)[],bytes,bytes32[])
//      cross-chain-eth-calls
//  0xc13465d9      account_info(bytes32)
//
//      v1 CU-shortcut precompiles — see docs/CPI_PRECOMPILE_SHORTCUTS.md:
//  0x593762e8      account_data_at(bytes32,uint16,uint16)             — generic read
//      v2 CU-shortcut precompiles — see docs/CPI_PRECOMPILE_SHORTCUTS_V2.md:
//  0xb317d4c1      account_u64_at(bytes32,uint16)                     — typed u64 read
//  0xde79ed54      account_lamports(bytes32)                          — lamports-only read
//  0x944336f8      pdas_batch_derive(bytes[][],bytes32)                — batched PDA derive
//
// derive_user_ata does not have a dispatch arm on this precompile — its
// capability moved to HelperProgram.ata(address,bytes32) (0xfeb1c647).

const ACCOUNT_INFO: &[u8] = &[0xc1, 0x34, 0x65, 0xd9];
const INVOKE: &[u8] = &[0x74, 0x80, 0xcb, 0x86];
const INVOKE_SIGNED: &[u8] = &[0xb9, 0x4f, 0x37, 0x33];

// Real keccak256(signature)[..4] selectors. Match the convention used by
// the existing precompiles above (ACCOUNT_INFO / INVOKE / INVOKE_SIGNED).
//   account_data_at(bytes32,uint16,uint16)              → 0x593762e8
//   (spl_transfer_checked_v1 / 0x351aa22f — REMOVED; no dispatch arm here,
//    superseded by HelperProgram; see the CpiProgram note in docs/PRECOMPILES.md)
const ACCOUNT_DATA_AT: &[u8]            = &[0x59, 0x37, 0x62, 0xe8];

// v2 selectors:
//   account_u64_at(bytes32,uint16)                      → 0xb317d4c1
//   account_lamports(bytes32)                            → 0xde79ed54
//   pdas_batch_derive(bytes[][],bytes32)                 → 0x944336f8
const ACCOUNT_U64_AT: &[u8]             = &[0xb3, 0x17, 0xd4, 0xc1];
const ACCOUNT_LAMPORTS: &[u8]           = &[0xde, 0x79, 0xed, 0x54];
const PDAS_BATCH_DERIVE: &[u8]          = &[0x94, 0x43, 0x36, 0xf8];

impl<'a, T: Origin> Program for CpiProgram<'a, T> {
    fn from_abi<'b>(&self, params: &'b CallInterrupt) -> Result<NonEvmCall<'b>> {
        not_payable(params)?;
        let (a, b) = params.input.split_at(4);
        let call = match a {
            ACCOUNT_INFO => NonEvmCall::CrossStateEthCall(a, b),
            INVOKE_SIGNED => {
                let (ix, seed) = self.invoke_signed_ix(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            },
            INVOKE => {
                let (ix, seed) = self.invoke_ix(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            },
            // v1 — read-side shortcut: pure cross-state query.
            ACCOUNT_DATA_AT => NonEvmCall::CrossStateEthCall(a, b),
            // v2 — typed read shortcuts (sugar over account_data_at)
            // + Rome ATA derivation + cardo-friendly batched PDA derive.
            // All four route through cross_state_call (no on-chain side
            // effects). See docs/CPI_PRECOMPILE_SHORTCUTS_V2.md for the
            // case-by-case justification.
            ACCOUNT_U64_AT
            | ACCOUNT_LAMPORTS
            | PDAS_BATCH_DERIVE => NonEvmCall::CrossStateEthCall(a, b),
            _ => return Err(Unimplemented(format!("method is not supported by CpiProgram 0x{}", hex::encode(a))))
        };

        Ok(call)
    }

    fn eth_call(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
        unimplemented!()
    }
    fn cross_state_call(&self, a: &[u8],b:&[u8]) -> Result<Vec<u8>> {
        match a {
            ACCOUNT_INFO       => self.account_info(b),
            ACCOUNT_DATA_AT    => super::account_data::account_data_at(self.state, b),
            ACCOUNT_U64_AT     => super::account_data_extras::account_u64_at(self.state, b),
            ACCOUNT_LAMPORTS   => super::account_data_extras::account_lamports(self.state, b),
            PDAS_BATCH_DERIVE  => super::derive_helpers::pdas_batch_derive(self.state, b),
            _ => unimplemented!()
        }
    }
    fn precompile(&self) -> bool {
        false
    }
    // invoke/invoke_signed are boundary-exempt because the refusal they need is
    // account-meta-dependent (the bare external_auth signer), not selector-dependent
    // — cpi_ix.rs enforces it internally (see invoke_ix/invoke_signed_ix). Every
    // other selector on this precompile is a pure cross-state read with no signer.
    fn delegatecall_exempt(&self, selector: &[u8]) -> bool {
        [INVOKE, INVOKE_SIGNED].contains(&selector)
    }
}
