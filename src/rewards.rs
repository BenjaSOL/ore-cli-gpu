use std::str::FromStr;

use solana_program::pubkey::Pubkey;
use solana_sdk::signature::Signer;

use crate::{utils::get_proof, Miner};

impl Miner {
    pub async fn rewards(&self, address: Option<String>) {
        let address = if let Some(address) = address {
            if let Ok(address) = Pubkey::from_str(&address) {
                address
            } else {
                println!("Invalid address: {:?}", address);
                return;
            }
        } else {
            self.signer().pubkey()
        };
        let proof = get_proof(&self.rpc_client, address).await;
        let amount = (proof.claimable_rewards as f64) / 10f64.powf(ore::TOKEN_DECIMALS as f64);
        println!("{:} ORE", amount);
    }
}
