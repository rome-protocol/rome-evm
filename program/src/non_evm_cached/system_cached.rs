use {
    solana_program::instruction::Instruction,
    crate::{
        origin::Origin, error::{Result, RomeProgramError::*, }, handler::CallInterrupt,
        non_evm::{
            Program, aux::*, NonEvmCall,
        },
        state::NonEvmState, H160, BINCODE_CAP,
    },
    super::system_cached_ix::{SystemCachedReader, SystemCachedWriter,},
    solana_bincode::limited_deserialize,
    solana_system_interface::instruction::SystemInstruction::{CreateAccount, Allocate, Assign, Transfer,},
};
//      invokes
//  0xe0402a8d       create_pda()
//  0x4ceab657       create_pda(uint64)                             create_pda(lamports)
//  0x48e2bb86       create_pda(uint64,bytes32)                     create_pda(lamports, salt)
//  0xcc258bbf       create_pda(bytes32,uint64,bytes32)             create_pda(owner, len, salt)

//  0x93225c9f       allocate(uint64,bytes32)                       allocate(len, salt)
//  0x8ac00bdc       assign(bytes32,bytes32)                        assign(owner, salt)

//  0x5d359fbd       transfer(address,uint64)                       transfer(to, lamports)
//  0xfd54d1ea       transfer(bytes32,uint64)                       transfer(to, lamports)
//  0x875abfc0       transfer(bytes32,uint64,bytes32)               transfer(to, lamports, salt)
const CREATE_PDA: &[u8] = &[0xe0, 0x40, 0x2a, 0x8d];
const CREATE_PDA_LAMPORTS: &[u8] = &[0x4c, 0xea, 0xb6, 0x57];
const CREATE_PDA_LAMPORTS_SALT: &[u8] = &[0x48, 0xe2, 0xbb, 0x86];
const CREATE_PDA_OWNER_LEN_SALT: &[u8] = &[0xcc, 0x25, 0x8b, 0xbf];
const ALLOCATE: &[u8] = &[0x93, 0x22, 0x5c, 0x9f];
const ASSIGN: &[u8] = &[0x8a, 0xc0, 0x0b, 0xdc];
const TRANSFER: &[u8] = &[0x5d, 0x35, 0x9f, 0xbd];
const TRANSFER_B32: &[u8] = &[0xfd, 0x54, 0xd1, 0xea];
const TRANSFER_B32_SALT: &[u8] = &[0x87, 0x5a, 0xbf, 0xc0];

pub struct SystemCached<'a, T: Origin> {
    pub state: &'a T,
}
impl<'a, T: Origin> SystemCached<'a, T> {
    pub const ADDRESS: H160 = H160([
        0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x04,
    ]);
    pub fn new(state: &'a T) -> Self {
        Self {
            state
        }
    }
}

impl<'a, T: Origin> Program for SystemCached<'a, T> {
    fn from_abi<'b>(&self, _: &'b CallInterrupt) -> Result<NonEvmCall<'b>> {
        unreachable!()
    }
    fn from_abi_cached<'c>(&self, params: &'c CallInterrupt, nes: Option<&NonEvmState>) -> Result<NonEvmCall<'c>> {
        not_payable(params)?;
        let (a, b) = params.input.split_at(4);
        let reader = SystemCachedReader::new(self.state, nes);

        let call = match a {
            CREATE_PDA => {
                let (ix, seeds) = reader.create_pda(params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            CREATE_PDA_LAMPORTS => {
                let (ix, seeds) = reader.create_pda_lamports(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            CREATE_PDA_LAMPORTS_SALT => {
                let (ix, seeds) = reader.create_pda_lamports_salt(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            CREATE_PDA_OWNER_LEN_SALT => {
                let (ix, seeds) = reader.create_pda_owner_len_salt(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            ALLOCATE => {
                let (ix, seeds) = reader.allocate(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            ASSIGN => {
                let (ix, seeds) = reader.assign(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            TRANSFER => {
                let (ix, seeds) = reader.transfer(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            TRANSFER_B32 => {
                let (ix, seeds) = reader.transfer_b32(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            TRANSFER_B32_SALT => {
                let (ix, seeds) = reader.transfer_b32_salt(b, params)?;
                NonEvmCall::Invoke(ix, seeds)
            },
            _ =>  return Err(Unimplemented(format!("method is not supported by SystemProgram {}", hex::encode(a)))),
        };

        Ok(call)
    }
    fn emulate(&self, ix: &Instruction, nes: &mut NonEvmState) -> Result<()>  {
        let mut writer = SystemCachedWriter::new(self.state, nes);

        match limited_deserialize(&ix.data, BINCODE_CAP).map_err(|_| InvalidNonEvmInstructionData)? {
            CreateAccount{lamports, space, owner} =>
                writer.create_account(&ix, lamports, space, &owner),
            Allocate {space} => writer.allocate(&ix, space),
            Assign {owner} => writer.assign(&ix, &owner),
            Transfer {lamports} => writer.transfer(&ix, lamports),
            _ => Err(Unimplemented("instruction is not supported by SystemProgram".to_string())),
        }
    }
    fn eth_call(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
        unreachable!()
    }
    fn cross_state_call(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
        unimplemented!()
    }
    fn cross_state_call_cached(&self, _: &[u8], _: &[u8], _: Option<&NonEvmState>) -> Result<Vec<u8>> {
        unimplemented!()
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
    use {
        crate::{context::AccountLock, pda::Seed, state::Base, Account, U256},
        solana_program::{account_info::AccountInfo, instruction::AccountMeta, pubkey::Pubkey},
        solana_system_interface::{instruction::SystemInstruction, program as system_program},
    };

    /// Minimal `Origin` stub so `SystemCached::emulate()` can run for real,
    /// outside a live Solana/Mollusk runtime. `emulate()`/`SystemCachedWriter`
    /// never touch anything but the `NonEvmState` overlay the test passes in
    /// directly -- everything else here is unreachable and panics if that
    /// changes.
    struct TestOrigin<'a> {
        base: Base<'a>,
    }

    impl<'a> TestOrigin<'a> {
        fn new(program_id: &'a Pubkey) -> Self {
            Self {
                base: Base::new(program_id, 1, None),
            }
        }
    }

    impl<'a> Origin for TestOrigin<'a> {
        fn nonce(&self, _: &H160) -> Result<Option<u64>> {
            unimplemented!()
        }
        fn balance(&self, _: &H160) -> Result<Option<U256>> {
            unimplemented!()
        }
        fn code(&self, _: &H160) -> Result<Option<Vec<u8>>> {
            unimplemented!()
        }
        fn valids(&self, _: &H160) -> Result<Option<Vec<u8>>> {
            unimplemented!()
        }
        fn storage(&self, _: &H160, _: &U256) -> Result<Option<U256>> {
            unimplemented!()
        }
        fn inc_nonce<L: AccountLock>(&self, _: &H160, _: &L) -> Result<()> {
            unimplemented!()
        }
        fn add_balance<L: AccountLock>(&self, _: &H160, _: &U256, _: &L) -> Result<()> {
            unimplemented!()
        }
        fn sub_balance<L: AccountLock>(&self, _: &H160, _: &U256, _: &L) -> Result<()> {
            unimplemented!()
        }
        fn set_code<L: AccountLock>(&self, _: &H160, _: &[u8], _: &[u8], _: &L) -> Result<()> {
            unimplemented!()
        }
        fn set_storage<L: AccountLock>(&self, _: &H160, _: &U256, _: &U256, _: &L) -> Result<()> {
            unimplemented!()
        }
        fn base(&self) -> &Base<'_> {
            &self.base
        }
        fn account(&self, _: &Pubkey) -> Result<Account> {
            unimplemented!()
        }
        fn with_account_info<F, R>(&self, _: &Pubkey, _: F) -> Result<R>
        where
            F: FnOnce(&AccountInfo) -> Result<R>,
        {
            unimplemented!()
        }
        fn invoke_signed(&self, _: &Instruction, _: Vec<Seed>, _: bool) -> Result<()> {
            unimplemented!()
        }
        fn invoke_signed_unchecked(&self, _: &Instruction, _: Vec<Seed>) -> Result<()> {
            unimplemented!()
        }
        fn signer(&self) -> Pubkey {
            unimplemented!()
        }
        fn wallet(&self) -> Result<Pubkey> {
            unimplemented!()
        }
        fn treasure(&self, _: u64) -> Result<Pubkey> {
            unimplemented!()
        }
        fn owner(&self, _: &Pubkey) -> Result<Pubkey> {
            unimplemented!()
        }
        fn ed25519_data(&self) -> Result<Vec<u8>> {
            unimplemented!()
        }
    }

    fn system_owned(lamports: u64) -> Account {
        Account {
            lamports,
            data: vec![],
            owner: system_program::ID,
            executable: false,
            writable: true,
            signer: false,
        }
    }

    // bincode fixint encoding: enum discriminant as u32 LE, then fields in
    // declaration order with no framing between them.
    // SystemInstruction::Transfer { lamports: u64 } -- discriminant 2.
    fn transfer_payload(lamports: u64) -> Vec<u8> {
        let mut data = vec![];
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&lamports.to_le_bytes());
        data
    }

    // SystemInstruction::CreateAccount { lamports, space, owner } -- discriminant 0.
    fn create_account_payload(lamports: u64, space: u64, owner: Pubkey) -> Vec<u8> {
        let mut data = vec![];
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&lamports.to_le_bytes());
        data.extend_from_slice(&space.to_le_bytes());
        data.extend_from_slice(&owner.to_bytes());
        data
    }

    // SystemInstruction::CreateAccountWithSeed { base, seed: String, lamports,
    // space, owner } -- discriminant 3. `seed` is the only length-prefixed
    // field on any SystemInstruction variant -- the exact shape find-029
    // exploited: a u64 LE length prefix read before the declared number of
    // bytes is known to exist in the buffer.
    //
    // Forged variant: declares `declared_len` with zero real seed bytes
    // behind it. Caller picks `declared_len` strictly between isize::MAX and
    // u64::MAX (exclusive of literal u64::MAX itself, which the running
    // byte-budget check catches for an unrelated reason even with no real
    // cap): in that band, pre-fix (limit=u64::MAX) bincode's running budget
    // check passes, so its IoReader unconditionally does
    // `temp_buffer.resize(declared_len, 0)` before reading anything -- an
    // immediate, deterministic "capacity overflow" panic (the layout check
    // trips before any real allocation is attempted, so this is safe to run
    // in a test). With BINCODE_CAP the length is checked against the
    // remaining budget first and refused cleanly, no matter how large.
    fn forged_create_account_with_seed(declared_len: u64) -> Vec<u8> {
        let mut data = vec![];
        data.extend_from_slice(&3u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 32]); // base
        data.extend_from_slice(&declared_len.to_le_bytes());
        data
    }

    // Same shape, but well-formed: `seed_len` real bytes are actually
    // present, plus the trailing lamports/space/owner fields, so the total
    // byte count can be pinned to an exact value for boundary testing.
    fn well_formed_create_account_with_seed(seed_len: usize) -> Vec<u8> {
        let mut data = vec![];
        data.extend_from_slice(&3u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 32]); // base
        data.extend_from_slice(&(seed_len as u64).to_le_bytes());
        data.extend(std::iter::repeat_n(b'a', seed_len));
        data.extend_from_slice(&0u64.to_le_bytes()); // lamports
        data.extend_from_slice(&0u64.to_le_bytes()); // space
        data.extend_from_slice(&[0u8; 32]); // owner
        data
    }

    /// The malformed payload is rejected with the clean error emulate() maps
    /// deserialize failures to -- NOT a panic. This is the find-029 exploit
    /// shape at the `emulate()` (cached-track) site.
    #[test]
    fn forged_declared_length_is_rejected_not_panicked() {
        let program_id = Pubkey::new_unique();
        let origin = TestOrigin::new(&program_id);
        let sc = SystemCached::new(&origin);
        let mut nes = NonEvmState::default();

        let ix = Instruction {
            program_id: system_program::ID,
            accounts: vec![],
            data: forged_create_account_with_seed(u64::MAX - 1000),
        };
        let result = sc.emulate(&ix, &mut nes);

        assert!(
            matches!(result, Err(InvalidNonEvmInstructionData)),
            "expected a clean InvalidNonEvmInstructionData, got {result:?}"
        );
    }

    /// Legitimate traffic must still deserialize AND execute -- the bound
    /// must not break real traffic. Exercises both handled variants fully
    /// (not just parse) since emulate()'s match runs the real writer.
    #[test]
    fn legit_transfer_and_create_account_still_execute() {
        let program_id = Pubkey::new_unique();
        let origin = TestOrigin::new(&program_id);
        let sc = SystemCached::new(&origin);

        let from = Pubkey::new_unique();
        let to = Pubkey::new_unique();
        let mut nes = NonEvmState::default();
        nes.insert(from, system_owned(10_000));
        nes.insert(to, system_owned(0));

        let transfer_ix = Instruction {
            program_id: system_program::ID,
            accounts: vec![AccountMeta::new(from, false), AccountMeta::new(to, false)],
            data: transfer_payload(1_000),
        };
        sc.emulate(&transfer_ix, &mut nes)
            .expect("a legit Transfer must deserialize and execute");
        assert_eq!(nes.get(&from).unwrap().lamports, 9_000);
        assert_eq!(nes.get(&to).unwrap().lamports, 1_000);

        let signer = Pubkey::new_unique();
        let new_account = Pubkey::new_unique();
        let mut nes = NonEvmState::default();
        nes.insert(signer, system_owned(10_000));
        nes.insert(new_account, system_owned(0));

        let create_ix = Instruction {
            program_id: system_program::ID,
            accounts: vec![
                AccountMeta::new(signer, true),
                AccountMeta::new(new_account, false),
            ],
            data: create_account_payload(5_000, 0, system_program::ID),
        };
        sc.emulate(&create_ix, &mut nes)
            .expect("a legit CreateAccount must deserialize and execute");
        assert_eq!(nes.get(&new_account).unwrap().lamports, 5_000);
    }

    /// Drives the boundary off BINCODE_CAP itself, not a copied literal.
    /// A payload whose total consumed bytes equal the cap deserializes; one
    /// byte more is rejected at the SAME deserialize step, not downstream.
    #[test]
    fn bincode_cap_boundary_accepts_at_cap_rejects_above() {
        let program_id = Pubkey::new_unique();
        let origin = TestOrigin::new(&program_id);
        let sc = SystemCached::new(&origin);

        // Self-check: confirm the hand-rolled encoding round-trips before
        // using its byte count to compute the boundary.
        let zero_seed = well_formed_create_account_with_seed(0);
        let parsed: SystemInstruction = limited_deserialize(&zero_seed, u64::MAX)
            .expect("hand-rolled CreateAccountWithSeed encoding must parse");
        assert!(
            matches!(parsed, SystemInstruction::CreateAccountWithSeed { .. }),
            "expected CreateAccountWithSeed, got {parsed:?}"
        );

        let overhead = zero_seed.len() as u64;
        let seed_len_at_cap = (BINCODE_CAP - overhead) as usize;

        let at_cap = well_formed_create_account_with_seed(seed_len_at_cap);
        assert_eq!(at_cap.len() as u64, BINCODE_CAP);
        let ix_at_cap = Instruction {
            program_id: system_program::ID,
            accounts: vec![],
            data: at_cap,
        };
        let result_at_cap = sc.emulate(&ix_at_cap, &mut NonEvmState::default());
        assert!(
            !matches!(result_at_cap, Err(InvalidNonEvmInstructionData)),
            "a payload of exactly BINCODE_CAP bytes must deserialize: {result_at_cap:?}"
        );

        let above_cap = well_formed_create_account_with_seed(seed_len_at_cap + 1);
        let ix_above_cap = Instruction {
            program_id: system_program::ID,
            accounts: vec![],
            data: above_cap,
        };
        let result_above_cap = sc.emulate(&ix_above_cap, &mut NonEvmState::default());
        assert!(
            matches!(result_above_cap, Err(InvalidNonEvmInstructionData)),
            "a payload one byte over BINCODE_CAP must be rejected at deserialize: {result_above_cap:?}"
        );
    }
}
