use ore::TREASURY_ADDRESS;

use solana_sdk::signature::Signer;

use crate::Miner;

impl Miner {
    pub async fn initialize(&self) {
        // Return early if program is initialized
        let signer = self.signer();
        let client = self.rpc_client.clone();
        if client.get_account(&TREASURY_ADDRESS).await.is_ok() {
            return;
        }

        // Sign and send transaction.
        let ix = ore::instruction::initialize(signer.pubkey());
        self.send_and_confirm(&[ix], false, false)
            .await
            .expect("Transaction failed");
    }
}
