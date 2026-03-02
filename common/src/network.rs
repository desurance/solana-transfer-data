use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SolanaNetwork {
    Localhost,
    Devnet,
    Mainnet,
}

impl SolanaNetwork {
    pub fn rpc_url(&self) -> &str {
        match self {
            SolanaNetwork::Localhost => "http://127.0.0.1:8899",
            SolanaNetwork::Devnet => "https://api.devnet.solana.com",
            SolanaNetwork::Mainnet => "https://api.mainnet-beta.solana.com",
        }
    }

    pub fn ws_url(&self) -> &str {
        match self {
            SolanaNetwork::Localhost => "ws://127.0.0.1:8900",
            SolanaNetwork::Devnet => "wss://api.devnet.solana.com",
            SolanaNetwork::Mainnet => "wss://api.mainnet-beta.solana.com",
        }
    }

    pub fn from_str_name(s: &str) -> anyhow::Result<Self> {
        match s.to_lowercase().as_str() {
            "localhost" | "local" | "l" => Ok(SolanaNetwork::Localhost),
            "devnet" | "dev" | "d" => Ok(SolanaNetwork::Devnet),
            "mainnet" | "mainnet-beta" | "main" | "m" => Ok(SolanaNetwork::Mainnet),
            _ => anyhow::bail!("Unknown network '{}'. Use: localhost, devnet, or mainnet", s),
        }
    }

    pub fn get_rpc_client(&self) -> RpcClient {
        let url = self.rpc_url();
        log::info!("Connecting to Solana cluster: {} ({})", self, url);
        RpcClient::new_with_commitment(url.to_string(), CommitmentConfig::confirmed())
    }
}

impl fmt::Display for SolanaNetwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolanaNetwork::Localhost => write!(f, "localhost"),
            SolanaNetwork::Devnet => write!(f, "devnet"),
            SolanaNetwork::Mainnet => write!(f, "mainnet-beta"),
        }
    }
}

