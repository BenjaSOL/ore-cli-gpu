use solana_sdk::signature::Signer;

use crate::{utils::proof_pubkey, Miner};

impl Miner {
    pub async fn register(&self) {
        // Return early if miner is already registered
        let signer = self.signer();
        let proof_address = proof_pubkey(signer.pubkey());
        let client = self.rpc_client.clone();
        if client.get_account(&proof_address).await.is_ok() {
            return;
        }

        // Sign and send transaction.
        println!("Generating challenge...");
        'send: loop {
            let ix = ore::instruction::register(signer.pubkey());
            if self.send_and_confirm(&[ix], true, false).await.is_ok() {
                break 'send;
            }
        }
    }
}
