use actix_web::{App, HttpResponse, HttpServer, web};
use anyhow::{Result};
use crypto_box::{PublicKey, SecretKey};
use log::{error, info, warn};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::RpcTransactionConfig;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use solana_transaction_status::UiTransactionEncoding;
use solana_transfer_data_common::{
    crypto,
    network::SolanaNetwork,
    protocol::{self, Chunk, MAGIC},
};
use std::{
    collections::HashMap,
    fs,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(serde::Serialize, serde::Deserialize)]
struct NaClKeypairFile {
    secret_key: Vec<u8>,
    public_key: Vec<u8>,
}

struct TransferState {
    chunks: HashMap<u32, Chunk>,
    total_chunks: u32,
}

pub struct AppState {
    nacl_pk: PublicKey,
    nacl_sk: SecretKey,
    solana_pubkey: Pubkey,
    transfers: Mutex<HashMap<String, TransferState>>,
    data_callback: fn(Vec<u8>)
}

async fn get_public_key(state: web::Data<Arc<AppState>>) -> HttpResponse {
    let pk_bytes = state.nacl_pk.as_bytes();
    let body = serde_json::json!({
        "nacl_public_key": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, pk_bytes),
        "solana_address": state.solana_pubkey.to_string(),
    });
    HttpResponse::Ok().json(body)
}

async fn health() -> HttpResponse {
    HttpResponse::Ok().body("ok")
}


pub struct SolanaTransferDataServer {
    pub rpc: Arc<RpcClient>,
    pub app: Arc<AppState>,
    pub poll_interval: Duration,
    http_addr: String,
    http_port: u16,
}

impl SolanaTransferDataServer {
    pub async fn spawn_http_server(&self) -> Result<()> {
        let http_state = Arc::clone(&self.app);
        info!(
            "HTTP server listening on {}:{}",
            self.http_addr, self.http_port
        );
        info!("  GET /public-key  → NaCl public key + Solana address");
        info!("  GET /health      → health check");

        HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(Arc::clone(&http_state)))
                .route("/public-key", web::get().to(get_public_key))
                .route("/health", web::get().to(health))
        })
        .bind((self.http_addr.clone(), self.http_port))?
        .run()
        .await?;
        Ok(())
    }

    pub async fn spawn_poll_thread(&self) -> Result<()> {
        let state = Arc::clone(&self.app);
        let rpc = Arc::clone(&self.rpc);
        let poll_interval = Duration::from_secs(3);

        let mut last_sig: Option<Signature> = None;

        loop {
            tokio::time::sleep(poll_interval).await;
            info!("Polling transactions");

            let sigs_result = rpc.get_signatures_for_address(&state.solana_pubkey).await;
            let sigs = match sigs_result {
                Ok(s) => s,
                Err(e) => {
                    warn!("Error fetching signatures: {}", e);
                    continue;
                }
            };

            let new_sigs: Vec<_> = if let Some(ref last) = last_sig {
                sigs.iter()
                    .take_while(|s| Signature::from_str(&s.signature).ok().as_ref() != Some(last))
                    .collect()
            } else {
                sigs.iter().collect()
            };

            for sig_info in new_sigs.iter().rev() {
                let sig = match Signature::from_str(&sig_info.signature) {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                info!("Processing transaction: {}", sig);

                let chunks = match Self::extract_chunks_from_tx(&rpc, &sig).await {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("Failed to extract chunks from {}: {}", sig, e);
                        continue;
                    }
                };

                for chunk in chunks {
                    let tid_hex = hex::encode(chunk.header.transfer_id);
                    info!(
                        "  Received chunk {}/{} for transfer {}",
                        chunk.header.chunk_index + 1,
                        chunk.header.total_chunks,
                        &tid_hex[..16]
                    );

                    let mut transfers = state.transfers.lock().unwrap();
                    let entry = transfers
                        .entry(tid_hex.clone())
                        .or_insert_with(|| TransferState {
                            chunks: HashMap::new(),
                            total_chunks: chunk.header.total_chunks,
                        });
                    entry.chunks.insert(chunk.header.chunk_index, chunk);

                    if entry.chunks.len() == entry.total_chunks as usize {
                        info!("Transfer {} complete — reassembling", &tid_hex[..16]);

                        let mut ordered: Vec<Chunk> = entry.chunks.values().cloned().collect();
                        ordered.sort_by_key(|c| c.header.chunk_index);

                        // Capture expected transfer_id before dropping the lock
                        let expected_id = ordered[0].header.transfer_id;
                        drop(transfers);

                        match protocol::reassemble_chunks(&ordered) {
                            Ok(encrypted_data) => {
                                match crypto::decrypt(&encrypted_data, &state.nacl_sk) {
                                    Ok(plaintext) => {
                                        let actual_id = protocol::compute_transfer_id(&plaintext);
                                        if actual_id != expected_id {
                                            error!(
                                                "Transfer {} integrity check failed: SHA256 mismatch",
                                                &tid_hex[..16]
                                            );
                                        } else {
                                            (state.data_callback)(plaintext);
                                        }
                                    }
                                    Err(e) => error!(
                                        "Decryption failed for transfer {}: {}",
                                        &tid_hex[..16],
                                        e
                                    ),
                                }
                            }
                            Err(e) => {
                                error!("Reassembly failed for transfer {}: {}", &tid_hex[..16], e)
                            }
                        }

                        // Clean up completed transfer
                        state.transfers.lock().unwrap().remove(&tid_hex);
                    }
                }

                last_sig = Some(sig);
            }

            // Update last_sig to newest
            if let Some(first) = sigs.first() {
                if let Ok(s) = Signature::from_str(&first.signature) {
                    last_sig = Some(s);
                }
            }
        }
    }

    async fn extract_chunks_from_tx(rpc: &RpcClient, sig: &Signature) -> Result<Vec<Chunk>> {
        let config = RpcTransactionConfig {
            encoding: Some(UiTransactionEncoding::Base64),
            commitment: Some(CommitmentConfig::confirmed()),
            max_supported_transaction_version: Some(0),
        };
        let tx_result = rpc.get_transaction_with_config(sig, config).await?;

        let mut found = Vec::new();

        if let Some(tx) = tx_result.transaction.transaction.decode() {
            for ix in tx.message.instructions() {
                let data = &ix.data;
                if data.len() >= 4 && &data[..4] == MAGIC {
                    match Chunk::from_bytes(data) {
                        Ok(chunk) => found.push(chunk),
                        Err(e) => warn!("Failed to parse chunk from tx {}: {}", sig, e),
                    }
                }
            }
        }

        Ok(found)
    }
}

#[derive(Default)]
pub struct SolanaTransferDataServerBuilder {
    network: Option<SolanaNetwork>,
    nacl_sk: Option<SecretKey>,
    nacl_pk: Option<PublicKey>,
    solana_pubkey: Option<Pubkey>,
    http_addr: Option<String>,
    http_port: Option<u16>,
    poll_interval: Option<Duration>,
    data_callback: Option<fn(Vec<u8>)>
}

impl SolanaTransferDataServerBuilder {
    pub fn new() -> Self {
        SolanaTransferDataServerBuilder::default()
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

    pub fn with_keypair(&mut self, keypair: (SecretKey, PublicKey)) -> &mut Self {
        self.nacl_sk = Some(keypair.0);
        self.nacl_pk = Some(keypair.1);
        self
    }

    pub fn with_keypair_file_or_create(&mut self, path: &str) -> Result<&mut Self> {
        let expanded = shellexpand::tilde(path).to_string();
        if let Ok(data) = fs::read(&expanded) {
            let kf: NaClKeypairFile = serde_json::from_slice(&data)?;
            let sk_bytes: [u8; 32] = kf
                .secret_key
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid secret key length"))?;
            let sk = SecretKey::from(sk_bytes);
            let pk = sk.public_key();
            info!("Loaded existing NaCl keypair from {}", expanded);
            self.nacl_sk = Some(sk);
            self.nacl_pk = Some(pk);
        } else {
            let (sk, pk) = crypto::generate_keypair();
            let kf = NaClKeypairFile {
                secret_key: sk.to_bytes().to_vec(),
                public_key: pk.as_bytes().to_vec(),
            };
            fs::write(&expanded, serde_json::to_string_pretty(&kf)?)?;
            info!("Generated new NaCl keypair → {}", expanded);
            self.nacl_sk = Some(sk);
            self.nacl_pk = Some(pk);
        }
        Ok(self)
    }

    pub fn with_solana_pubkey(&mut self, solana_pubkey: Pubkey) -> &mut Self {
        self.solana_pubkey = Some(solana_pubkey);
        self
    }

    pub fn with_http_addr(&mut self, http_addr: String) -> &mut Self {
        self.http_addr = Some(http_addr);
        self
    }

    pub fn with_http_port(&mut self, http_port: u16) -> &mut Self {
        self.http_port = Some(http_port);
        self
    }

    pub fn with_poll_interval(&mut self, poll_interval: Duration) -> &mut Self {
        self.poll_interval = Some(poll_interval);
        self
    }

    pub fn with_data_callback(&mut self, data_callback: fn(Vec<u8>)) -> &mut Self {
        self.data_callback = Some(data_callback);
        self
    }

    pub fn build(&mut self) -> SolanaTransferDataServer {
        SolanaTransferDataServer {
            rpc: Arc::new(
                self.network
                    .unwrap_or(SolanaNetwork::Localhost)
                    .get_rpc_client(),
            ),
            app: Arc::new(AppState {
                nacl_pk: self.nacl_pk.take().unwrap(),
                nacl_sk: self.nacl_sk.take().unwrap(),
                solana_pubkey: self.solana_pubkey.unwrap(),
                transfers: Mutex::new(HashMap::new()),
                data_callback: self.data_callback.unwrap()
            }),
            poll_interval: self
                .poll_interval
                .unwrap_or_else(|| Duration::from_secs(10)),
            http_addr: self
                .http_addr
                .take()
                .unwrap_or_else(|| "0.0.0.0".to_string()),
            http_port: self.http_port.unwrap_or(8080),
        }
    }
}
