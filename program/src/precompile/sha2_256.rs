use {evm::H160, solana_program::hash::hash, crate::msg, super::impl_contract,};

impl_contract!(Sha2, [0_u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2,]);

fn contract(input: &[u8]) -> Result<Vec<u8>> {
    msg!("sha2_256");

    Ok(Vec::from(hash(input).as_ref()))
}
