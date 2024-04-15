use std::{
    io::{stdout, Write,BufRead},
    sync::{atomic::AtomicBool, Arc, Mutex},
    mem
};
use std::process::Command;
use std::str::FromStr;
use bs58;
use hex;
use ore::{self, state::Bus, BUS_ADDRESSES, BUS_COUNT, EPOCH_DURATION};
use rand::Rng;
use solana_program::{keccak::HASH_BYTES, program_memory::sol_memcmp, pubkey::Pubkey};
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    keccak::{hashv, Hash as KeccakHash},
    signature::Signer,
};
use crate::{
    cu_limits::{CU_LIMIT_MINE, CU_LIMIT_RESET},
    utils::{get_clock_account, get_proof, get_treasury},
    Miner,
};
use base64::{encode};
use tokio::io::{AsyncWriteExt, BufReader, AsyncBufReadExt};
// Odds of being selected to submit a reset tx
const RESET_ODDS: u64 = 20;

impl Miner {
    pub async fn mine(&self, threads: u64) {
        // Register, if needed.
        let signer = self.signer();
        self.register().await;
        let mut stdout = stdout();
        let mut rng = rand::thread_rng();

        // Start mining loop
        loop {
            // Fetch account state
            let balance = self.get_ore_display_balance().await;
            let treasury = get_treasury(&self.rpc_client).await;
            let proof = get_proof(&self.rpc_client, signer.pubkey()).await;
            let rewards =
                (proof.claimable_rewards as f64) / (10f64.powf(ore::TOKEN_DECIMALS as f64));
            let reward_rate =
                (treasury.reward_rate as f64) / (10f64.powf(ore::TOKEN_DECIMALS as f64));
            stdout.write_all(b"\x1b[2J\x1b[3J\x1b[H").ok();
            println!("Balance: {} ORE", balance);
            println!("Claimable: {} ORE", rewards);
            println!("Reward rate: {} ORE", reward_rate);

            // Escape sequence that clears the screen and the scrollback buffer
            println!("\nMining for a valid hash...");
            println!("Calling find_next_hash_par");
            let hash_and_pubkey = [(solana_sdk::keccak::Hash::new_from_array(proof.hash.0),signer.pubkey())];
            let (next_hash, nonce) =
                self.find_next_hash_par(&treasury.difficulty.into(),&hash_and_pubkey, 0).await;
                println!("Called find_next_hash_par");
                println!("{} {}",next_hash, nonce);
            // Submit mine tx.
            // Use busses randomly so on each epoch, transactions don't pile on the same busses
            println!("\n\nSubmitting hash for validation...");
            'submit: loop {
                // Double check we're submitting for the right challenge
                let proof_ = get_proof(&self.rpc_client, signer.pubkey()).await;
                if !self.validate_hash(
                    ore::state::Hash(next_hash.to_bytes()).into(),
                    proof_.hash.into(),
                    signer.pubkey(),
                    nonce,
                    treasury.difficulty.into(),
                ) {
                    println!("Hash already validated! An earlier transaction must have landed.");
                    break;
                }

                // Reset epoch, if needed
                let treasury = get_treasury(&self.rpc_client).await;
                let clock = get_clock_account(&self.rpc_client).await;
                let threshold = treasury.last_reset_at.saturating_add(EPOCH_DURATION);
                if clock.unix_timestamp.ge(&threshold) {
                    // There are a lot of miners right now, so randomly select into submitting tx
                    if rng.gen_range(0..RESET_ODDS).eq(&0) {
                        println!("Sending epoch reset transaction...");
                        let cu_limit_ix =
                            ComputeBudgetInstruction::set_compute_unit_limit(CU_LIMIT_RESET);
                        let cu_price_ix =
                            ComputeBudgetInstruction::set_compute_unit_price(self.priority_fee);
                        let reset_ix = ore::instruction::reset(signer.pubkey());
                        self.send_and_confirm(&[cu_limit_ix, cu_price_ix, reset_ix], false, true)
                            .await
                            .ok();
                    }
                }
                println!("{:?}",ore::state::Hash(next_hash.to_bytes()));
                // Submit request.
                let bus = self.find_bus_id(treasury.reward_rate).await;
                let bus_rewards = (bus.rewards as f64) / (10f64.powf(ore::TOKEN_DECIMALS as f64));
                println!("Sending on bus {} ({} ORE)", bus.id, bus_rewards);
                let cu_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(CU_LIMIT_MINE);
                let cu_price_ix =
                    ComputeBudgetInstruction::set_compute_unit_price(self.priority_fee);
                let ix_mine = ore::instruction::mine(
                    signer.pubkey(),
                    BUS_ADDRESSES[bus.id as usize],
                    ore::state::Hash(next_hash.to_bytes()).into(),
                    nonce,
                );
                match self
                    .send_and_confirm(&[cu_limit_ix, cu_price_ix, ix_mine], false, false)
                    .await
                {
                    Ok(sig) => {
                        println!("Success: {}", sig);
                        break;
                    }
                    Err(_err) => {
                        // TODO
                    }
                }
            }
        }
    }

    async fn find_bus_id(&self, reward_rate: u64) -> Bus {
        let mut rng = rand::thread_rng();
        loop {
            let bus_id = rng.gen_range(0..BUS_COUNT);
            if let Ok(bus) = self.get_bus(bus_id).await {
                if bus.rewards.gt(&reward_rate.saturating_mul(20)) {
                    return bus;
                }
            }
        }
    }

    fn _find_next_hash(&self, hash: KeccakHash, difficulty: KeccakHash) -> (KeccakHash, u64) {
        let signer = self.signer();
        let mut next_hash: KeccakHash;
        let mut nonce = 0u64;
        loop {
            next_hash = hashv(&[
                hash.to_bytes().as_slice(),
                signer.pubkey().to_bytes().as_slice(),
                nonce.to_le_bytes().as_slice(),
            ]);
            if next_hash.le(&difficulty) {
                break;
            } else {
                println!("Invalid hash: {} Nonce: {:?}", next_hash.to_string(), nonce);
            }
            nonce += 1;
        }
        (next_hash, nonce)
    }

    async fn  find_next_hash_par(
        &self,
        difficulty: &solana_sdk::keccak::Hash,
        hash_and_pubkey: &[(solana_sdk::keccak::Hash, Pubkey)],
        threads: usize
    ) -> (KeccakHash, u64) {
        let found_solution = Arc::new(AtomicBool::new(false));
        let solution = Arc::new(Mutex::<(KeccakHash, u64)>::new((
            KeccakHash::new_from_array([0; 32]),
            0,
        )));
        let signer = self.signer();
        let pubkey = signer.pubkey();

    let mut child = tokio::process::Command::new("PATH_TO_EXE")
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .spawn()
    .expect("nonce_worker failed to spawn");
    println!("3");
    
    if let Some(mut stdin) = child.stdin.take() {
        let mut data_to_send = Vec::new();
    
        // Add difficulty bytes
        data_to_send.extend_from_slice(difficulty.as_ref());
    
        // Add hash and pubkey bytes
        for (hash, pubkey) in hash_and_pubkey {
            data_to_send.extend_from_slice(hash.as_ref());
            data_to_send.extend_from_slice(pubkey.as_ref());
        }
    
        // Optionally prepend the number of threads or any other control data
        // Here, we send the number of threads as the first byte, if required by your application
        let mut final_data = Vec::new();
        final_data.push(0 as u8);
        final_data.extend_from_slice(&data_to_send);
        println!("Sending the following bytes to the executable:");
        for byte in &final_data {
            print!("{:02X} ", byte);
        }
       
        // Write all bytes in one go
        stdin.write_all(&final_data).await.unwrap();
    
        // Dropping stdin to close it, signaling the end of input
        drop(stdin);
    }


    println!("4");

    let output = child.wait_with_output().await.unwrap().stdout;
        let mut results = vec![];
        println!("output {:?}", output);
        let chunks = output.chunks(40);
        for chunk in chunks {
            if chunk.len() < 40 {
                println!("Incomplete data chunk received, length: {}", chunk.len());
                continue;  // Skip this chunk or handle it according to your needs
            }
        
            let hash = solana_sdk::keccak::Hash(chunk[..32].try_into().unwrap());
            let nonce = u64::from_le_bytes(chunk[32..40].try_into().unwrap());
            println!("hash {:?}", hash);
            println!("nonce {:?}", nonce);
            results.push((hash, nonce));
        }
        println!("{:?}", results);
    

        results.get(0)
        .cloned()
        .ok_or_else(|| "No valid results were found".to_string()).expect("REASON")
    }

    pub fn validate_hash(
        &self,
        hash: KeccakHash,
        current_hash: KeccakHash,
        signer: Pubkey,
        nonce: u64,
        difficulty: KeccakHash,
    ) -> bool {
        // Validate hash correctness
        let hash_ = hashv(&[
            current_hash.as_ref(),
            signer.as_ref(),
            nonce.to_le_bytes().as_slice(),
        ]);
        if sol_memcmp(hash.as_ref(), hash_.as_ref(), HASH_BYTES) != 0 {
            return false;
        }

        // Validate hash difficulty
        if hash.gt(&difficulty) {
            return false;
        }

        true
    }

    pub async fn get_ore_display_balance(&self) -> String {
        let client = self.rpc_client.clone();
        let signer = self.signer();
        let token_account_address = spl_associated_token_account::get_associated_token_address(
            &signer.pubkey(),
            &ore::MINT_ADDRESS,
        );
        match client.get_token_account(&token_account_address).await {
            Ok(token_account) => {
                if let Some(token_account) = token_account {
                    token_account.token_amount.ui_amount_string
                } else {
                    "0.00".to_string()
                }
            }
            Err(_) => "0.00".to_string(),
        }
    }

}
