use solana_client::rpc_client::RpcClient;
use solana_program::pubkey::Pubkey;
use solana_account::Account;
use rome_evm::{
    error::Result,
};
pub trait AccountStorage: Send + Sync {
    fn get_account(
        &self,
        pubkey: &Pubkey,
    ) -> Result<Option<Account>>;

    fn get_multiple_accounts(
        &self, pubkeys: &[Pubkey]
    ) -> Result<Vec<Option<Account>>>;
}

impl AccountStorage for RpcClient {
    fn get_account(&self, pubkey: &Pubkey) -> Result<Option<Account>> {
        Ok(self.get_account_with_commitment(pubkey, self.commitment())?.value)
    }

    fn get_multiple_accounts(&self, pubkeys: &[Pubkey]) -> Result<Vec<Option<Account>>> {
        Ok(self.get_multiple_accounts(pubkeys)?)
    }
}
