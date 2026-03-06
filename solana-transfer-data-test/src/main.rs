#[cfg(test)]
mod tests {
    use anyhow::Result;
    use log::{error, info};
    use rand::RngExt;
    use solana_keypair::Signer;
    use solana_sdk::pubkey::Pubkey;
    use solana_transfer_data_client::{SolanaTransferDataClient, SolanaTransferDataClientBuilder};
    use solana_transfer_data_common::crypto::{airdrop_if_needed, load_or_create_solana_keypair};
    use solana_transfer_data_server::SolanaTransferDataServerBuilder;
    use std::path::PathBuf;
    use std::str::FromStr;
    use std::sync::Once;
    use std::time::Duration;
    use tokio::fs;
    use tokio::sync::OnceCell;
    use tokio::try_join;

    const DATA_TRANSFER_PROGRAM_ID: &str = "8iafGoJHJUt4rGER5jQ7UCR8ZzHmFWzP3miUv3bi1WXC";
    const OUTPUT_DIR: &str = "./target/test";

    static INIT_LOGGER: Once = Once::new();
    static SERVER_HANDLE: OnceCell<()> = OnceCell::const_new();

    fn init_logger() {
        INIT_LOGGER.call_once(|| {
            env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                .init();
        });
    }

    async fn ensure_server() {
        SERVER_HANDLE
            .get_or_init(|| async {
                fs::create_dir_all(OUTPUT_DIR).await.unwrap();

                let server = SolanaTransferDataServerBuilder::new()
                    .with_localhost()
                    .with_solana_pubkey(
                        load_or_create_solana_keypair("~/.config/solana/id_server.json")
                            .expect("Failed to load server keypair")
                            .pubkey(),
                    )
                    .with_keypair_file_or_create("./server_nacl_keypair.json")
                    .expect("Failed to create NaCl keypair")
                    .with_data_callback(|content| {
                        let len = u16::from_be_bytes([content[0], content[1]]) as usize;
                        let filename = std::str::from_utf8(&content[2..2 + len])
                            .unwrap()
                            .to_owned();
                        let inner = content[2 + len..].to_vec();
                        tokio::spawn(async move {
                            let out_path = PathBuf::from(OUTPUT_DIR).join(&filename);
                            match fs::write(&out_path, &inner).await {
                                Ok(_) => info!(
                                    "✓ Saved decrypted file: {} ({} bytes)",
                                    out_path.display(),
                                    inner.len()
                                ),
                                Err(e) => {
                                    error!("Failed to write {}: {}", out_path.display(), e)
                                }
                            }
                        });
                    })
                    .build();

                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    rt.block_on(async move {
                        let _ = try_join!(server.spawn_poll_thread(), server.spawn_http_server());
                    });
                });

                tokio::time::sleep(Duration::from_secs(3)).await;
            })
            .await;
    }

    async fn build_client() -> Result<SolanaTransferDataClient> {
        let client = SolanaTransferDataClientBuilder::new()
            .with_localhost()
            .with_server_url("http://localhost:8080")
            .with_solana_keypair(load_or_create_solana_keypair(
                "~/.config/solana/id_client.json",
            )?)
            .with_program_id(Pubkey::from_str(DATA_TRANSFER_PROGRAM_ID).unwrap())
            .build()
            .await?;

        airdrop_if_needed(&client.get_rpc(), &client.sender_keypair.pubkey()).await?;
        Ok(client)
    }

    fn random_data() -> Vec<u8> {
        let mut rng = rand::rng();
        let len: usize = rng.random_range(1000..=10000);
        (0..len).map(|_| rng.random::<u8>()).collect()
    }

    async fn wait_for_output(path: &PathBuf) -> Result<Vec<u8>> {
        loop {
            if let Ok(data) = fs::read(path).await {
                return Ok(data);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn run_transfer_test(name: &str) -> Result<()> {
        init_logger();
        ensure_server().await;

        println!("[{name}] building client...");
        let client = build_client().await?;

        let filename = format!(
            "{}.bin",
            rand::rng()
                .sample_iter(rand::distr::Alphanumeric)
                .take(10)
                .map(|b| b as char)
                .collect::<String>()
        );
        let filename_bytes = filename.as_bytes();
        let filename_len = filename_bytes.len();
        let out_path = PathBuf::from(OUTPUT_DIR).join(&filename);
        if fs::try_exists(&out_path).await? {
            fs::remove_file(&out_path).await?;
        }

        let sent = random_data();

        let mut buf = Vec::new();
        buf.extend_from_slice(&(filename_len as u16).to_be_bytes());
        buf.extend_from_slice(filename_bytes);
        buf.extend_from_slice(&sent);

        println!("[{name}] sending {} bytes", buf.len());
        client.send_as_bytes(buf.clone()).await?;

        println!("[{name}] waiting for output...");
        let received = wait_for_output(&out_path).await?;

        anyhow::ensure!(
            received == sent,
            "[{}] data mismatch — sent {} bytes, received {} bytes",
            name,
            sent.len(),
            received.len()
        );
        println!("[{name}] ✓ passed ({} bytes)", sent.len());
        Ok(())
    }

    seq_macro::seq!(N in 0..10 {
        #[tokio::test]
        async fn test_transfer_~N() -> Result<()> {
            run_transfer_test(&format!("transfer-{}", N)).await
        }
    });
}

fn main() {}
