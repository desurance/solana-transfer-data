use anyhow::Result;
use log::{error, info};
use rand::RngExt;
use solana_keypair::Signer;
use solana_sdk::pubkey::Pubkey;
use solana_transfer_data_client::{SolanaTransferDataClient, SolanaTransferDataClientBuilder};
use solana_transfer_data_common::{
    crypto::{airdrop_if_needed, load_or_create_solana_keypair},
};
use solana_transfer_data_server::SolanaTransferDataServerBuilder;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use tokio::fs;
use tokio::try_join;


// MUST Change it into actual(deployed) ID
const DATA_TRANSFER_PROGRAM_ID: &str = "8iafGoJHJUt4rGER5jQ7UCR8ZzHmFWzP3miUv3bi1WXC";
const OUTPUT_DIR: &str = "./target/test";

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

async fn run_transfer_test(client: &SolanaTransferDataClient, name: &str) -> Result<()> {
    let sent = random_data();
    let filename = format!("{}.bin", name);
    let out_path = PathBuf::from(OUTPUT_DIR).join(&filename);

    info!("Test '{}': sending {} bytes", name, sent.len());
    client.send_as_file(sent.clone(), filename).await?;

    let received = wait_for_output(&out_path).await?;

    anyhow::ensure!(
        received == sent,
        "Test '{}': data mismatch — sent {} bytes, received {} bytes",
        name,
        sent.len(),
        received.len()
    );
    info!("✓ Test '{}' passed ({} bytes)", name, sent.len());
    Ok(())
}

async fn client_test() -> Result<()> {
    tokio::time::sleep(Duration::from_secs(5)).await;
    let solana_client = SolanaTransferDataClientBuilder::new()
        .with_localhost()
        .with_server_url("http://localhost:8080")
        .with_solana_keypair(load_or_create_solana_keypair("~/.config/solana/id_client.json")?)
        .with_program_id(Pubkey::from_str(DATA_TRANSFER_PROGRAM_ID).unwrap())
        .build()
        .await?;

    info!("Sender address: {}", solana_client.sender_keypair.pubkey());
    airdrop_if_needed(&solana_client.get_rpc(), &solana_client.sender_keypair.pubkey()).await?;

    let balance = solana_client
        .get_rpc()
        .get_balance(&solana_client.sender_keypair.pubkey())
        .await?;
    info!("Sender balance: {:.4} SOL", balance as f64 / 1e9);

    // Clear output directory so we don't read stale files from a previous run
    fs::remove_dir_all(OUTPUT_DIR).await.ok();
    fs::create_dir_all(OUTPUT_DIR).await?;

    try_join!(
        run_transfer_test(&solana_client, "test_1"),
        run_transfer_test(&solana_client, "test_2"),
        run_transfer_test(&solana_client, "test_3"),
        run_transfer_test(&solana_client, "test_4"),
    )?;

    info!("✓ All tests passed!");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    fs::create_dir_all(OUTPUT_DIR).await?;

    let server = SolanaTransferDataServerBuilder::new()
        .with_localhost()
        .with_solana_pubkey(load_or_create_solana_keypair("~/.config/solana/id_server.json")?.pubkey())
        .with_keypair_file_or_create("./server_nacl_keypair.json")?
        .with_file_callback(|filename, content| {
            tokio::spawn(async move {
                let out_path = PathBuf::from(OUTPUT_DIR).join(&filename);
                match fs::write(&out_path, &content).await {
                    Ok(_) => info!(
                        "✓ Saved decrypted file: {} ({} bytes)",
                        out_path.display(),
                        content.len()
                    ),
                    Err(e) => error!("Failed to write {}: {}", out_path.display(), e),
                }
            });
        })
        .build();

    let _ = try_join!(
        server.spawn_poll_thread(),
        server.spawn_http_server(),
        client_test()
    );

    loop {}
}
