use {
    crate::{
        context::ContextIt,
        error::{Result, RomeProgramError::InvalidInstructionData,},
        split_fee, split_hash, split_u64,
        state::State,
        vm::{vm_iterative::MachineIt::FromStateHolder, Execute, VmIt},
        Holder, msg,
        api::do_tx_iterative::verify_opcode_limit,
    },
    evm::{H160, H256},
    solana_program::{account_info::AccountInfo, pubkey::Pubkey},
};

// unique | session | holder_index | limit | tx_hash | chain_id | Option<fee_recipient> | pri_fee
#[allow(clippy::type_complexity)]
pub fn args(data: &[u8]) -> Result<(u64, u64, u64, H256, u64, Option<H160>, u64)> {
    let (_, data) = split_u64(data)?;
    let (session, data) = split_u64(data)?;
    let (holder, data) = split_u64(data)?;
    let (limit, data) = split_u64(data)?;
    let (hash, data) = split_hash(data)?;
    let (chain, data) = split_u64(data)?;
    let (fee_addr, pri_fee, right) = split_fee(data)?;
    if !right.is_empty() {
        return Err(InvalidInstructionData)
    }

    Ok((session, holder, limit, hash, chain, fee_addr, pri_fee))
}

pub fn do_tx_holder_iterative<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    msg!("Instruction: Iterative transaction from holder");

    let (session, holder, limit, hash, chain, fee_addr, pri_fee) = args(data)?;
    verify_opcode_limit(limit)?;
    let state = State::new(program_id, accounts, chain)?;

    let tx_holder = state.info_tx_holder(holder, false)?;
    let rlp = Holder::rlp(tx_holder, hash, chain)?;

    let state_holder = state.info_state_holder(holder, true)?;

    let context = ContextIt::new(
        &state,
        &rlp,
        hash,
        session,
        fee_addr,
        pri_fee,
        Some(tx_holder),
        state_holder,
    )?;
    let mut vm = VmIt::new(&state, &context, limit)?;
    vm.consume(FromStateHolder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OPCODE_LIMIT_MIN;

    // unique | session | holder | limit | tx_hash | chain_id | pay_fee(0) | pri_fee
    fn build_args_data(limit: u64) -> Vec<u8> {
        let mut data = vec![];
        data.extend_from_slice(&0u64.to_le_bytes()); // unique
        data.extend_from_slice(&0u64.to_le_bytes()); // session
        data.extend_from_slice(&0u64.to_le_bytes()); // holder
        data.extend_from_slice(&limit.to_le_bytes()); // limit
        data.extend_from_slice(&[0u8; 32]); // tx_hash
        data.extend_from_slice(&0u64.to_le_bytes()); // chain_id
        data.push(0u8); // pay_fee = 0 -> no fee_addr
        data.extend_from_slice(&0u64.to_le_bytes()); // pri_fee
        data
    }

    /// Parity with do_tx_iterative: the holder-iterative entry point enforces
    /// the same OPCODE_LIMIT_MIN floor via the same verify_opcode_limit, over its
    /// own (differently-shaped) ix-data buffer. limit=0 parses fine; it's
    /// verify_opcode_limit, called right after args() and before any lock/state
    /// touch, that refuses it.
    #[test]
    fn zero_limit_parses_then_is_rejected_by_verify_opcode_limit() {
        let zero_limit_data = build_args_data(0);
        let (_, _, limit, _, _, _, _) = args(&zero_limit_data).unwrap();
        assert_eq!(limit, 0);
        assert!(verify_opcode_limit(limit).is_err());

        let legit_data = build_args_data(OPCODE_LIMIT_MIN);
        let (_, _, limit, _, _, _, _) = args(&legit_data).unwrap();
        assert!(verify_opcode_limit(limit).is_ok());
    }
}
