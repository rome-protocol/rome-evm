use {
    solana_bincode::limited_deserialize,
    solana_system_interface::instruction::SystemInstruction::Transfer,
    solana_program::{
        instruction::Instruction, pubkey::Pubkey,
    },
    solana_system_interface::program as system_program,
    crate::{
        error::Result, origin::Origin, error::RomeProgramError::*,
        H160, pda::Seed, handler::CallInterrupt, BINCODE_CAP,
    },
    super::{
        u64_to_bytes32, align_len, get_pubkey, add_dyn_to_static, get_account_meta, get_slice,
        get_vec_bytes32, len_eq,
    },
    std::convert::TryFrom,
};

#[repr(C, packed)]
#[derive(Default)]
pub struct AccInfo {
    pub lamports: [u8; 32],
    pub owner: Pubkey,
    pub is_signer: [u8; 32],
    pub is_writable: [u8; 32],
    pub executable: [u8; 32],
}

pub struct CpiProgram<'a, T: Origin> {
    pub state: &'a T,
}

impl<'a, T: Origin> CpiProgram<'a, T> {
    pub const ADDRESS: H160 = H160([
        0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x08,
    ]);
    pub fn  new(state: &'a T) -> Self {
        Self {
            state
        }
    }
    pub fn invoke_ix(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (program_id, _) = get_pubkey(abi)?;
        let meta = get_account_meta(abi, 32)?;
        let data = get_slice(abi, 64, 1)?.to_vec();

        // affects emulation
        if meta.is_empty() {
            return Err(NonEvmCallError("CpiProgam.invoke_signed(): accounts not found".to_string()))
        }
        let ix = Instruction { program_id, accounts: meta, data };
        let (auth, with_bump) = self.state.base().pda.external_auth(&params.context.caller);

        let mut seeds = vec![];
        let found_default_signer = ix
            .accounts
            .iter()
            .any(|a| a.pubkey == auth && a.is_signer);

        // The bare authority is where the user's assets live (their ATA, their
        // lamports, their main PDA). invoke() has no salt param, so this is its
        // only signer path — refusing it under delegatecall is a full close, not
        // a narrowing, of what invoke() can do there.
        if found_default_signer {
            if !params.owner_authenticated() {
                return Err(DelegatecallOwnerAuthority)
            }
            seeds.push(with_bump);
        };

        self.verify(&ix)?;
        Ok((ix, seeds))
    }
    pub fn invoke_signed_ix(&self, abi: &[u8], params: &CallInterrupt) -> Result<(Instruction, Vec<Seed>)> {
        let (program_id, _) = get_pubkey(abi)?;
        let meta = get_account_meta(abi, 32)?;
        let data = get_slice(abi, 64, 1)?.to_vec();
        let salt_vec = get_vec_bytes32(abi, 96)?;

        let seed_no_bump = self.state.base().pda.external_auth_seed(&params.context.caller);
        // Delegatecall: fold the running contract's address into the seed so one
        // contract's salt cannot resolve to another contract's authority. Direct
        // call: context.address is the precompile's own fixed address (no
        // distinguishing identity) — omit it so direct-call PDAs are unchanged.
        let contract_ns = (!params.owner_authenticated()).then_some(params.context.address);
        let (keys, mut seeds) = self.do_seeds(salt_vec, seed_no_bump, contract_ns);

        for key in keys {
            let found  = meta.iter().any(|a| a.pubkey == key && a.is_signer);
            if !found {
                return Err(NonEvmCallError("signer account calculated from the provided seed was not found".to_string()))
            }
        }
        // affects emulation
        if meta.is_empty() {
            return Err(NonEvmCallError("CpiProgam.invoke_signed(): accounts not found".to_string()))
        }
        let ix = Instruction { program_id, accounts: meta, data };
        let (auth, with_bump) = self.state.base().pda.external_auth(&params.context.caller);

        let found_default_signer = ix
            .accounts
            .iter()
            .any(|a| a.pubkey == auth && a.is_signer);

        // Same asset-boundary refusal as invoke_ix: the bare authority is never
        // reachable by delegatecall, salted or not.
        if found_default_signer {
            if !params.owner_authenticated() {
                return Err(DelegatecallOwnerAuthority)
            }
            seeds.push(with_bump);
        };

        self.verify(&ix)?;
        Ok((ix, seeds))
    }

    // Seed order: [EXTERNAL_AUTHORITY, caller, contract_ns?, salt, bump]. contract_ns
    // is Some(context.address) only under delegatecall/callcode (see call sites) —
    // that's what namespaces a salted authority to the contract that derived it, so
    // a different contract naming the same salt gets a different key.
    pub fn do_seeds(&self, salt_vec: Vec<&[u8]>, base: Seed, contract_ns: Option<H160>) -> (Vec<Pubkey>, Vec<Seed>) {
        let program_id = self.state.base().program_id;

        salt_vec
            .into_iter()
            .map(|salt| {
                let mut seed =  base.clone();
                if let Some(addr) = contract_ns {
                    seed.add_seed(addr.as_bytes().to_vec());
                }
                seed.add_seed(salt.to_vec());
                let (key, bump) = self.state.base().pda.find_program_address(seed.cast().as_slice(), &program_id);
                seed.add(bump);
                (key, seed)
            })
            .collect::<Vec<_>>()
            .into_iter()
            .unzip()
    }

    pub fn verify(&self, ix: &Instruction) ->Result<()> {
        let operator = self.state.signer();
        let program_id = *self.state.base().program_id;
        let mut operator_prohibited = true;

        for meta in ix.accounts.iter() {
            if meta.pubkey == operator && operator_prohibited {
                if ix.program_id == system_program::id() {
                    let ix_data = limited_deserialize(&ix.data, BINCODE_CAP).map_err(|e| DeserializeInstructionDataError(e.to_string()))?;
                    match ix_data {
                        Transfer {lamports: _} => {
                            operator_prohibited = false;
                            continue
                        },
                        _ => {},
                    }
                }
                return Err(NonEvmCallError("attempt to use operator's account".to_string()))
            } else if meta.pubkey == program_id {
                return Err(NonEvmCallError("attempt to use program_id account".to_string()))
            }
        }

        if ix.program_id == *self.state.base().program_id {
            return Err(NonEvmCallError("recursive call prohibited ".to_string()));
        }

        Ok(())
    }

    pub fn account_info(&self, abi: &[u8]) -> Result<Vec<u8>> {
        len_eq!(abi, 32);
        let key = Pubkey::try_from(abi).map_err(|_| InvalidNonEvmInstructionData)?;
        let acc = self.state.account(&key)?;

        let len = size_of::<AccInfo>() + align_len(acc.data.len());
        let mut  vec = Vec::with_capacity(len);

        vec.resize(size_of::<AccInfo>(), 0_u8);

        let ptr = vec.as_mut_ptr().cast::<AccInfo>();
        let dst = unsafe { &mut *ptr };

        u64_to_bytes32(acc.lamports, &mut dst.lamports);
        dst.owner = acc.owner;
        *dst.is_signer.last_mut().unwrap() = acc.signer.into();
        *dst.is_writable.last_mut().unwrap() = acc.writable.into();
        *dst.executable.last_mut().unwrap() = acc.executable.into();

        Ok(add_dyn_to_static(vec, &acc.data))
    }
}

#[cfg(test)]
mod bincode_cap_tests {
    use super::*;
    use {
        crate::{
            context::AccountLock,
            state::{Account, Base},
            U256,
        },
        solana_program::{account_info::AccountInfo, instruction::AccountMeta},
        solana_system_interface::instruction::SystemInstruction,
    };

    /// Minimal `Origin` stub so `CpiProgram::verify()` can run for real,
    /// outside a live Solana/Mollusk runtime. `verify()` only ever touches
    /// `signer()` and `base().program_id` -- everything else here is
    /// unreachable from the paths under test and panics if that changes.
    struct TestOrigin<'a> {
        base: Base<'a>,
        signer: Pubkey,
    }

    impl<'a> TestOrigin<'a> {
        fn new(program_id: &'a Pubkey, signer: Pubkey) -> Self {
            Self {
                base: Base::new(program_id, 1, None),
                signer,
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
            self.signer
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

    fn ix_targeting_operator(operator: Pubkey, data: Vec<u8>) -> Instruction {
        Instruction {
            program_id: system_program::id(),
            accounts: vec![AccountMeta::new(operator, true)],
            data,
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
    // (String) field on any SystemInstruction variant, which is exactly the
    // shape the finding exploited: a u64 LE length prefix read BEFORE the
    // declared number of bytes is known to exist in the buffer.
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

    /// The malformed payload is rejected with the clean error verify() maps
    /// deserialize failures to -- NOT a panic. This is the find-029
    /// exploit shape: a guest-forged declared length with no bytes behind it.
    #[test]
    fn forged_declared_length_is_rejected_not_panicked() {
        let program_id = Pubkey::new_unique();
        let operator = Pubkey::new_unique();
        let origin = TestOrigin::new(&program_id, operator);
        let cpi = CpiProgram::new(&origin);

        let ix = ix_targeting_operator(operator, forged_create_account_with_seed(u64::MAX - 1000));
        let result = cpi.verify(&ix);

        assert!(
            matches!(result, Err(DeserializeInstructionDataError(_))),
            "expected a clean DeserializeInstructionDataError, got {result:?}"
        );
    }

    /// Legitimate traffic must still deserialize. A real Transfer clears
    /// verify() entirely (Ok); a real CreateAccount is still refused by
    /// verify()'s operator-account POLICY, but that refusal must never be
    /// the deserialize error -- the bound must not break real traffic.
    #[test]
    fn legit_transfer_and_create_account_still_deserialize() {
        let program_id = Pubkey::new_unique();
        let operator = Pubkey::new_unique();
        let origin = TestOrigin::new(&program_id, operator);
        let cpi = CpiProgram::new(&origin);

        let transfer_ix = ix_targeting_operator(operator, transfer_payload(1_000));
        assert!(
            cpi.verify(&transfer_ix).is_ok(),
            "a legit Transfer of the operator's own account must clear verify()"
        );

        let create_ix =
            ix_targeting_operator(operator, create_account_payload(0, 0, Pubkey::new_unique()));
        let result = cpi.verify(&create_ix);
        assert!(
            !matches!(result, Err(DeserializeInstructionDataError(_))),
            "CreateAccount must deserialize; any rejection must be policy \
             (attempt to use operator's account), not a parse failure: {result:?}"
        );
    }

    /// Drives the boundary off BINCODE_CAP itself, not a copied literal.
    /// A payload whose total consumed bytes equal the cap deserializes; one
    /// byte more is rejected at the SAME deserialize step, not downstream.
    #[test]
    fn bincode_cap_boundary_accepts_at_cap_rejects_above() {
        let program_id = Pubkey::new_unique();
        let operator = Pubkey::new_unique();
        let origin = TestOrigin::new(&program_id, operator);
        let cpi = CpiProgram::new(&origin);

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
        let result_at_cap = cpi.verify(&ix_targeting_operator(operator, at_cap));
        assert!(
            !matches!(result_at_cap, Err(DeserializeInstructionDataError(_))),
            "a payload of exactly BINCODE_CAP bytes must deserialize: {result_at_cap:?}"
        );

        let above_cap = well_formed_create_account_with_seed(seed_len_at_cap + 1);
        let result_above_cap = cpi.verify(&ix_targeting_operator(operator, above_cap));
        assert!(
            matches!(result_above_cap, Err(DeserializeInstructionDataError(_))),
            "a payload one byte over BINCODE_CAP must be rejected at deserialize: {result_above_cap:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            accounts::OwnerInfo,
            context::AccountLock,
            error::RomeProgramError,
            state::{base::Base, Account, Allocate},
            EXTERNAL_AUTHORITY,
        },
        evm::{Context, U256},
        solana_program::account_info::AccountInfo,
    };

    static PROGRAM_ID: Pubkey = Pubkey::new_from_array([7; 32]);
    static OPERATOR: Pubkey = Pubkey::new_from_array([0xee; 32]);
    static CPI_TARGET: Pubkey = Pubkey::new_from_array([0x42; 32]);

    const CALLER: H160 = H160([0xaa; 20]);
    const ADAPTER_A: H160 = H160([0xbb; 20]);
    const ADAPTER_B: H160 = H160([0xcc; 20]);

    // Minimal Origin stub: enough to construct CpiProgram and reach invoke_ix /
    // invoke_signed_ix / do_seeds. Everything unused by those paths panics loudly.
    struct Stub {
        base: Base<'static>,
    }
    impl Stub {
        fn new() -> Self {
            let owner_info = OwnerInfo {
                chain: 1,
                mint: Some(Pubkey::new_from_array([9; 32])),
                slot: 0,
                single_state: true,
            };
            Self {
                base: Base::new(&PROGRAM_ID, 1, Some(owner_info)),
            }
        }
    }
    impl Allocate for Stub {
        fn alloc_balance<L: AccountLock>(&self, _: &H160, _: &L) -> Result<()> {
            unimplemented!()
        }
        fn alloc_slots<L: AccountLock>(
            &self,
            _: &Pubkey,
            _: &Seed,
            _: usize,
            _: &L,
            _: &H160,
        ) -> Result<bool> {
            unimplemented!()
        }
        fn alloc_slots_unchecked(&self, _: &Pubkey, _: &Seed, _: usize, _: &H160) -> Result<()> {
            unimplemented!()
        }
        fn alloc_contract<L: AccountLock>(
            &self,
            _: &H160,
            _: &[u8],
            _: &[u8],
            _: &L,
        ) -> Result<bool> {
            unimplemented!()
        }
    }
    impl Origin for Stub {
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
            Ok(())
        }
        fn invoke_signed_unchecked(&self, _: &Instruction, _: Vec<Seed>) -> Result<()> {
            unimplemented!()
        }
        fn signer(&self) -> Pubkey {
            OPERATOR
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

    fn call(context_address: H160) -> CallInterrupt {
        CallInterrupt {
            code_address: CpiProgram::<'static, Stub>::ADDRESS,
            transfer: None,
            input: vec![],
            is_static: false,
            context: Context {
                address: context_address,
                caller: CALLER,
                apparent_value: U256::zero(),
            },
        }
    }
    // context.address == code_address: owner_authenticated() true
    fn direct() -> CallInterrupt {
        call(CpiProgram::<'static, Stub>::ADDRESS)
    }
    // context.address is the delegatecall site (the running contract)
    fn delegatecall(adapter: H160) -> CallInterrupt {
        call(adapter)
    }

    fn u256_be(n: u64) -> [u8; 32] {
        let v: U256 = n.into();
        let mut b = [0u8; 32];
        v.to_big_endian(&mut b);
        b
    }
    fn align_up32(len: usize) -> usize {
        if len.is_multiple_of(32) { len } else { (len / 32 + 1) * 32 }
    }

    // Solidity ABI encoding of invoke(bytes32,(bytes32,bool,bool)[],bytes) minus
    // the 4-byte selector, matching what invoke_ix's get_pubkey/get_account_meta/
    // get_slice offsets expect.
    fn encode_invoke(program_id: Pubkey, accounts: &[(Pubkey, bool, bool)], data: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        let offset_accounts = 96usize;
        let accounts_area_len = 32 + accounts.len() * 96;
        let offset_data = offset_accounts + accounts_area_len;

        buf.extend_from_slice(program_id.as_ref());
        buf.extend_from_slice(&u256_be(offset_accounts as u64));
        buf.extend_from_slice(&u256_be(offset_data as u64));

        buf.extend_from_slice(&u256_be(accounts.len() as u64));
        for (pk, is_signer, is_writable) in accounts {
            buf.extend_from_slice(pk.as_ref());
            buf.extend_from_slice(&u256_be(*is_signer as u64));
            buf.extend_from_slice(&u256_be(*is_writable as u64));
        }

        buf.extend_from_slice(&u256_be(data.len() as u64));
        buf.extend_from_slice(data);
        buf.resize(buf.len() + align_up32(data.len()) - data.len(), 0u8);
        buf
    }

    // Same, for invoke_signed(bytes32,(bytes32,bool,bool)[],bytes,bytes32[]).
    fn encode_invoke_signed(
        program_id: Pubkey,
        accounts: &[(Pubkey, bool, bool)],
        data: &[u8],
        salts: &[[u8; 32]],
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        let offset_accounts = 128usize;
        let accounts_area_len = 32 + accounts.len() * 96;
        let offset_data = offset_accounts + accounts_area_len;
        let data_area_len = 32 + align_up32(data.len());
        let offset_salts = offset_data + data_area_len;

        buf.extend_from_slice(program_id.as_ref());
        buf.extend_from_slice(&u256_be(offset_accounts as u64));
        buf.extend_from_slice(&u256_be(offset_data as u64));
        buf.extend_from_slice(&u256_be(offset_salts as u64));

        buf.extend_from_slice(&u256_be(accounts.len() as u64));
        for (pk, is_signer, is_writable) in accounts {
            buf.extend_from_slice(pk.as_ref());
            buf.extend_from_slice(&u256_be(*is_signer as u64));
            buf.extend_from_slice(&u256_be(*is_writable as u64));
        }

        buf.extend_from_slice(&u256_be(data.len() as u64));
        buf.extend_from_slice(data);
        buf.resize(buf.len() + align_up32(data.len()) - data.len(), 0u8);

        buf.extend_from_slice(&u256_be(salts.len() as u64));
        for s in salts {
            buf.extend_from_slice(s);
        }
        buf
    }

    // The property both invoke fns must hold, asserted once over both rather than
    // trusting two copies of the refusal to stay in sync: under !owner_authenticated,
    // NEITHER function may ever return a seed equal to external_auth(caller)'s.
    // A future third entry point that forgets the check fails here too, provided it
    // is added to the list below.
    #[test]
    fn no_invoke_path_returns_the_bare_caller_seed_under_delegatecall() {
        let s = Stub::new();
        let cpi = CpiProgram::new(&s);
        let params = delegatecall(ADAPTER_A);
        let (bare, bare_seed) = s.base.pda.external_auth(&CALLER);
        let salt = [0x01u8; 32];

        // Every shape that could plausibly smuggle the bare seed through: the bare
        // account named as signer, named as non-signer, and alongside a salted one.
        let shapes: Vec<Vec<(Pubkey, bool, bool)>> = vec![
            vec![(bare, true, false)],
            vec![(bare, false, true)],
            vec![(CPI_TARGET, false, false), (bare, true, false)],
        ];

        for accounts in shapes {
            for (name, out) in [
                ("invoke", cpi.invoke_ix(&encode_invoke(CPI_TARGET, &accounts, b"d"), &params)),
                (
                    "invoke_signed",
                    cpi.invoke_signed_ix(
                        &encode_invoke_signed(CPI_TARGET, &accounts, b"d", &[salt]),
                        &params,
                    ),
                ),
            ] {
                if let Ok((_, seeds)) = out {
                    assert!(
                        !seeds.iter().any(|sd| sd.cast() == bare_seed.cast()),
                        "{name} returned the bare caller seed under delegatecall",
                    );
                }
            }
        }
    }

    // (a) invoke: the bare external_auth(caller) signer is the asset boundary —
    // refused whenever the call is not owner-authenticated (DELEGATECALL/CALLCODE).
    #[test]
    fn bare_authority_refused_under_delegatecall_via_invoke() {
        let s = Stub::new();
        let cpi = CpiProgram::new(&s);
        let params = delegatecall(ADAPTER_A);
        let (bare, _) = s.base.pda.external_auth(&CALLER);

        let abi = encode_invoke(CPI_TARGET, &[(bare, true, false)], b"data");
        match cpi.invoke_ix(&abi, &params) {
            Err(e) => assert!(matches!(e, RomeProgramError::DelegatecallOwnerAuthority)),
            Ok(_) => panic!("expected DelegatecallOwnerAuthority"),
        }
    }

    // (a) invoke_signed twin.
    #[test]
    fn bare_authority_refused_under_delegatecall_via_invoke_signed() {
        let s = Stub::new();
        let cpi = CpiProgram::new(&s);
        let params = delegatecall(ADAPTER_A);
        let (bare, _) = s.base.pda.external_auth(&CALLER);

        let abi = encode_invoke_signed(CPI_TARGET, &[(bare, true, false)], b"data", &[]);
        match cpi.invoke_signed_ix(&abi, &params) {
            Err(e) => assert!(matches!(e, RomeProgramError::DelegatecallOwnerAuthority)),
            Ok(_) => panic!("expected DelegatecallOwnerAuthority"),
        }
    }

    // (b) A salted-only signer is allowed under delegatecall, and the key produced
    // is namespaced by the running contract's address (context.address) — changing
    // only that address changes the derived key.
    #[test]
    fn salted_authority_allowed_under_delegatecall_and_namespaced_by_contract() {
        let s = Stub::new();
        let cpi = CpiProgram::new(&s);
        let params = delegatecall(ADAPTER_A);
        let salt = [0x01u8; 32];

        let seed_no_bump = s.base.pda.external_auth_seed(&CALLER);
        let ns = (!params.owner_authenticated()).then_some(params.context.address);
        let (keys, seeds) = cpi.do_seeds(vec![&salt], seed_no_bump.clone(), ns);
        let expected_key = keys[0];

        let abi = encode_invoke_signed(
            CPI_TARGET,
            &[(expected_key, true, false)],
            b"data",
            &[salt],
        );
        let (ix, seeds_out) = cpi.invoke_signed_ix(&abi, &params).unwrap();
        assert_eq!(ix.accounts[0].pubkey, expected_key);
        assert_eq!(seeds_out.len(), seeds.len());

        // Control: same caller, same salt, different context.address -> different key.
        let ns_b = Some(ADAPTER_B);
        let (keys_b, _) = cpi.do_seeds(vec![&salt], seed_no_bump, ns_b);
        assert_ne!(keys_b[0], expected_key, "namespace must be real, not a no-op");
    }

    // (c) Cross-adapter isolation: two different running contracts, same caller,
    // same salt -> different derived authorities. This is what stops a malicious
    // contract from naming a legitimate adapter's salt and signing for its position.
    #[test]
    fn cross_adapter_isolation_same_caller_same_salt_different_contract() {
        let s = Stub::new();
        let cpi = CpiProgram::new(&s);
        let salt = [0x02u8; 32];
        let base = s.base.pda.external_auth_seed(&CALLER);

        let (keys_a, _) = cpi.do_seeds(vec![&salt], base.clone(), Some(ADAPTER_A));
        let (keys_b, _) = cpi.do_seeds(vec![&salt], base, Some(ADAPTER_B));

        assert_ne!(keys_a[0], keys_b[0]);
    }

    // (d-a) Direct-call twin of (a): unchanged from 631a0e1 — owner_authenticated
    // calls were never gated, so the bare authority still signs.
    #[test]
    fn direct_call_bare_authority_unchanged() {
        let s = Stub::new();
        let cpi = CpiProgram::new(&s);
        let params = direct();
        let (bare, with_bump) = s.base.pda.external_auth(&CALLER);

        let abi = encode_invoke(CPI_TARGET, &[(bare, true, false)], b"data");
        let (ix, seeds) = cpi.invoke_ix(&abi, &params).unwrap();
        assert_eq!(ix.accounts[0].pubkey, bare);
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].items, with_bump.items);
    }

    // (d-b) Direct-call twin of (b): a direct call's context.address is the
    // precompile's own fixed address, not a distinguishing contract identity, so
    // it must NOT be folded in — the derived key must match the pre-carve-out
    // formula (external_auth_seed(caller) + salt, no contract segment).
    #[test]
    fn direct_call_salted_authority_unchanged() {
        let s = Stub::new();
        let cpi = CpiProgram::new(&s);
        let params = direct();
        let salt = [0x03u8; 32];

        let seed_no_bump = s.base.pda.external_auth_seed(&CALLER);
        let ns = (!params.owner_authenticated()).then_some(params.context.address);
        assert_eq!(ns, None, "direct call must not carry a namespace");
        let (keys, _) = cpi.do_seeds(vec![&salt], seed_no_bump, ns);

        let expected = Pubkey::find_program_address(
            &[EXTERNAL_AUTHORITY, CALLER.as_bytes(), &salt],
            &PROGRAM_ID,
        );
        assert_eq!(keys[0], expected.0);
    }
}
