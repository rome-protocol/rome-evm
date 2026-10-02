use {evm::H160, crate::msg, super::impl_contract};

impl_contract!(Identity, [0_u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4,]);

fn contract(input: &[u8]) -> Result<Vec<u8>> {
    msg!("identity");

    Ok(input.to_vec())
}
