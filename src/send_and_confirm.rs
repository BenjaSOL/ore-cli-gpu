use std::{
    io::{stdout, Write},
    time::Duration,
};

use solana_client::{
    client_error::{ClientError, ClientErrorKind, Result as ClientResult},
    rpc_config::{RpcSendTransactionConfig, RpcSimulateTransactionConfig},
};
use solana_program::instruction::Instruction;
use solana_sdk::{
    commitment_config::CommitmentLevel,
    compute_budget::ComputeBudgetInstruction,
    signature::{Signature, Signer},
    transaction::Transaction,
};
use solana_transaction_status::{TransactionConfirmationStatus, UiTransactionEncoding};

use crate::Miner;

const RPC_RETRIES: usize = 0;
const SIMULATION_RETRIES: usize = 4;
const GATEWAY_RETRIES: usize = 150;
const CONFIRM_RETRIES: usize = 1;

const CONFIRM_DELAY: u64 = 0;
const GATEWAY_DELAY: u64 = 300;

impl Miner {
    pub async fn send_and_confirm(
        &self,
        ixs: &[Instruction],
        dynamic_cus: bool,
        skip_confirm: bool,
    ) -> ClientResult<Signature> {
        let mut stdout = stdout();
        let signer = self.signer();
        let client = self.rpc_client.clone();

        // Return error if balance is zero
        let balance = client.get_balance(&signer.pubkey()).await.unwrap();
        if balance <= 0 {
            return Err(ClientError {
                request: None,
                kind: ClientErrorKind::Custom("Insufficient SOL balance".into()),
            });
        }

        // Build tx
        let (_hash, slot) = client
            .get_latest_blockhash_with_commitment(self.rpc_client.commitment())
            .await
            .unwrap();
        let send_cfg = RpcSendTransactionConfig {
            skip_preflight: true,
            preflight_commitment: Some(CommitmentLevel::Confirmed),
            encoding: Some(UiTransactionEncoding::Base64),
            max_retries: Some(RPC_RETRIES),
            min_context_slot: None,
        };
        let mut tx = Transaction::new_with_payer(ixs, Some(&signer.pubkey()));

        // Simulate tx
        let mut sim_attempts = 0;
        'simulate: loop {
            let sim_res = client
                .simulate_transaction_with_config(
                    &tx,
                    RpcSimulateTransactionConfig {
                        sig_verify: false,
                        replace_recent_blockhash: true,
                        commitment: Some(self.rpc_client.commitment()),
                        encoding: Some(UiTransactionEncoding::Base64),
                        accounts: None,
                        min_context_slot: Some(slot),
                        inner_instructions: false,
                    },
                )
                .await;
            match sim_res {
                Ok(sim_res) => {
                    if let Some(err) = sim_res.value.err {
                        println!("Simulaton error: {:?}", err);
                        sim_attempts += 1;
                    } else if let Some(units_consumed) = sim_res.value.units_consumed {
                        if dynamic_cus {
                            println!("Dynamic CUs: {:?}", units_consumed);
                            let cu_budget_ix = ComputeBudgetInstruction::set_compute_unit_limit(
                                units_consumed as u32 + 1000,
                            );
                            let cu_price_ix =
                                ComputeBudgetInstruction::set_compute_unit_price(self.priority_fee);
                            let mut final_ixs = vec![];
                            final_ixs.extend_from_slice(&[cu_budget_ix, cu_price_ix]);
                            final_ixs.extend_from_slice(ixs);
                            tx = Transaction::new_with_payer(&final_ixs, Some(&signer.pubkey()));
                        }
                        break 'simulate;
                    }
                }
                Err(err) => {
                    println!("Simulaton error: {:?}", err);
                    sim_attempts += 1;
                }
            }

            // Abort if sim fails
            if sim_attempts.gt(&SIMULATION_RETRIES) {
                return Err(ClientError {
                    request: None,
                    kind: ClientErrorKind::Custom("Simulation failed".into()),
                });
            }
        }

        // Update hash before sending transactions
        let (hash, _slot) = client
            .get_latest_blockhash_with_commitment(self.rpc_client.commitment())
            .await
            .unwrap();

        // Submit tx
        tx.sign(&[&signer], hash);
        // let mut sigs = vec![];
        let mut attempts = 0;
        loop {
            println!("Attempt: {:?}", attempts);
            match client.send_transaction_with_config(&tx, send_cfg).await {
                Ok(sig) => {
                    println!("{:?}", sig);
                    // sigs.push(sig);

                    // Confirm tx
                    if skip_confirm {
                        return Ok(sig);
                    }
                    for _ in 0..CONFIRM_RETRIES {
                        std::thread::sleep(Duration::from_millis(CONFIRM_DELAY));
                        match client.get_signature_statuses(&[sig]).await {
                            Ok(signature_statuses) => {
                                println!("Confirmation: {:?}", signature_statuses.value[0]);
                                for signature_status in signature_statuses.value {
                                    if let Some(signature_status) = signature_status.as_ref() {
                                        if signature_status.confirmation_status.is_some() {
                                            let current_commitment = signature_status
                                                .confirmation_status
                                                .as_ref()
                                                .unwrap();
                                            match current_commitment {
                                                TransactionConfirmationStatus::Processed => {}
                                                TransactionConfirmationStatus::Confirmed
                                                | TransactionConfirmationStatus::Finalized => {
                                                    println!("Transaction landed!");
                                                    std::thread::sleep(Duration::from_millis(
                                                        GATEWAY_DELAY,
                                                    ));
                                                    return Ok(sig);
                                                }
                                            }
                                        } else {
                                            println!("No status");
                                        }
                                    }
                                }
                            }

                            // Handle confirmation errors
                            Err(err) => {
                                println!("{:?}", err.kind().to_string());
                            }
                        }
                    }
                    println!("Transaction did not land");
                }

                // Handle submit errors
                Err(err) => {
                    println!("{:?}", err.kind().to_string());
                }
            }

            // Retry
            stdout.flush().ok();
            std::thread::sleep(Duration::from_millis(GATEWAY_DELAY));
            attempts += 1;
            if attempts > GATEWAY_RETRIES {
                return Err(ClientError {
                    request: None,
                    kind: ClientErrorKind::Custom("Max retries".into()),
                });
            }
        }
    }
}
