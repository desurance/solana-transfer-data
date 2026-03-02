use std::{fs, time::Duration};
use anyhow::{Context, Result};
use crypto_box::{
    aead::{Aead, AeadCore, OsRng},
    Nonce, PublicKey, SalsaBox, SecretKey,
};
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{pubkey::Pubkey, signature::Keypair};

pub const NONCE_SIZE: usize = 24;

pub const PUBLIC_KEY_SIZE: usize = 32;

pub const MAC_OVERHEAD: usize = 16;

/// Generate a fresh X25519 keypair for NaCl box encryption.
pub fn generate_keypair() -> (SecretKey, PublicKey) {
    let secret = SecretKey::generate(&mut OsRng);
    let public = secret.public_key();
    (secret, public)
}

/// Encrypt `plaintext` using the recipient's `recipient_pk` and an ephemeral
/// keypair. Returns `nonce || ephemeral_pk || ciphertext`.
pub fn encrypt(plaintext: &[u8], recipient_pk: &PublicKey) -> Result<Vec<u8>> {
    let (ephemeral_sk, ephemeral_pk) = generate_keypair();
    let salsa_box = SalsaBox::new(recipient_pk, &ephemeral_sk);
    let nonce = SalsaBox::generate_nonce(&mut OsRng);

    let ciphertext = salsa_box
        .encrypt(&nonce, plaintext)
        .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

    // Wire format: nonce (24) || ephemeral_pk (32) || ciphertext (len + 16 MAC)
    let mut out = Vec::with_capacity(NONCE_SIZE + PUBLIC_KEY_SIZE + ciphertext.len());
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(ephemeral_pk.as_bytes());
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// `encrypted` must be `nonce (24) || ephemeral_pk (32) || ciphertext`.
pub fn decrypt(encrypted: &[u8], recipient_sk: &SecretKey) -> Result<Vec<u8>> {
    let min_len = NONCE_SIZE + PUBLIC_KEY_SIZE + MAC_OVERHEAD;
    anyhow::ensure!(
        encrypted.len() >= min_len,
        "Encrypted blob too short ({} bytes, need >= {})",
        encrypted.len(),
        min_len
    );

    let nonce = Nonce::from_slice(&encrypted[..NONCE_SIZE]);
    let ephemeral_pk_bytes: [u8; PUBLIC_KEY_SIZE] = encrypted[NONCE_SIZE..NONCE_SIZE + PUBLIC_KEY_SIZE]
        .try_into()
        .context("Invalid ephemeral public key length")?;
    let ephemeral_pk = PublicKey::from(ephemeral_pk_bytes);
    let ciphertext = &encrypted[NONCE_SIZE + PUBLIC_KEY_SIZE..];

    let salsa_box = SalsaBox::new(&ephemeral_pk, recipient_sk);
    let plaintext = salsa_box
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("Decryption failed (wrong key or tampered data): {}", e))?;

    Ok(plaintext)
}

pub fn load_or_create_solana_keypair(path: &str) -> Result<Keypair> {
    let expanded = shellexpand::tilde(path).to_string();
    if let Ok(data) = fs::read_to_string(&expanded) {
        let bytes: Vec<u8> = serde_json::from_str(&data)?;
        Keypair::try_from(bytes.as_slice())
            .map_err(|e| anyhow::anyhow!("Invalid Solana keypair: {}", e))
    } else {
        let keypair = Keypair::new();
        if let Some(parent) = std::path::Path::new(&expanded).parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes: Vec<u8> = keypair.to_bytes().to_vec();
        fs::write(&expanded, serde_json::to_string(&bytes)?)
            .with_context(|| format!("Cannot write Solana keypair to {}", expanded))?;
        Ok(keypair)
    }
}

pub async fn airdrop_if_needed(rpc: &RpcClient, pubkey: &Pubkey) -> Result<()> {
    const MIN_BALANCE: u64 = 1_000_000_000; // 1 SOL
    const AIRDROP_AMOUNT: u64 = 2_000_000_000; // 2 SOL

    let balance = rpc.get_balance(pubkey).await?;
    if balance >= MIN_BALANCE {
        return Ok(());
    }

    info!(
        "Balance {:.2} SOL < 1 SOL, airdropping 2 SOL to {}",
        balance as f64 / 1e9,
        pubkey
    );
    let sig = rpc
        .request_airdrop(pubkey, AIRDROP_AMOUNT)
        .await
        .with_context(|| format!("Airdrop failed for {}", pubkey))?;

    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if rpc.confirm_transaction(&sig).await.unwrap_or(false) {
            info!("Airdrop confirmed for {} — tx: {}", pubkey, sig);
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_trip() {
        let (sk, pk) = generate_keypair();
        let msg = b"Hello, Solana data transfer!";
        let encrypted = encrypt(msg, &pk).unwrap();
        let decrypted = decrypt(&encrypted, &sk).unwrap();
        assert_eq!(decrypted, msg);
    }

    #[test]
    fn test_wrong_key_fails() {
        let (_sk, pk) = generate_keypair();
        let (wrong_sk, _) = generate_keypair();
        let encrypted = encrypt(b"secret", &pk).unwrap();
        assert!(decrypt(&encrypted, &wrong_sk).is_err());
    }
}