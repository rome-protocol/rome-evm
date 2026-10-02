use {
    crate::{
        context::ContextAt,
        error::Result,
        state::State,
        vm::{vm_atomic::MachineAt, Execute, VmAt}, msg,
        tx::{
            eip1559_unsigned::Eip1559unsigned, Base, legacy::Legacy,
        },
        aux::derive_sender, H256,
    },
    rlp::Rlp,
    solana_program::{
        account_info::AccountInfo, pubkey::Pubkey, sysvar::Sysvar,
        clock::Clock,
    },
};

pub fn do_tx_unsigned<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    data: &'a [u8],
) -> Result<()> {
    msg!("Instruction: Atomic unsigned transaction");

    let rlp = Rlp::new(data);
    let chain = Eip1559unsigned::rlp_at_chain_id(&rlp)?.as_u64();
    let state = State::new(program_id, accounts, chain)?;
    let tx = Eip1559unsigned::from_rlp(&rlp)?;

    let legacy = Legacy {
        from: derive_sender(&state.signer.key),
        to: tx.to,
        chain_id: chain.into(),
        gas_price: tx.gas_price(),
        data: tx.data,
        nonce: tx.nonce,
        gas_limit: tx.gas_limit,
        value: tx.value,
        ..Default::default()
    };

    let clock = Clock::get()?;
    let mut hash = H256::zero();
    hash.0[24..].copy_from_slice(&clock.slot.to_le_bytes());
    let context = ContextAt::new(&state);
    // Out of scope for #502: DoTxUnsigned is Solana-native, the v1 config
    // already charges priority natively — pri_fee stays 0 here or it would
    // double-charge.
    let mut vm = VmAt::new_with_unsigned_tx(&state, legacy, hash, &context, None, 0)?;

    vm.consume(MachineAt::Lock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evm::H160;

    // Pins the Solana-pubkey -> EVM-address derivation. This address is where
    // a Solana user's EVM balance/nonce lives; changing the derivation would
    // strand every existing user's funds. Expected value computed independently:
    //   keccak256(0x01 * 32)[12..32]
    #[test]
    fn derive_sender_is_pinned() {
        let key = Pubkey::new_from_array([1u8; 32]);
        let expected = H160::from_slice(&[
            0xb3, 0x12, 0xbe, 0xc0, 0x18, 0x88, 0x4c, 0x2d, 0x66, 0x66,
            0x7c, 0x67, 0xa9, 0x05, 0x08, 0x21, 0x4b, 0xd8, 0xba, 0xfc,
        ]);
        assert_eq!(derive_sender(&key), expected);
    }
}
