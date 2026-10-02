use {
    crate::{
        context::ContextAt,
        error::Result,
        split_fee,
        state::State,
        tx::tx::Tx,
        vm::{vm_atomic::MachineAt, Execute, VmAt}, msg,
    },
    solana_program::{account_info::AccountInfo, pubkey::Pubkey,},
};

pub fn do_tx<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    msg!("Instruction: Atomic transaction");

    let (fee_addr, pri_fee, rlp) = split_fee(data)?;
    let chain = Tx::chain_id_from_rlp(rlp)?;
    let state = State::new(program_id, accounts, chain)?;
    let context = ContextAt::new(&state);
    let mut vm = VmAt::new(&state, rlp, fee_addr, pri_fee, &context)?;

    vm.consume(MachineAt::Lock)
}
