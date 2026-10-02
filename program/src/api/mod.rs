pub mod deposit;
mod do_tx;
pub mod do_tx_holder;
pub mod do_tx_holder_iterative;
pub mod do_tx_iterative;
pub mod reg_owner;
pub mod transmit_tx;
pub mod create_treasure;
pub mod join_treasure;
pub mod alloc_resource;
pub mod dealloc_resource;
pub mod settle_inbound_bridge_v2;
mod do_tx_unsigned;
mod activate_ata;

use solana_program::pubkey::Pubkey;
pub use deposit::deposit;
pub use do_tx::do_tx;
pub use do_tx_holder::do_tx_holder;
pub use do_tx_holder_iterative::do_tx_holder_iterative;
pub use do_tx_iterative::do_tx_iterative;
pub use reg_owner::reg_owner;
pub use transmit_tx::transmit_tx;
pub use create_treasure::create_treasure;
pub use join_treasure::join_treasure;
pub use alloc_resource::alloc_resource;
pub use dealloc_resource::dealloc_resource;
pub use do_tx_unsigned::do_tx_unsigned;
pub use activate_ata::activate_ata;
pub use settle_inbound_bridge_v2::settle_inbound_bridge_v2;


use {
    crate::{
        error::{Result, RomeProgramError::InvalidInstructionData},
        H160, H256,
    },
    std::convert::{TryInto, TryFrom,},
    std::mem::size_of,
};

pub fn split_u64(data: &[u8]) -> Result<(u64, &[u8])> {
    if data.len() < size_of::<u64>() {
        return Err(InvalidInstructionData);
    }

    let (left, right) = data.split_at(size_of::<u64>());
    let value = u64::from_le_bytes(left.try_into().unwrap());

    Ok((value, right))
}
pub fn split_hash(data: &[u8]) -> Result<(H256, &[u8])> {
    if data.len() < size_of::<H256>() {
        return Err(InvalidInstructionData);
    }

    let (left, right) = data.split_at(size_of::<H256>());
    let hash = H256::from_slice(left);

    Ok((hash, right))
}
// pay_fee: u8 | [H160 iff pay_fee==1] | pri_fee: u64 LE | rest
//
// pri_fee is the operator-attested, already-ceil-divided priority-fee
// reservation (lamports) for this tx. It replaces the on-chain
// ComputeBudget sibling-walk (#502) — the caller composes it from the same
// (cu_limit, cu_price) it puts in the v1 TransactionConfig, so ix data and
// the tx's actual priority fee are single-sourced off-chain. MANDATORY
// field: an unpriced tx must author an explicit 0, never omit it —
// truncated data fails closed with `InvalidInstructionData`.
pub fn split_fee(data: &[u8]) -> Result<(Option<H160>, u64, &[u8])> {
    let (pay_fee, right) = split_u8(data)?;

    let (fee_addr, right) = if pay_fee == 0 {
        (None, right)
    } else {
        if right.len() < size_of::<H160>() {
            return Err(InvalidInstructionData);
        }

        let (left, right) = right.split_at(size_of::<H160>());
        (Some(H160::from_slice(left)), right)
    };

    let (pri_fee, right) = split_u64(right)?;

    Ok((fee_addr, pri_fee, right))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_fee_parses_priority() {
        let mut data = vec![1u8];
        data.extend_from_slice(&[0xAAu8; 20]);
        data.extend_from_slice(&12345u64.to_le_bytes());
        data.extend_from_slice(b"rlp");

        let (fee_addr, pri_fee, rest) = split_fee(&data).unwrap();

        assert_eq!(fee_addr, Some(H160::from_slice(&[0xAAu8; 20])));
        assert_eq!(pri_fee, 12345);
        assert_eq!(rest, b"rlp");
    }

    #[test]
    fn split_fee_priced_zero() {
        let mut data = vec![0u8];
        data.extend_from_slice(&0u64.to_le_bytes());
        data.extend_from_slice(b"rlp");

        let (fee_addr, pri_fee, rest) = split_fee(&data).unwrap();

        assert_eq!(fee_addr, None);
        assert_eq!(pri_fee, 0);
        assert_eq!(rest, b"rlp");
    }

    #[test]
    fn split_fee_truncated_priority_is_invalid() {
        let data = vec![0u8]; // pay_fee only, no pri_fee — must fail closed
        assert!(matches!(split_fee(&data), Err(InvalidInstructionData)));
    }
}
pub fn split_u8(data: &[u8]) -> Result<(u8, &[u8])> {
    if data.len() < size_of::<u8>() {
        return Err(InvalidInstructionData);
    }

    Ok((data[0], &data[1..]))
}

pub fn split_pubkey_opt(data: &[u8]) -> Result<Option<Pubkey>> {
    let opt = if data.len() > 0 {
        if data.len() != size_of::<Pubkey>() {
            return Err(InvalidInstructionData);
        }
        Some(Pubkey::try_from(data).unwrap())
    } else {
        None
    };

    Ok(opt)
}
pub fn split_pubkey(data: &[u8]) -> Result<(Pubkey, &[u8])> {
    if data.len() < size_of::<Pubkey>() {
        return Err(InvalidInstructionData);
    }
    let (left, right) = data.split_at(size_of::<Pubkey>());
    let key = Pubkey::try_from(left).unwrap();

    Ok((key, right))
}

pub fn split_h160(data: &[u8]) -> Result<(H160, &[u8])> {
    if data.len() < size_of::<H160>() {
        return Err(InvalidInstructionData);
    }
    let (left, right) = data.split_at(size_of::<H160>());
    let addr = H160::from_slice(left);
    Ok((addr, right))
}
