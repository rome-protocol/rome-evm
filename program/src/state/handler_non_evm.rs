use {
    super::{aux::{revert_msg,}, JournaledState,},
    crate::{
        origin::Origin, state::{Allocate, aux::Ix,},
        non_evm::{
            Program, NonEvmCall, IxList, CpiProgram, HelperProgram, System, Withdraw,
        },
        non_evm_cached::{
            ASplCached, SplCached, SystemCached, WithdrawCached,
        },
        precompile::{
            blake2f::*, ecadd::*, ecmul::*, ecpairing::*, ecrecover::*, identity::*, ripemd_160::*,
            sha2_256::*, modexp::*,
        },
        error::{Result, RomeProgramError::*,}, pda::Seed, msg,
        state::handler::CallInterrupt,
    },
    evm::{
        ExitReason, ExitRevert::Reverted, ExitSucceed::Returned, H160,
    },
    solana_program::instruction::Instruction,
};

// Default-deny at the non-EVM dispatch boundary: a state-changing call reached by
// DELEGATECALL/CALLCODE keeps context.caller pinned to the outer caller, so the
// precompile would sign as that caller's PDA with arguments the running code chose.
// from_abi/from_abi_cached only build the ix, so refusing here commits nothing.
pub(crate) fn delegatecall_refused(params: &CallInterrupt, p: &dyn Program) -> bool {
    !params.owner_authenticated() && !p.delegatecall_exempt(params.input.get(..4).unwrap_or(&[]))
}

impl<'a, T: Origin + Allocate> JournaledState<'a, T> {
    pub fn non_evm_program(&self, address: &H160) -> Option<Box<dyn Program + 'a>>  {
        match *address {
            _ if *address == Ecrecover::ADDRESS  => Some(Box::new(Ecrecover())),
            _ if *address == Sha2::ADDRESS => Some(Box::new(Sha2())),
            _ if *address == Ripemd::ADDRESS => Some(Box::new(Ripemd())),
            _ if *address == Identity::ADDRESS => Some(Box::new(Identity())),
            _ if *address == Modexp::ADDRESS => Some(Box::new(Modexp())),
            _ if *address == Ecadd::ADDRESS => Some(Box::new(Ecadd())),
            _ if *address == Ecmul::ADDRESS => Some(Box::new(Ecmul())),
            _ if *address == Ecpairing::ADDRESS => Some(Box::new(Ecpairing())),
            _ if *address == Blake2f::ADDRESS => Some(Box::new(Blake2f())),

            _ if *address == SystemCached::<'a, T>::ADDRESS => Some(Box::new(SystemCached::new(self.state))),
            _ if *address == SplCached::<'a, T>::ADDRESS => Some(Box::new(SplCached::new(self.state))),
            _ if *address == ASplCached::<'a, T>::ADDRESS => Some(Box::new(ASplCached::new(self.state))),

            _ if *address == System::<'a, T>::ADDRESS => Some(Box::new(System::new(self.state))),
            _ if *address == CpiProgram::<'a, T>::ADDRESS => Some(Box::new(CpiProgram::new(self. state))),
            _ if *address == HelperProgram::<'a, T>::ADDRESS => Some(Box::new(HelperProgram::new(self.state))),
            _ if *address == WithdrawCached::<'a, T>::ADDRESS => Some(Box::new(WithdrawCached::new(self.state))),

            _ if *address == Withdraw::<'a, T>::ADDRESS => Some(Box::new(Withdraw::new(self.state))),
            _ => None
        }
    }

    pub fn handle_non_evm_call(
        &mut self,
        program: Box<dyn Program + 'a>,
        params: CallInterrupt,
        atomic: bool
    ) -> (ExitReason, Vec<u8>) {

        match self.ne_call(program, params, atomic) {
            Ok(v) => (ExitReason::Succeed(Returned), v),
            Err(e) => {
                msg!("non-evm call error: {}", e.to_string());
                (Reverted.into(), revert_msg(e.to_string()))
            }
        }
    }
    fn ne_call(&mut self, p: Box<dyn Program + 'a>, params: CallInterrupt, atomic: bool) -> Result<Vec<u8>> {
        // Dispatchers split a 4-byte selector off the input; precompiles take raw input of any length.
        if !p.precompile() && params.input.len() < 4 {
            return Err(InvalidNonEvmInstructionData)
        }

        let call = if p.cached() {
            let nes = self.journal.non_evm_state();
            p.from_abi_cached(&params, nes)?
        } else {
            p.from_abi(&params)?
        };

        self.verify_call(&call, &p)?;

        match call {
            NonEvmCall::EthCall(a, b) => p.eth_call(a, b),
            NonEvmCall::Precompiled(b) => p.eth_call(&[], b),
            NonEvmCall::CrossStateEthCall(a, b) => {
                if p.cached() {
                    let nes = self.journal.non_evm_state();
                    p.cross_state_call_cached(a,b, nes)
                } else {
                    p.cross_state_call(a, b)
                }
            },
            NonEvmCall::Invoke(_,_) | NonEvmCall::Composed(_) => {
                if params.is_static {
                    return Err(NonEvmCallError("static mode violation".to_string()))
                }

                if delegatecall_refused(&params, &*p) {
                    return Err(DelegatecallOwnerAuthority)
                }

                if p.cached() {
                    self.emulate_ix(call, p)?;
                } else {
                    self.execute_ix(call, atomic)?;
                }

                Ok(vec![])
            }
        }
    }
    fn verify_call(&self, call: &NonEvmCall, p: &Box<dyn Program + 'a>) -> Result<()> {
        let err = match call {
            NonEvmCall::CrossStateEthCall(_, _) if !p.cached()  => self.found_cpi_cached,
            NonEvmCall::Invoke(_, _) | NonEvmCall::Composed(_) => {
                if p.cached() {
                    self.found_cpi
                } else {
                    self.found_cpi_cached
                }
            }
            _ => false,
        };

        if err {
            return Err(NonEvmCallError("attempt to use cpi-cached program with cpi-program".to_string()))
        }

        Ok(())
    }
    fn emulate_ix(&mut self, call: NonEvmCall, program: Box<dyn Program + 'a>) -> Result<()> {
        self.found_cpi_cached = true;

        let mut f = |ix: Instruction, seeds: Vec<Seed>| -> Result<()> {
            let nes = self.journal.non_evm_state_mut();
            program.emulate(&ix, nes)?;

            let ixs = self.journal.non_evm_ix.get_or_insert(vec![]);
            ixs.push(Ix::new(ix, seeds));
            Ok(())
        };

        match call {
            NonEvmCall::Invoke(ix, seeds) =>  f(ix, seeds),
            NonEvmCall::Composed(IxList { ixs, diff} ) => {
                ixs
                    .into_iter()
                    .map(|(ix, seeds)| f(ix, seeds) )
                    .collect::<Result<Vec<_>>>()?;
                diff
                    .into_iter()
                    .for_each(|(a, b)| self.journal.get_mut(&a).push(b));
                Ok(())
            },
            _ => panic!("unexpected non-evm call type")
        }
    }
    fn execute_ix(&mut self, call: NonEvmCall, atomic: bool) -> Result<()> {
        self.found_cpi = true;
        self.journal.found_cpi = true;

        if cfg!(target_os = "solana") {     // gas-estimate should pass
            if !atomic {
                return Err(CpiProhibitedInIterativeTx)
            }
        } else {
            *self.state.base().found_cpi.borrow_mut() = true;
        }

        match call {
            NonEvmCall::Invoke(ix, seeds) =>  self.state.invoke_signed(&ix, seeds, true),
            NonEvmCall::Composed(IxList { ixs, diff} ) => {
                ixs
                    .into_iter()
                    .map(|(ix, seeds)| self.state.invoke_signed(&ix, seeds, true))
                    .collect::<Result<Vec<_>>>()?;
                diff
                    .into_iter()
                    .for_each(|(a, b)| self.journal.get_mut(&a).push(b));
                Ok(())
            },
            _ => panic!("unexpected non-evm call type")
        }
    }
}


#[cfg(test)]
mod tests {
    use {
        crate::{
            non_evm::*,
            non_evm_cached::*,
            precompile::{
                blake2f::*, ecadd::*, ecmul::*, ecpairing::*, ecrecover::*, identity::*,
                ripemd_160::*,
                sha2_256::*,
            },
            state::State,
        },
        evm::H160,
        std::{
            collections::HashSet, iter::FromIterator,
        },
    };

    #[test]
    fn unique_non_evm_addresses() {
        let list = [
            Ecrecover::ADDRESS,
            Sha2::ADDRESS,
            Ripemd::ADDRESS,
            Identity::ADDRESS,
            Ecadd::ADDRESS,
            Ecmul::ADDRESS,
            Ecpairing::ADDRESS,
            Blake2f::ADDRESS,

            SystemCached::<'static, State>::ADDRESS,
            SplCached::<'static, State>::ADDRESS,
            ASplCached::<'static, State>::ADDRESS,
            System::<'static, State>::ADDRESS,
            CpiProgram::<'static, State>::ADDRESS,
            HelperProgram::<'static, State>::ADDRESS,
            WithdrawCached::<'static, State>::ADDRESS,
            Withdraw::<'static, State>::ADDRESS,
        ];
        let set: HashSet<H160> = HashSet::from_iter(list.iter().cloned());

        assert_eq!(set.len(), list.len());
    }
}

// FIND-003: the non-EVM dispatch boundary default-denies DELEGATECALL/CALLCODE.
#[cfg(test)]
mod delegatecall_boundary {
    use {
        super::delegatecall_refused,
        crate::{
            accounts::OwnerInfo,
            context::AccountLock,
            error::{Result, RomeProgramError},
            non_evm::{CpiProgram, HelperProgram, NonEvmCall, Program, Withdraw},
            non_evm_cached::{ASplCached, SplCached, SystemCached, WithdrawCached},
            origin::Origin,
            state::{
                base::Base, handler::CallInterrupt, journal::Journal, pda::Seed, Account, Allocate,
                JournaledState,
            },
        },
        evm::{Context, H160, U256},
        solana_program::{account_info::AccountInfo, instruction::Instruction, pubkey::Pubkey},
        std::collections::BTreeMap,
    };

    static PROGRAM_ID: Pubkey = Pubkey::new_from_array([7; 32]);

    const PRECOMPILE: H160 = H160([0xff; 20]);
    const CALLER: H160 = H160([0xaa; 20]);
    const SITE: H160 = H160([0xbb; 20]);

    fn call(address: H160, selector: &[u8]) -> CallInterrupt {
        CallInterrupt {
            code_address: PRECOMPILE,
            transfer: None,
            input: selector.to_vec(),
            is_static: false,
            context: Context {
                address,
                caller: CALLER,
                apparent_value: U256::zero(),
            },
        }
    }
    // context.address == code_address
    fn direct(selector: &[u8]) -> CallInterrupt {
        call(PRECOMPILE, selector)
    }
    // context.address is the delegatecall site: the running code is not the precompile
    fn delegatecall(selector: &[u8]) -> CallInterrupt {
        call(SITE, selector)
    }

    // No delegatecall_exempt override: exercises the trait default.
    struct NoExempt;
    impl Program for NoExempt {
        fn from_abi<'a>(&self, _: &'a CallInterrupt) -> Result<NonEvmCall<'a>> {
            unimplemented!()
        }
        fn eth_call(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
            unimplemented!()
        }
        fn cross_state_call<'a>(&self, _: &'a [u8], _: &'a [u8]) -> Result<Vec<u8>> {
            unimplemented!()
        }
        fn precompile(&self) -> bool {
            false
        }
    }

    // Exempts exactly one selector.
    struct OneExempt(&'static [u8]);
    impl Program for OneExempt {
        fn from_abi<'a>(&self, _: &'a CallInterrupt) -> Result<NonEvmCall<'a>> {
            unimplemented!()
        }
        fn eth_call(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
            unimplemented!()
        }
        fn cross_state_call<'a>(&self, _: &'a [u8], _: &'a [u8]) -> Result<Vec<u8>> {
            unimplemented!()
        }
        fn precompile(&self) -> bool {
            false
        }
        fn delegatecall_exempt(&self, selector: &[u8]) -> bool {
            selector == self.0
        }
    }

    // Always yields a mutating call, so ne_call reaches the Invoke arm. Exempts one selector.
    struct Mutating(&'static [u8]);
    impl Program for Mutating {
        fn from_abi<'a>(&self, _: &'a CallInterrupt) -> Result<NonEvmCall<'a>> {
            let ix = Instruction::new_with_bytes(Pubkey::new_unique(), &[], vec![]);
            Ok(NonEvmCall::Invoke(ix, vec![]))
        }
        fn eth_call(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
            unimplemented!()
        }
        fn cross_state_call<'a>(&self, _: &'a [u8], _: &'a [u8]) -> Result<Vec<u8>> {
            unimplemented!()
        }
        fn precompile(&self) -> bool {
            false
        }
        fn delegatecall_exempt(&self, selector: &[u8]) -> bool {
            selector == self.0
        }
    }

    // delegatecall_exempt() reads no state; the real precompiles only need constructing.
    // Base + a no-op invoke_signed are what let the ne_call wiring test run a direct
    // call to completion instead of tripping an unimplemented!().
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

    // (a) The fail-closed property: nothing is reachable by delegatecall unless exempted.
    #[test]
    fn default_denies_delegatecall_and_allows_direct() {
        let sel = &[0x11, 0x22, 0x33, 0x44];
        assert!(!delegatecall_refused(&direct(sel), &NoExempt));
        assert!(delegatecall_refused(&delegatecall(sel), &NoExempt));
    }

    // (b) The exemption mechanism keys on the selector, and only on the listed one.
    #[test]
    fn exemption_applies_only_to_the_listed_selector() {
        let p = OneExempt(&[0xaa, 0xbb, 0xcc, 0xdd]);
        assert!(!delegatecall_refused(
            &delegatecall(&[0xaa, 0xbb, 0xcc, 0xdd]),
            &p
        ));
        assert!(delegatecall_refused(
            &delegatecall(&[0xaa, 0xbb, 0xcc, 0xde]),
            &p
        ));
        assert!(!delegatecall_refused(
            &direct(&[0xaa, 0xbb, 0xcc, 0xde]),
            &p
        ));
    }

    // (d) A precompile that never implements delegatecall_exempt is gated automatically.
    #[test]
    fn trait_default_exempts_nothing() {
        for sel in [&[0u8; 4][..], &[0x11, 0x22, 0x33, 0x44][..], &[0xff; 4][..]] {
            assert!(!NoExempt.delegatecall_exempt(sel));
        }
    }

    // (e) A truncated selector refuses instead of panicking on the [..4] slice.
    #[test]
    fn short_input_refuses_without_panic() {
        for n in 0..4 {
            let sel = &[0xaa, 0xbb, 0xcc, 0xdd][..n];
            assert!(delegatecall_refused(
                &delegatecall(sel),
                &OneExempt(&[0xaa, 0xbb, 0xcc, 0xdd])
            ));
            assert!(delegatecall_refused(&delegatecall(sel), &NoExempt));
        }
    }

    // The gate keys on the first four bytes, not the whole calldata: every other
    // test here passes a bare selector, so a widened slice would go unnoticed.
    #[test]
    fn exemption_keys_on_the_selector_not_the_whole_calldata() {
        let s = Stub::new();
        let helper = HelperProgram::new(&s);

        let mut exempt = vec![0x5a, 0x7c, 0x32, 0x59]; // create_ata(address)
        exempt.extend_from_slice(&[0u8; 32]);
        assert!(!delegatecall_refused(&delegatecall(&exempt), &helper));

        let mut refused = vec![0xd7, 0x95, 0x52, 0x2b]; // mint_spl
        refused.extend_from_slice(&[0u8; 96]);
        assert!(delegatecall_refused(&delegatecall(&refused), &helper));
    }

    // ASplCached has no REFUSED row above; pin that its list is not unconditional.
    #[test]
    fn aspl_cached_exempts_only_its_own_selectors() {
        let s = Stub::new();
        let aspl = ASplCached::new(&s);
        assert!(!aspl.delegatecall_exempt(&[0u8; 4]));
        assert!(!aspl.delegatecall_exempt(&[0xd7, 0x95, 0x52, 0x2b]));
    }

    // CpiProgram's exemption is a 2-item allowlist (invoke, invoke_signed), not a
    // blanket pass. Guards against silently widening it to cover a future signing
    // selector or one of CpiProgram's own read selectors.
    #[test]
    fn cpi_exempts_only_the_two_invoke_selectors() {
        let s = Stub::new();
        let cpi = CpiProgram::new(&s);
        assert!(cpi.delegatecall_exempt(&[0x74, 0x80, 0xcb, 0x86])); // invoke
        assert!(cpi.delegatecall_exempt(&[0xb9, 0x4f, 0x37, 0x33])); // invoke_signed

        for sel in [
            [0u8, 0, 0, 0],
            [0xff, 0xff, 0xff, 0xff],
            [0xc1, 0x34, 0x65, 0xd9], // account_info — a real selector, but a read
        ] {
            assert!(!cpi.delegatecall_exempt(&sel), "{}: must not be exempt", hex::encode(sel));
        }
    }

    // (c) The real precompiles. Selector bytes are re-declared here on purpose: a
    // const changed in a precompile flips one of these instead of silently widening
    // the gate. EXEMPT = reachable by delegatecall unconditionally, EXEMPT_GUARDED =
    // reachable by delegatecall at the boundary but conditionally refused deeper
    // (cpi_ix.rs), REFUSED = never reachable by delegatecall.
    #[test]
    fn real_precompile_exemption_lists() {
        let s = Stub::new();
        let helper = HelperProgram::new(&s);
        let aspl = ASplCached::new(&s);
        let cpi = CpiProgram::new(&s);
        let spl = SplCached::new(&s);
        let system = SystemCached::new(&s);
        let withdraw = Withdraw::new(&s);
        let withdraw_cached = WithdrawCached::new(&s);

        // exempt: reachable by DELEGATECALL because nothing signs as context.caller
        let exempt: Vec<(&dyn Program, [u8; 4])> = vec![
            (&helper, [0x5a, 0x7c, 0x32, 0x59]), // create_ata(address)
            (&helper, [0x3d, 0xe2, 0x25, 0x1a]), // create_ata(address,bytes32)
            (&helper, [0xd2, 0x58, 0xa6, 0x9d]), // create_ata_for_key
            (&helper, [0xff, 0x35, 0x56, 0xca]), // create_pda(address)
            (&helper, [0x58, 0xe8, 0x82, 0x98]), // create_pda(address,uint64)
            (&helper, [0x4f, 0x75, 0xe9, 0x87]), // init_spl_mint
            (&helper, [0x20, 0x97, 0x2d, 0x0f]), // create_and_init_mint  [residual: caller seeds]
            (&helper, [0x6e, 0x3f, 0x24, 0xe0]), // swap_gas_to_lamports
            (&aspl, [0xb6, 0xd3, 0x36, 0xed]),   // create_ata()
            (&aspl, [0x81, 0x97, 0x2e, 0x35]),   // create_ata(bytes32)
            (&aspl, [0x5a, 0x7c, 0x32, 0x59]),   // create_ata(address)
            (&aspl, [0x3d, 0xe2, 0x25, 0x1a]),   // create_ata(address,bytes32)
        ];
        assert_eq!(exempt.len(), 12);
        for (p, sel) in exempt {
            let h = hex::encode(sel);
            assert!(p.delegatecall_exempt(&sel), "{h}: must be exempt");
            assert!(
                !delegatecall_refused(&delegatecall(&sel), p),
                "{h}: must pass the boundary under delegatecall"
            );
        }

        // exempt at the boundary, but NOT unconditionally safe like the list above:
        // invoke/invoke_signed pass the selector-only boundary check, then cpi_ix.rs
        // enforces the real, account-meta-dependent refusal (the bare external_auth
        // signer) internally. See non_evm::cpi_ix::tests for that enforcement.
        let exempt_guarded: Vec<(&dyn Program, [u8; 4])> = vec![
            (&cpi, [0x74, 0x80, 0xcb, 0x86]), // invoke
            (&cpi, [0xb9, 0x4f, 0x37, 0x33]), // invoke_signed
        ];
        assert_eq!(exempt_guarded.len(), 2);
        for (p, sel) in exempt_guarded {
            let h = hex::encode(sel);
            assert!(p.delegatecall_exempt(&sel), "{h}: must be boundary-exempt");
            assert!(
                !delegatecall_refused(&delegatecall(&sel), p),
                "{h}: must pass the boundary under delegatecall"
            );
        }

        // refused: every selector that signs as external_auth(context.caller)
        let refused: Vec<(&dyn Program, [u8; 4])> = vec![
            (&helper, [0x5f, 0xe7, 0x16, 0x65]),          // transfer_lamports
            (&helper, [0xb1, 0x2b, 0xe5, 0xba]),          // transfer_spl(address,uint64)
            (&helper, [0xba, 0x3a, 0x5e, 0xac]),          // transfer_spl(bytes32,uint64)
            (&helper, [0x53, 0xb5, 0x05, 0xe0]),          // transfer_spl(address,uint64,bytes32)
            (&helper, [0xb6, 0x97, 0x78, 0x79]),          // transfer_spl(bytes32,uint64,bytes32)
            (&helper, [0x76, 0x6b, 0x36, 0x2a]),          // transfer_spl_from_ata
            (&helper, [0xe4, 0x79, 0xdf, 0x56]),          // transfer_spl_from_to
            (&helper, [0x46, 0xef, 0xa6, 0x79]),          // transfer_spl_to_signer
            (&helper, [0xab, 0xf6, 0xf6, 0x75]),          // approve_spl
            (&helper, [0x78, 0x81, 0xd4, 0x53]),          // approve_spl_raw_delegate
            (&helper, [0xd7, 0x95, 0x52, 0x2b]),          // mint_spl
            (&helper, [0xe9, 0x7d, 0x32, 0x91]),          // create_mint_account
            (&helper, [0x44, 0x79, 0xb7, 0x09]),          // deposit_from_ata
            (&spl, [0xa9, 0x05, 0x9c, 0xbb]),             // transfer
            (&spl, [0x6a, 0x46, 0x73, 0x94]),             // transfer_b32
            (&spl, [0x57, 0xcf, 0xee, 0xee]),             // transfer_mint
            (&spl, [0x7d, 0xb5, 0x27, 0xf9]),             // transfer_mint_b32
            (&spl, [0x40, 0x1e, 0x33, 0x67]),             // transfer_from
            (&spl, [0x81, 0x80, 0xf2, 0xfc]),             // approve
            (&spl, [0x1e, 0x45, 0x8b, 0xee]),             // mint
            (&spl, [0x0b, 0x0a, 0xd5, 0x08]),             // init
            (&system, [0xe0, 0x40, 0x2a, 0x8d]),          // create_pda
            (&system, [0x4c, 0xea, 0xb6, 0x57]),          // create_pda_lamports
            (&system, [0x48, 0xe2, 0xbb, 0x86]),          // create_pda_lamports_salt
            (&system, [0xcc, 0x25, 0x8b, 0xbf]),          // create_pda_owner_len_salt
            (&system, [0x93, 0x22, 0x5c, 0x9f]),          // allocate
            (&system, [0x8a, 0xc0, 0x0b, 0xdc]),          // assign
            (&system, [0x5d, 0x35, 0x9f, 0xbd]),          // transfer
            (&system, [0xfd, 0x54, 0xd1, 0xea]),          // transfer_b32
            (&system, [0x87, 0x5a, 0xbf, 0xc0]),          // transfer_b32_salt
            (&withdraw, [0x4d, 0x8b, 0x0e, 0xa4]),        // withdrawal_id
            (&withdraw, [0x7f, 0x31, 0x24, 0xa0]),        // withdraw_to_pda
            (&withdraw, [0x80, 0x59, 0xab, 0xc0]),        // withdraw_to_ata
            (&withdraw_cached, [0x4d, 0x8b, 0x0e, 0xa4]), // withdrawal_id
            (&withdraw_cached, [0x7f, 0x31, 0x24, 0xa0]), // withdraw_to_pda
            (&withdraw_cached, [0x80, 0x59, 0xab, 0xc0]), // withdraw_to_ata
            (&withdraw_cached, [0xb6, 0xb5, 0x5f, 0x25]), // deposit
        ];
        assert_eq!(refused.len(), 37);
        for (p, sel) in refused {
            let h = hex::encode(sel);
            assert!(!p.delegatecall_exempt(&sel), "{h}: must not be exempt");
            assert!(
                delegatecall_refused(&delegatecall(&sel), p),
                "{h}: must be refused under delegatecall"
            );
            assert!(
                !delegatecall_refused(&direct(&sel), p),
                "{h}: must still be reachable by a direct call"
            );
        }
    }

    // The boundary is WIRED: same program, same selector, same ix — only context.address
    // differs, and that alone decides whether ne_call commits or refuses. Deleting the
    // call site in ne_call fails this test (the predicate unit tests would not).
    #[test]
    fn ne_call_consults_the_boundary() {
        let s = Stub::new();
        let exempt: &'static [u8] = &[0x11, 0x22, 0x33, 0x44];
        let gated = &[0x55, 0x66, 0x77, 0x88];

        fn js(s: &Stub) -> JournaledState<'_, Stub> {
            JournaledState {
                state: s,
                journal: Journal::new(),
                mutable: true,
                timestamp: 0,
                slot: 0,
                origin: None,
                gas_limit: None,
                gas_price: None,
                gas_recipient: None,
                merged_slots: BTreeMap::new(),
                found_cpi: false,
                found_cpi_cached: false,
            }
        }

        // direct call: not refused, runs through to the invoke
        assert!(js(&s)
            .ne_call(Box::new(Mutating(exempt)), direct(gated), true)
            .is_ok());

        // delegatecall, selector not exempt: refused at the boundary
        assert!(matches!(
            js(&s).ne_call(Box::new(Mutating(exempt)), delegatecall(gated), true),
            Err(RomeProgramError::DelegatecallOwnerAuthority)
        ));

        // delegatecall, selector exempt: allowed through
        assert!(js(&s)
            .ne_call(Box::new(Mutating(exempt)), delegatecall(exempt), true)
            .is_ok());
    }
}
