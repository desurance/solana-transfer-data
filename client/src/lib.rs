use anyhow::{Context, Result};
use crypto_box::PublicKey;
use log::{error, info};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_request::Address;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Signer,
    transaction::Transaction,
};
use solana_transfer_data_common::{
    crypto,
    network::{SolanaNetwork},
    protocol::{self, Chunk},
};
use std::{str::FromStr, sync::Arc, time::Duration};
use solana_keypair::Keypair;

pub struct SolanaTransferDataClient {
    pub sender_keypair: Keypair,
    tx_delay_ms: u64,
    nacl_pk: PublicKey,
    destination_address: Address,
    rpc: Arc<RpcClient>,
    program_id: Pubkey,
}

impl SolanaTransferDataClient {
    pub fn get_rpc(&self) -> &RpcClient {
        &self.rpc
    }

    async fn send_chunk(
        &self,
        receiver_addr: &Pubkey,
        chunks: &[Chunk],
    ) -> Result<()> {
        let tx_delay = Duration::from_millis(self.tx_delay_ms);

        let total = chunks.len();
        info!("Sending {} chunk transaction(s)…", total);

        for (i, chunk) in chunks.iter().enumerate() {
            let ix = Self::build_chunk_instruction(chunk, &self.sender_keypair.pubkey(), receiver_addr, &self.program_id)?;
            let recent_blockhash = self.rpc.get_latest_blockhash().await?;

            let tx = Transaction::new_signed_with_payer(
                &[ix],
                Some(&self.sender_keypair.pubkey()),
                &[&self.sender_keypair],
                recent_blockhash,
            );

            match self.rpc.send_and_confirm_transaction(&tx).await {
                Ok(sig) => {
                    info!(
                        "  [{}/{}] Sent chunk {} — tx: {}",
                        i + 1,
                        total,
                        chunk.header.chunk_index,
                        sig
                    );
                }
                Err(e) => {
                    error!(
                        "  [{}/{}] FAILED to send chunk {}: {}",
                        i + 1,
                        total,
                        chunk.header.chunk_index,
                        e
                    );
                    return Err(e.into());
                }
            }

            if i + 1 < total {
                tokio::time::sleep(tx_delay).await;
            }
        }

        info!("✓ All {} chunks sent successfully", total);
        Ok(())
    }

    fn build_chunk_instruction(
        chunk: &Chunk,
        sender: &Pubkey,
        receiver: &Pubkey,
        program_id: &Pubkey,
    ) -> Result<Instruction> {
        Ok(Instruction {
            program_id: *program_id,
            accounts: vec![
                AccountMeta::new_readonly(*sender, true),
                AccountMeta::new_readonly(*receiver, false),
            ],
            data: chunk.to_bytes(),
        })
    }

    pub async fn send_as_file(
        &self,
        content: Vec<u8>,
        filename: String,
    ) -> Result<()> {
        info!("Read file: {} ({} bytes)", filename, content.len());

        let transfer_id = protocol::compute_transfer_id(&content);
        info!("Transfer ID: {}", hex::encode(transfer_id));

        let encrypted = crypto::encrypt(&content, &self.nacl_pk)?;
        info!(
            "Encrypted size: {} bytes (+{} overhead)",
            encrypted.len(),
            encrypted.len() - content.len()
        );

        let chunks = protocol::split_into_chunks(&encrypted, transfer_id, &filename);
        info!("Split into {} chunk(s)", chunks.len());

        self.send_chunk(&self.destination_address, &chunks).await?;

        Ok(())
    }
}

#[derive(Default)]
pub struct SolanaTransferDataClientBuilder {
    network: Option<SolanaNetwork>,
    sender_keypair: Option<Keypair>,
    server_url: Option<String>,
    tx_delay_ms: u64,
    program_id: Option<Pubkey>,
}

impl SolanaTransferDataClientBuilder {
    pub fn new() -> SolanaTransferDataClientBuilder {
        SolanaTransferDataClientBuilder {
            network: Some(SolanaNetwork::Devnet),
            sender_keypair: None,
            tx_delay_ms: 500,
            server_url: None,
            program_id: None,
        }
    }

    pub fn with_localhost(&mut self) -> &mut Self {
        self.network = Some(SolanaNetwork::Localhost);
        self
    }

    pub fn with_devnet(&mut self) -> &mut Self {
        self.network = Some(SolanaNetwork::Devnet);
        self
    }

    pub fn with_mainnet(&mut self) -> &mut Self {
        self.network = Some(SolanaNetwork::Mainnet);
        self
    }

    pub fn with_network(&mut self, network: SolanaNetwork) -> &mut Self {
        self.network = Some(network);
        self
    }

    pub fn with_solana_keypair(&mut self, keypair: Keypair) -> &mut Self {
        self.sender_keypair = Some(keypair);
        self
    }

    pub fn with_tx_delay_ms(&mut self, delay_ms: u64) -> &mut Self {
        self.tx_delay_ms = delay_ms;
        self
    }

    pub fn with_program_id(&mut self, program_id: Pubkey) -> &mut Self {
        self.program_id = Some(program_id);
        self
    }

    pub fn with_server_url(&mut self, server_url: &str) -> &mut Self {
        self.server_url = Some(server_url.to_string());
        self
    }

    pub async fn build(&mut self) -> Result<SolanaTransferDataClient> {
        let (nacl_pk, destination_addr) = self.fetch_server_key().await?;
        Ok(SolanaTransferDataClient {
            sender_keypair: self.sender_keypair.take().unwrap(),
            tx_delay_ms: self.tx_delay_ms,
            nacl_pk,
            destination_address: destination_addr,
            rpc: Arc::new(self.network.unwrap().get_rpc_client()),
            program_id: self.program_id.take().unwrap(),
        })
    }

    async fn fetch_server_key(&self) -> Result<(PublicKey, Pubkey)> {
        let server_url = self.server_url.clone().unwrap();
        let url = format!("{}/public-key", server_url.trim_end_matches('/'));
        info!("Fetching server public key from {}", url);

        let resp: ServerKeyResponse = reqwest::get(&url)
            .await
            .with_context(|| format!("Cannot reach server at {}", url))?
            .json()
            .await
            .context("Invalid JSON response from server")?;

        let pk_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &resp.nacl_public_key,
        )
            .context("Invalid base64 in nacl_public_key")?;
        let pk_array: [u8; 32] = pk_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("NaCl public key must be 32 bytes"))?;
        let nacl_pk = PublicKey::from(pk_array);

        // Parse Solana address
        let solana_addr =
            Pubkey::from_str(&resp.solana_address).context("Invalid Solana address from server")?;

        info!("Server Solana address: {}", solana_addr);
        Ok((nacl_pk, solana_addr))
    }
}

#[derive(serde::Deserialize)]
struct ServerKeyResponse {
    nacl_public_key: String,
    solana_address: String,
}
