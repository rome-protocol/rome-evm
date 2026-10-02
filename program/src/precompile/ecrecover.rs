use {
    crate::tx::tx::Tx,
    evm::{H160, U256},
    crate::msg,
    std::{cmp::Ordering::*, convert::TryInto},
    super::impl_contract,
};

impl_contract!(Ecrecover, [0_u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,]);

fn contract(input: &[u8]) -> Result<Vec<u8>> {
    msg!("ecrecover");

    let len = input.len();

    let data: [u8; 128] = match len.cmp(&128) {
        Less => {
            let mut data = [0_u8; 128];
            data[..len].copy_from_slice(input);
            data
        }
        Equal | Greater => input[..128].try_into().unwrap(),
    };

    let (hash, right) = data.split_at(32);
    let (v, rs) = right.split_at(32);

    let v = U256::from_big_endian(v);

    if v != 27.into() && v != 28.into() {
        return Ok(vec![]);
    }

    let v = v.as_u64() as u8;

    if let Ok(mut pub_key) = Tx::syscall(hash, v - 27, rs) {
        pub_key[0..12].fill(0);
        Ok(pub_key.to_vec())
    } else {
        Ok(vec![])
    }
}
