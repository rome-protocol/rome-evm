use {
    crate::{
        error::Result, origin::Origin,
        error::RomeProgramError::*, handler::CallInterrupt,
    },
    super::{
        Program, helper_ix::HelperProgram, NonEvmCall,
        aux::{
            not_payable,
        },
    },
};

// create_ata(address)          0x5a7c3259
const CREATE_ATA: &[u8] = &[0x5a, 0x7c, 0x32, 0x59];

// create_ata(address,bytes32)  0x3de2251a
const CREATE_ATA_WITH_MINT: &[u8] = &[0x3d, 0xe2, 0x25, 0x1a];

// create_ata_for_key(bytes32,bytes32)   0xd258a69d  (idempotent ATA-create for raw-pubkey owner)
const CREATE_ATA_FOR_KEY: &[u8] = &[0xd2, 0x58, 0xa6, 0x9d];

// approve_spl_raw_delegate(bytes32,bytes32,uint64,bytes32,uint8)   0x7881d453
// SPL approve_checked with caller-supplied raw-pubkey delegate (e.g. Wormhole
// authorityAuthoritySigner). Caller passes decimals to skip on-chain mint read.
// Hardcoded spl_program = SPL Token; Token-2022 raw-delegate is a future selector.
const APPROVE_SPL_RAW_DELEGATE: &[u8] = &[0x78, 0x81, 0xd4, 0x53];

// create_mint_account(bytes32)   0xe97d3291
// System CreateAccount for salt-derived PDA: space=82 (SPL_MINT_LEN), owner=SPL Token,
// lamports=rent-floor. Funder = caller's external_auth PDA. New account = external_auth_with_salt.
const CREATE_MINT_ACCOUNT: &[u8] = &[0xe9, 0x7d, 0x32, 0x91];

// init_spl_mint(bytes32,uint8,bytes32,bool,bytes32)   0x4f75e987
// SPL InitializeMint2 against pre-allocated mint. No signer needed (mint_authority + freeze_authority
// stored as account data; SPL Token runtime only requires mint to be writable).
const INIT_SPL_MINT: &[u8] = &[0x4f, 0x75, 0xe9, 0x87];

// create_and_init_mint(uint8,bytes32,bool,bytes32,bytes32)   0x20972d0f
// Composed: System CreateAccount + SPL InitializeMint2 in one dispatch. Saves 1 Rome DoTx overhead
// vs separate A4 + A5 calls.
const CREATE_AND_INIT_MINT: &[u8] = &[0x20, 0x97, 0x2d, 0x0f];

// create_pda(address)          0xff3556ca
const CREATE_PDA: &[u8] = &[0xff, 0x35, 0x56, 0xca];

// create_pda(address,uint64)   0x58e88298
const CREATE_PDA_WITH_LAMPORTS: &[u8] = &[0x58, 0xe8, 0x82, 0x98];

// swap_gas_to_lamports(uint64)    0x6e3f24e0
const SWAP_GAS_TO_LAMPORTS: &[u8] = &[0x6e, 0x3f, 0x24, 0xe0];

// transfer_lamports(address,uint64)    0x5fe71665
const TRANSFER_LAMPORTS: &[u8] = &[0x5f, 0xe7, 0x16, 0x65];

// transfer_spl(address,uint64)    0xb12be5ba
const TRANSFER_SPL: &[u8] = &[0xb1, 0x2b, 0xe5, 0xba];

// transfer_spl(bytes32,uint64)    0xba3a5eac
const TRANSFER_SPL_TO_ATA: &[u8] = &[0xba, 0x3a, 0x5e, 0xac];

// transfer_spl(address,uint64,bytes32)    0x53b505e0
const TRANSFER_SPL_WITH_MINT: &[u8] = &[0x53, 0xb5, 0x05, 0xe0];

// transfer_spl(bytes32,uint64,bytes32)    0xb6977879
const TRANSFER_SPL_WITH_MINT_TO_ATA: &[u8] = &[0xb6, 0x97, 0x78, 0x79];

// transfer_spl(bytes32,bytes32,uint64,bytes32)    0x766b362a
// Delegate variant: src_ata is caller-supplied (vs. derived from caller's
// external_auth PDA in the other transfer_spl overloads). Signs as
// `external_auth(caller)`; SPL Token Program enforces that this PDA is
// either the source ATA's owner OR its delegate with delegated_amount ≥
// tokens.
const TRANSFER_SPL_FROM_ATA: &[u8] = &[0x76, 0x6b, 0x36, 0x2a];

// pda(address)     0x8854a299
const PDA: &[u8] = &[0x88, 0x54, 0xa2, 0x99];
// pda_with_salt(address,bytes32) — read-only, salt-derived EXTERNAL_AUTHORITY PDA
const PDA_WITH_SALT: &[u8] = &[0x5c, 0x6d, 0x04, 0xb3];

// ata(address)     0x31db4f82
const ATA: &[u8] = &[0x31, 0xdb, 0x4f, 0x82];

// ata(address,bytes32)     0xfeb1c647
const ATA_WITH_MINT: &[u8] = &[0xfe, 0xb1, 0xc6, 0x47];

// deposit_from_ata(uint256)    0x4479b709
const DEPOSIT_FROM_ATA: &[u8] = &[0x44, 0x79, 0xb7, 0x09];

// --- SPL_ERC20 direct-precompile rewrite selectors -----------------
// Precompile-surface reference: docs/PRECOMPILES.md.
// Sigs locked against `cast keccak`; tests at the bottom assert each
// const == `keccak256(canonical_sig)[..4]`.

// approve_spl(address,uint64,bytes32)             0xabf6f675
const APPROVE_SPL: &[u8] = &[0xab, 0xf6, 0xf6, 0x75];

// mint_spl(address,uint64,bytes32)                0xd795522b
const MINT_SPL: &[u8] = &[0xd7, 0x95, 0x52, 0x2b];

// user_balance(address,bytes32)                   0xdd0119c8
const USER_BALANCE: &[u8] = &[0xdd, 0x01, 0x19, 0xc8];

// allowance_of(address,address,bytes32)           0xed72dbc8
const ALLOWANCE_OF: &[u8] = &[0xed, 0x72, 0xdb, 0xc8];

// mint_info(bytes32)                              0xe24bf5d4
const MINT_INFO: &[u8] = &[0xe2, 0x4b, 0xf5, 0xd4];

// transfer_spl(address,address,uint64,bytes32)             0xe479df56
// Delegate variant — both endpoints by EVM address. Distinct from
// existing TRANSFER_SPL_FROM_ATA (0x766b362a) which takes bytes32 ATAs.
const TRANSFER_SPL_FROM_TO: &[u8] = &[0xe4, 0x79, 0xdf, 0x56];


// REMOVED 2026-05-16 (PR-1): the 3 Token-2022 4-arg variants
//   approve_spl(address,uint64,bytes32,bytes32)             0xc9884b1e
//   mint_spl(address,uint64,bytes32,bytes32)                0x406ee21b
//   transfer_spl(address,address,uint64,bytes32,bytes32)    0x7b11c48f
// were broken-as-advertised: their impls still called mint_owner_decimals,
// defeating the explicit token_program argument. Zero callers across the
// monorepo. Token-2022 raw-delegate flows will get fresh selectors when
// the use case emerges.


// transfer_spl_to_signer(uint64,bytes32)    0x46efa679             transfer_spl_to_signer(amount,mint)
pub const TRANSFER_SPL_TO_SIGNER: &[u8] = &[0x46, 0xef, 0xa6, 0x79];

impl<'a, T: Origin> Program for HelperProgram<'a, T> {
    fn from_abi<'b>(&self, params: &'b CallInterrupt) -> Result<NonEvmCall<'b>> {
        not_payable(params)?;
        let (a, b) = params.input.split_at(4);
        let call = match a {
            CREATE_ATA => {
                let (ix, seed) = self.create_ata(b)?;
                NonEvmCall::Invoke(ix, seed)
            },
            CREATE_ATA_WITH_MINT => {
                let (ix, seed) = self.create_ata_with_mint(b)?;
                NonEvmCall::Invoke(ix, seed)
            },
            CREATE_ATA_FOR_KEY => {
                let (ix, seed) = self.create_ata_for_key(b)?;
                NonEvmCall::Invoke(ix, seed)
            },
            CREATE_PDA => {
                let (ix, seed) = self.create_pda(b)?;
                NonEvmCall::Invoke(ix, seed)
            },
            CREATE_PDA_WITH_LAMPORTS => {
                let (ix, seed) = self.create_pda_with_lamports(b)?;
                NonEvmCall::Invoke(ix, seed)
            },
            SWAP_GAS_TO_LAMPORTS => {
                let (ix, seed) = self.swap_gas_to_lamports(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            },
            TRANSFER_LAMPORTS => {
                let (ix, seed) = self.transfer_lamports(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            }
            TRANSFER_SPL => {
                let (ix, seed) = self.transfer_spl(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            }
            TRANSFER_SPL_TO_ATA => {
                let (ix, seed) = self.transfer_spl_to_ata(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            }
            TRANSFER_SPL_WITH_MINT => {
                let (ix, seed) = self.transfer_spl_with_mint(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            }
            TRANSFER_SPL_WITH_MINT_TO_ATA => {
                let (ix, seed) = self.transfer_spl_with_mint_to_ata(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            }
            TRANSFER_SPL_FROM_ATA => {
                let (ix, seed) = self.transfer_spl_from_ata(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            }
            APPROVE_SPL => {
                let (ix, seed) = self.approve_spl(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            }
            APPROVE_SPL_RAW_DELEGATE => {
                let (ix, seed) = self.approve_spl_raw_delegate(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            }
            CREATE_MINT_ACCOUNT => {
                let (ix, seed) = self.create_mint_account(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            }
            INIT_SPL_MINT => {
                let (ix, seed) = self.init_spl_mint(b)?;
                NonEvmCall::Invoke(ix, seed)
            }
            CREATE_AND_INIT_MINT => {
                let list = self.create_and_init_mint(b, params)?;
                NonEvmCall::Composed(list)
            }
            MINT_SPL => {
                let (ix, seed) = self.mint_spl(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            }
            TRANSFER_SPL_FROM_TO => {
                let (ix, seed) = self.transfer_spl_from_to(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            }
            TRANSFER_SPL_TO_SIGNER => {
                let (ix, seed) = self.transfer_spl_to_signer(b, params)?;
                NonEvmCall::Invoke(ix, seed)
            }
            DEPOSIT_FROM_ATA => {
                let list = self.deposit_from_ata(b, params)?;
                NonEvmCall::Composed(list)
            }
            PDA | PDA_WITH_SALT | ATA | ATA_WITH_MINT => NonEvmCall::EthCall(a, b),
            USER_BALANCE | ALLOWANCE_OF | MINT_INFO => NonEvmCall::CrossStateEthCall(a, b),
            _ => return Err(Unimplemented(format!("method is not supported by HelperProgram 0x{}", hex::encode(a))))
        };

        Ok(call)
    }
    fn eth_call(&self, a: &[u8], b: &[u8]) -> Result<Vec<u8>> {
        match a {
            PDA => self.pda(b),
            PDA_WITH_SALT => self.pda_with_salt(b),
            ATA => self.ata(b),
            ATA_WITH_MINT => self.ata_with_mint(b),
            _ => Err(Unimplemented(format!("eth_call is not supported by HelperProgram 0x{}", hex::encode(a))))
        }
    }
    fn cross_state_call(&self, a: &[u8], b: &[u8]) -> Result<Vec<u8>> {
        match a {
            USER_BALANCE => self.user_balance(b),
            ALLOWANCE_OF => self.allowance_of(b),
            MINT_INFO => self.mint_info(b),
            _ => Err(Unimplemented(format!("cross_state_call is not supported by HelperProgram 0x{}", hex::encode(a))))
        }
    }
    fn precompile(&self) -> bool {
        false
    }
    // Creates and the lamport top-up. Three of these do return a bare
    // external_auth seed, so read the reason each is safe rather than assuming
    // the boundary covers it:
    //  - create_ata*, create_pda*: the target address comes from the ABI with no
    //    caller binding and the operator funds it, so a delegatecall frame gains
    //    nothing a direct call could not already do.
    //  - init_spl_mint: no signer at all.
    //  - swap_gas_to_lamports: no seed; operator credits the caller's own PDA.
    //  - create_and_init_mint: funds from external_auth(context.caller), so it
    //    requires both authorities written into the mint to BE that same PDA. A
    //    delegatecall frame can still make its caller pay the rent, but the mint
    //    it creates is the caller's own and it gains no authority over it.
    fn delegatecall_exempt(&self, selector: &[u8]) -> bool {
        [
            CREATE_ATA,
            CREATE_ATA_WITH_MINT,
            CREATE_ATA_FOR_KEY,
            CREATE_PDA,
            CREATE_PDA_WITH_LAMPORTS,
            INIT_SPL_MINT,
            CREATE_AND_INIT_MINT,
            SWAP_GAS_TO_LAMPORTS,
        ]
        .contains(&selector)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::keccak;

    fn selector(sig: &str) -> [u8; 4] {
        let h = keccak::hash(sig.as_bytes()).to_bytes();
        [h[0], h[1], h[2], h[3]]
    }

    // Selector locks for the SPL_ERC20 direct-precompile rewrite.
    // Each const must equal `keccak256(canonical_sig)[..4]`; a typo
    // here is a silent dispatch failure at runtime. Keep these selector
    // locks in sync with the dispatch table above.

    #[test]
    fn approve_spl_selectors_locked() {
        assert_eq!(selector("approve_spl(address,uint64,bytes32)"), [0xab, 0xf6, 0xf6, 0x75]);
        assert_eq!(APPROVE_SPL, &selector("approve_spl(address,uint64,bytes32)")[..]);
    }

    #[test]
    fn mint_spl_selectors_locked() {
        assert_eq!(selector("mint_spl(address,uint64,bytes32)"), [0xd7, 0x95, 0x52, 0x2b]);
        assert_eq!(MINT_SPL, &selector("mint_spl(address,uint64,bytes32)")[..]);
    }

    #[test]
    fn read_selectors_locked() {
        assert_eq!(selector("user_balance(address,bytes32)"),         [0xdd, 0x01, 0x19, 0xc8]);
        assert_eq!(USER_BALANCE, &selector("user_balance(address,bytes32)")[..]);

        assert_eq!(selector("allowance_of(address,address,bytes32)"), [0xed, 0x72, 0xdb, 0xc8]);
        assert_eq!(ALLOWANCE_OF, &selector("allowance_of(address,address,bytes32)")[..]);
        assert_eq!(MINT_INFO, &selector("mint_info(bytes32)")[..]);
    }

    #[test]
    fn transfer_spl_addr_to_addr_selectors_locked() {
        assert_eq!(selector("transfer_spl(address,address,uint64,bytes32)"), [0xe4, 0x79, 0xdf, 0x56]);
        assert_eq!(TRANSFER_SPL_FROM_TO, &selector("transfer_spl(address,address,uint64,bytes32)")[..]);
    }

    #[test]
    fn transfer_spl_to_signer_selector_locked() {
        assert_eq!(selector("transfer_spl_to_signer(uint64,bytes32)"), [0x46, 0xef, 0xa6, 0x79]);
        assert_eq!(TRANSFER_SPL_TO_SIGNER, &selector("transfer_spl_to_signer(uint64,bytes32)")[..]);
    }

    // Selector locks for delegation to Rome's known programs.
    // A mismatch here means a call would silently miss its dispatch
    // arm, so each selector is pinned explicitly.

    #[test]
    fn pda_with_salt_selector_locked() {
        assert_eq!(selector("pda_with_salt(address,bytes32)"), [0x5c, 0x6d, 0x04, 0xb3]);
        assert_eq!(PDA_WITH_SALT, &selector("pda_with_salt(address,bytes32)")[..]);
    }

    #[test]
    fn create_ata_for_key_selector_locked() {
        assert_eq!(selector("create_ata_for_key(bytes32,bytes32)"), [0xd2, 0x58, 0xa6, 0x9d]);
        assert_eq!(CREATE_ATA_FOR_KEY, &selector("create_ata_for_key(bytes32,bytes32)")[..]);
    }

    #[test]
    fn approve_spl_raw_delegate_selector_locked() {
        assert_eq!(
            selector("approve_spl_raw_delegate(bytes32,bytes32,uint64,bytes32,uint8)"),
            [0x78, 0x81, 0xd4, 0x53]
        );
        assert_eq!(
            APPROVE_SPL_RAW_DELEGATE,
            &selector("approve_spl_raw_delegate(bytes32,bytes32,uint64,bytes32,uint8)")[..]
        );
    }

    #[test]
    fn create_mint_account_selector_locked() {
        assert_eq!(selector("create_mint_account(bytes32)"), [0xe9, 0x7d, 0x32, 0x91]);
        assert_eq!(CREATE_MINT_ACCOUNT, &selector("create_mint_account(bytes32)")[..]);
    }

    #[test]
    fn init_spl_mint_selector_locked() {
        assert_eq!(
            selector("init_spl_mint(bytes32,uint8,bytes32,bool,bytes32)"),
            [0x4f, 0x75, 0xe9, 0x87]
        );
        assert_eq!(
            INIT_SPL_MINT,
            &selector("init_spl_mint(bytes32,uint8,bytes32,bool,bytes32)")[..]
        );
    }

    #[test]
    fn create_and_init_mint_selector_locked() {
        assert_eq!(
            selector("create_and_init_mint(uint8,bytes32,bool,bytes32,bytes32)"),
            [0x20, 0x97, 0x2d, 0x0f]
        );
        assert_eq!(
            CREATE_AND_INIT_MINT,
            &selector("create_and_init_mint(uint8,bytes32,bool,bytes32,bytes32)")[..]
        );
    }
}
