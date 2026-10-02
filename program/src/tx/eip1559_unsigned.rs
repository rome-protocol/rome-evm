use {
    super::{eip2930::AccessList, fix, Base},
    crate::{
        error::Result,
        tx::{check_rlp, decode_to},
    },
    evm::{H160, H256, U256},
    rlp::Rlp,
    solana_program::keccak::hashv,
};

pub const EIP1559UNSIGNED_LEN: usize = 9;      // number of fields of unsigned rlp

#[derive(Debug, Clone)]
pub struct Eip1559unsigned {
    pub chain_id: U256,
    pub nonce: u64,
    pub max_priority_fee_per_gas: U256,
    #[allow(dead_code)]
    pub max_fee_per_gas: U256,
    pub gas_limit: U256,
    pub to: Option<H160>,
    pub value: U256,
    pub data: Option<Vec<u8>>,
    #[allow(dead_code)]
    pub access_list: AccessList,
    pub from: H160,
}

impl Base for Eip1559unsigned {
    fn nonce(&self) -> u64 {
        self.nonce
    }
    fn to(&self) -> Option<H160> {
        self.to
    }
    fn value(&self) -> U256 {
        self.value
    }
    fn data(&mut self) -> Option<Vec<u8>> {
        self.data.take()
    }
    fn gas_limit(&self) -> U256 {
        self.gas_limit
    }
    fn gas_price(&self) -> U256 {
        let f = |a: U256| {
            if a.is_zero() {
                None
            } else {
                Some(a)
            }
        };

        match (f(self.max_fee_per_gas), f(self.max_priority_fee_per_gas)) {
            (Some(max_fee), Some(_)) => max_fee,
            // this also covers the None, None case
            (None, prio_fee) => prio_fee.unwrap_or(U256::zero()),
            (max_fee, None) => max_fee.unwrap(),
        }
    }
    fn hash_unsign(&self, rlp: &Rlp) -> Result<H256> {
        Ok(H256::from(hashv(&[&[2], rlp.as_raw()]).to_bytes()))
    }
    fn rs(&self) -> (U256, U256) {
        unreachable!()
    }
    fn recovery_id(&self) -> Result<u8> {
        unreachable!()
    }
    fn chain_id(&self) -> u64 {
        self.chain_id.as_u64()
    }
    fn from(&self) -> H160 {
        self.from
    }
    fn set_from(&mut self, from: H160) {
        self.from = from;
    }
    #[cfg(test)]
    fn access_list(&self) -> Option<&AccessList> {
        Some(&self.access_list)
    }
    fn mint(&self) -> U256 {
        unreachable!()
    }
}
impl Eip1559unsigned {
    pub fn rlp_at_chain_id(rlp: &rlp::Rlp) -> Result<U256> {
        let chain = fix(rlp, 0)?;
        Ok(chain)
    }
    pub fn from_rlp(rlp: &rlp::Rlp) -> Result<Self> {
        check_rlp(rlp, EIP1559UNSIGNED_LEN)?;

        let chain_id = Eip1559unsigned::rlp_at_chain_id(rlp)?;
        let nonce: u64 = rlp.val_at(1)?;
        let max_priority_fee_per_gas = fix(rlp, 2)?;
        let max_fee_per_gas = fix(rlp, 3)?;
        let gas_limit = fix(rlp, 4)?;
        let to = decode_to(rlp, 5)?;
        let value = fix(rlp, 6)?;
        let data = rlp.val_at(7)?;
        let access_list = rlp.val_at(8)?;

        Ok(Eip1559unsigned {
            chain_id,
            nonce,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit,
            to,
            value,
            data: Some(data),
            access_list,
            from: H160::default(),
        })
    }
}
