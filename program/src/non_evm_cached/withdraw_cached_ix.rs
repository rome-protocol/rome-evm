use {
    solana_program::{
        instruction::{Instruction,},  pubkey::Pubkey, rent::Rent, sysvar::Sysvar,
    },
    solana_system_interface::{
        program as system_program, instruction as system_instruction,
    },
    crate::{
        H160, pda::Seed, error::{Result, RomeProgramError::*}, origin::Origin,
        state::{Diff, aux::checked_transfer}, SOL_DECIMALS, wei_to_u64, handler::CallInterrupt,
        non_evm::{
            IxList, aux::{get_pubkey, get_u256,}, Withdraw,
        },
        non_evm_cached::{
            WithdrawCached, CachedState,
        },
        NonEvmState,
    },
    evm::U256,
    spl_associated_token_account_interface::{
        instruction::create_associated_token_account_idempotent,
        address::get_associated_token_address_with_program_id as ata
    },
};

pub struct WithdrawCachedReader<'a, T: Origin> {
    pub cached: CachedState<'a, T>,
}

impl<'a, T: Origin> WithdrawCachedReader<'a, T> {
    pub fn new(state: &'a T, ne_state: Option<&'a NonEvmState>) -> Self {
        Self {
            cached: CachedState::new(state, ne_state)
        }
    }

    pub fn withdraw(&self, abi: &[u8], params: &CallInterrupt) -> Result<IxList> {
        let (to, _) = get_pubkey(abi)?;

        let wei = if let Some(t) = params.transfer {
            t.value
        } else {
            let list = IxList{
                ixs: vec![],
                diff: vec![],
            };
            return Ok(list)
        };

        // check that "to" is a system account (not spl_associated_account) and it exists
        let to_acc = self.cached.account(&to)?;
        // TODO: check data.len() == 0
        if to_acc.owner != system_program::ID || to_acc.lamports == 0 {
            return Err(InvalidWithdrawalAccount(to))
        }

        let (wallet, seed) = self.cached.base().pda.sol_wallet();

        let ixs = if let Some(mint) = self.cached.base().owner_info().mint {
            self.spl_transfer(&to, &mint, wei, &wallet, seed)?
        } else {
            let lamports = wei_to_u64(wei, SOL_DECIMALS)?;
            let ix = system_instruction::transfer(&wallet, &to, lamports);
            vec![(ix, vec![seed])]
        };

        self.burn(ixs, wei, params)
    }

    pub fn withdraw_to_pda(&self, abi: &[u8], params: &CallInterrupt) -> Result<IxList> {
        if self.cached.base().owner_info().mint.is_some() {
            return Err(NonEvmCallError("withdraw_to_pda() call is only applicable for rollup based on SOL".to_string()));
        };

        let wei = get_u256(abi)?;
        let lamports = wei_to_u64(wei, SOL_DECIMALS)?;
        let (wallet, seed) = self.cached.base().pda.sol_wallet();
        let (to, _) = self.cached.base().pda.external_auth(&params.context.caller);
        let to_acc =  self.cached.account(&to)?;

        if to_acc.lamports == 0 {
            let rent = Rent::get()?.minimum_balance(0);
            if lamports < rent {
                return Err(NonEvmCallError(format!("withdraw_to_pda(): number of lamports less than rent exempt {}", lamports)))
            }
        }

        let ix = system_instruction::transfer(&wallet, &to, lamports);
        let ixs = vec![(ix, vec![seed])];
        self.burn(ixs, wei, params)
    }

    pub fn withdraw_to_ata(&self, abi: &[u8], params: &CallInterrupt) -> Result<IxList> {
        let mint = if let Some(mint_) = self.cached.base().owner_info().mint {
            mint_
        } else {
            return Err(NonEvmCallError("withdraw_to_ata() call is only applicable for rollup based on SPL".to_string()));
        };

        let wei = get_u256(abi)?;
        let (wallet, seed) = self.cached.base().pda.sol_wallet();
        let (to, _) = self.cached.base().pda.external_auth(&params.context.caller);
        let ixs = self.spl_transfer(&to, &mint, wei, &wallet, seed)?;
        self.burn(ixs, wei, params)
    }

    // Deposit: inverse of `withdrawal`. SPL transfer from caller's
    // PDA-owned ATA to the chain's sol_wallet ATA on Solana side; mint
    // `wei` native gas to caller on EVM side. The chain's SPL wallet —
    // not `Withdraw::ADDRESS` — is the economic backing of native gas
    // in circulation, so the EVM-side credit is a pure mint with no
    // offsetting debit (matching `HelperProgram.deposit_from_ata`
    // semantics). Used by rome-ui useWrapUnwrap (unwrap leg) and future
    // SPL → native-gas user flows on SPL-based rollups.
    pub fn deposit(&self, abi: &[u8], params: &CallInterrupt) -> Result<IxList> {
        let mint = if let Some(mint_) = self.cached.base().owner_info().mint {
            mint_
        } else {
            return Err(NonEvmCallError("deposit() call is only applicable for rollup based on SPL".to_string()));
        };

        let wei = get_u256(abi)?;
        let p = self.cached.mint_profile(&mint)?;
        let (spl_program, decimals) = (p.program, p.decimals);
        let tokens = wei_to_u64(wei, decimals)?;
        let (wallet, _) = self.cached.base().pda.sol_wallet();
        let (from, seed) = self.cached.base().pda.external_auth(&params.context.caller);
        let from_ata = ata(&from, &mint, &spl_program);
        let to_ata = ata(&wallet, &mint, &spl_program);

        let ix = checked_transfer(&p, &mint, &from_ata, &to_ata, &from, &[], tokens)?;

        let ixs = vec![(ix, vec![seed])];
        let diff = vec![(params.context.caller, Diff::TransferTo { balance: wei })];

        Ok(IxList { ixs, diff })
    }

    fn burn(&self, ixs: Vec<(Instruction, Vec<Seed>)>, wei: U256, params: &CallInterrupt) -> Result<IxList> {
        let diff = WithdrawCachedReader::<'a, T>::rsol_transfer(params, wei);

        let list = IxList{
            ixs,
            diff,
        };

        Ok(list)
    }

    // every withdraw leg is direct-only: the ne_call boundary exempts no selector here
    fn rsol_transfer(params: &CallInterrupt, wei: U256) -> Vec<(H160, Diff)>{
        assert_eq!(params.code_address, WithdrawCached::<'a, T>::ADDRESS);
        let mut vec = vec![];
        vec.push((params.context.caller, Diff::TransferFrom {balance: wei}));
        // Fund are moved to 0x42..016 address
        vec.push((Withdraw::<'a, T>::ADDRESS, Diff::TransferTo {balance: wei}));

        vec
    }
    fn spl_transfer<'b>(
        &self,
        to: &Pubkey,
        mint: &Pubkey,
        wei: U256,
        wallet: &Pubkey,
        seed: Seed,
    ) -> Result<Vec<(Instruction, Vec<Seed>)>> {
        let mut  ixs = vec![];
        let p = self.cached.mint_profile(mint)?;
        let (spl_program, decimals) = (p.program, p.decimals);

        let tokens = wei_to_u64(wei, decimals)?;
        let from_ata = ata(wallet, mint, &spl_program);
        let to_ata= ata(to, mint, &spl_program);
        let to_ata_acc = self.cached.account(&to_ata)?;

        if to_ata_acc.owner == system_program::ID {
            let ix = create_associated_token_account_idempotent(&self.cached.signer(), to, mint, &spl_program);
            ixs.push((ix, vec![]))
        }

        let ix = checked_transfer(&p, mint, &from_ata, &to_ata, wallet, &[], tokens)?;
        ixs.push((ix, vec![seed]));

        Ok(ixs)
    }
}



