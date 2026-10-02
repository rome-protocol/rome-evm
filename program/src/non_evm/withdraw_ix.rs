use {
    solana_program::{
        instruction::{Instruction,},  pubkey::Pubkey, rent::Rent, sysvar::Sysvar,
    },
    solana_system_interface::{
        program as system_program, instruction as system_instruction,
    },
    crate::{
        H160, pda::Seed, error::{Result, RomeProgramError::*}, origin::Origin,
        state::Diff, SOL_DECIMALS, wei_to_u64, handler::CallInterrupt,
        state::aux::{checked_transfer, mint_profile},
    },
    super::{
        IxList, aux::{get_pubkey, get_u256,},
    },
    evm::U256,
    spl_associated_token_account_interface::{
        instruction::create_associated_token_account,
        address::get_associated_token_address_with_program_id as ata
    },
};

pub struct Withdraw<'a, T: Origin> {
    pub state: &'a T,
}
impl<'a, T: Origin> Withdraw<'a, T> {
    pub const ADDRESS: H160 = H160([
        0x42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x16,
    ]);
    pub fn new(state: &'a T) -> Self {
        Self {
            state
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
        let to_acc = self.state.account(&to)?;
        // TODO: check data.len() == 0
        if to_acc.owner != system_program::ID || to_acc.lamports == 0 {
            return Err(InvalidWithdrawalAccount(to))
        }

        let (wallet, seed) = self.state.base().pda.sol_wallet();

        let ixs = if let Some(mint) = self.state.base().owner_info().mint {
            self.spl_transfer(&to, &mint, wei, &wallet, seed)?
        } else {
            let lamports = wei_to_u64(wei, SOL_DECIMALS)?;
            let ix = system_instruction::transfer(&wallet, &to, lamports);
            vec![(ix, vec![seed])]
        };

        self.burn(ixs, wei, params)
    }

    pub fn withdraw_to_pda(&self, abi: &[u8], params: &CallInterrupt) -> Result<IxList> {
        if self.state.base().owner_info().mint.is_some() {
            return Err(NonEvmCallError("withdraw_to_pda() call is only applicable for rollup based on SOL".to_string()));
        };

        let wei = get_u256(abi)?;
        let lamports = wei_to_u64(wei, SOL_DECIMALS)?;
        let (wallet, seed) = self.state.base().pda.sol_wallet();
        let (to, _) = self.state.base().pda.external_auth(&params.context.caller);
        let to_acc =  self.state.account(&to)?;

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
        let mint = if let Some(mint_) = self.state.base().owner_info().mint {
            mint_
        } else {
            return Err(NonEvmCallError("withdraw_to_ata() call is only applicable for rollup based on SPL".to_string()));
        };

        let wei = get_u256(abi)?;
        let (wallet, seed) = self.state.base().pda.sol_wallet();
        let (to, _) = self.state.base().pda.external_auth(&params.context.caller);
        let ixs = self.spl_transfer(&to, &mint, wei, &wallet, seed)?;
        self.burn(ixs, wei, params)
    }

    fn burn(&self, ixs: Vec<(Instruction, Vec<Seed>)>, wei: U256, params: &CallInterrupt) -> Result<IxList> {
        let diff = Withdraw::<'a, T>::rsol_transfer(params, wei);

        let list = IxList{
            ixs,
            diff,
        };

        Ok(list)
    }

    // every withdraw leg is direct-only: the ne_call boundary exempts no selector here
    fn rsol_transfer(params: &CallInterrupt, wei: U256) -> Vec<(H160, Diff)>{
        assert_eq!(params.code_address, Withdraw::<'a, T>::ADDRESS);
        let mut vec = vec![];
        vec.push((params.context.caller, Diff::TransferFrom {balance: wei}));
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
        let profile = mint_profile(self.state, mint)?;
        let (spl_program, decimals) = (profile.program, profile.decimals);

        let tokens = wei_to_u64(wei, decimals)?;
        let from_ata = ata(wallet, mint, &spl_program);
        let to_ata= ata(to, mint, &spl_program);
        let to_ata_acc = self.state.account(&to_ata)?;

        if to_ata_acc.owner == system_program::ID {
            let ix = create_associated_token_account(&self.state.signer(), to, mint, &spl_program);
            ixs.push((ix, vec![]))
        }

        let ix = checked_transfer(&profile, mint, &from_ata, &to_ata, wallet, &[], tokens)?;
        ixs.push((ix, vec![seed]));

        Ok(ixs)
    }
}



