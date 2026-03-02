use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Maximum payload size per Solana transaction instruction data.
/// A Solana transaction has a hard limit of 1232 bytes, because
/// there are multiple metadata will add so fixing this to 800.
pub const MAX_CHUNK_PAYLOAD: usize = 800;

pub const MAGIC: &[u8; 4] = b"SDTS"; // Solana Data Transfer System

pub const VERSION: u8 = 1;

/// Header prepended to every chunk inside the on-chain program instruction.
///
/// Wire format (big-endian):
/// ```text
///   MAGIC (4) | VERSION (1) | transfer_id (32) | chunk_index (4) | total_chunks (4)
///   | filename_len (2) | filename (variable) | payload (variable)
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkHeader {
    pub transfer_id: [u8; 32],
    pub chunk_index: u32,
    pub total_chunks: u32,
    pub filename: String,
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub header: ChunkHeader,
    pub payload: Vec<u8>,
}

impl Chunk {
    pub fn to_bytes(&self) -> Vec<u8> {
        let filename_bytes = self.header.filename.as_bytes();
        let filename_len = filename_bytes.len() as u16;

        let total_len = 4  // MAGIC
            + 1            // VERSION
            + 32           // transfer_id
            + 4            // chunk_index
            + 4            // total_chunks
            + 2            // filename_len
            + filename_bytes.len()
            + self.payload.len();

        let mut buf = Vec::with_capacity(total_len);
        buf.extend_from_slice(MAGIC);
        buf.push(VERSION);
        buf.extend_from_slice(&self.header.transfer_id);
        buf.extend_from_slice(&self.header.chunk_index.to_be_bytes());
        buf.extend_from_slice(&self.header.total_chunks.to_be_bytes());
        buf.extend_from_slice(&filename_len.to_be_bytes());
        buf.extend_from_slice(filename_bytes);
        buf.extend_from_slice(&self.payload);
        buf
    }

    pub fn from_bytes(data: &[u8]) -> anyhow::Result<Self> {
        let min = 4 + 1 + 32 + 4 + 4 + 2;
        anyhow::ensure!(data.len() >= min, "Chunk data too short");

        let magic = &data[0..4];
        anyhow::ensure!(magic == MAGIC, "Invalid magic bytes");

        let version = data[4];
        anyhow::ensure!(version == VERSION, "Unsupported protocol version {}", version);

        let mut offset = 5;

        let mut transfer_id = [0u8; 32];
        transfer_id.copy_from_slice(&data[offset..offset + 32]);
        offset += 32;

        let chunk_index = u32::from_be_bytes(data[offset..offset + 4].try_into()?);
        offset += 4;

        let total_chunks = u32::from_be_bytes(data[offset..offset + 4].try_into()?);
        offset += 4;

        let filename_len = u16::from_be_bytes(data[offset..offset + 2].try_into()?) as usize;
        offset += 2;

        anyhow::ensure!(data.len() >= offset + filename_len, "Filename truncated");
        let filename = String::from_utf8(data[offset..offset + filename_len].to_vec())?;
        offset += filename_len;

        let payload = data[offset..].to_vec();

        Ok(Chunk {
            header: ChunkHeader {
                transfer_id,
                chunk_index,
                total_chunks,
                filename,
            },
            payload,
        })
    }
}

pub fn compute_transfer_id(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

pub fn split_into_chunks(
    encrypted_data: &[u8],
    transfer_id: [u8; 32],
    filename: &str,
) -> Vec<Chunk> {
    let header_overhead = 4 + 1 + 32 + 4 + 4 + 2;
    let first_chunk_payload_cap = MAX_CHUNK_PAYLOAD - header_overhead - filename.len();
    let other_chunk_payload_cap = MAX_CHUNK_PAYLOAD - header_overhead;

    if encrypted_data.is_empty() {
        return vec![Chunk {
            header: ChunkHeader {
                transfer_id,
                chunk_index: 0,
                total_chunks: 1,
                filename: filename.to_string(),
            },
            payload: vec![],
        }];
    }

    let first_part = encrypted_data.len().min(first_chunk_payload_cap);
    let remaining = encrypted_data.len().saturating_sub(first_part);
    let extra_chunks = if remaining == 0 {
        0
    } else {
        (remaining + other_chunk_payload_cap - 1) / other_chunk_payload_cap
    };
    let total_chunks = 1 + extra_chunks as u32;

    let mut chunks = Vec::with_capacity(total_chunks as usize);

    chunks.push(Chunk {
        header: ChunkHeader {
            transfer_id,
            chunk_index: 0,
            total_chunks,
            filename: filename.to_string(),
        },
        payload: encrypted_data[..first_part].to_vec(),
    });

    let mut offset = first_part;
    let mut idx = 1u32;
    while offset < encrypted_data.len() {
        let end = (offset + other_chunk_payload_cap).min(encrypted_data.len());
        chunks.push(Chunk {
            header: ChunkHeader {
                transfer_id,
                chunk_index: idx,
                total_chunks,
                filename: String::new(),
            },
            payload: encrypted_data[offset..end].to_vec(),
        });
        offset = end;
        idx += 1;
    }

    chunks
}

/// Reassemble payload from ordered chunks.
pub fn reassemble_chunks(chunks: &[Chunk]) -> anyhow::Result<(String, Vec<u8>)> {
    anyhow::ensure!(!chunks.is_empty(), "No chunks to reassemble");
    let total = chunks[0].header.total_chunks as usize;
    anyhow::ensure!(
        chunks.len() == total,
        "Expected {} chunks, got {}",
        total,
        chunks.len()
    );

    let filename = chunks
        .iter()
        .find(|c| c.header.chunk_index == 0)
        .map(|c| c.header.filename.clone())
        .unwrap_or_default();
    let mut data = Vec::new();
    for i in 0..total {
        let chunk = chunks
            .iter()
            .find(|c| c.header.chunk_index == i as u32)
            .ok_or_else(|| anyhow::anyhow!("Missing chunk {}", i))?;
        data.extend_from_slice(&chunk.payload);
    }

    Ok((filename, data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::SmallRng, RngExt, SeedableRng};

    fn round_trip(data: &[u8], filename: &str) {
        let tid = compute_transfer_id(data);
        let chunks = split_into_chunks(data, tid, filename);

        // Every chunk must fit within MAX_CHUNK_PAYLOAD
        for c in &chunks {
            assert!(
                c.to_bytes().len() <= MAX_CHUNK_PAYLOAD,
                "chunk {} exceeds MAX_CHUNK_PAYLOAD ({} > {})",
                c.header.chunk_index,
                c.to_bytes().len(),
                MAX_CHUNK_PAYLOAD,
            );
        }

        // Serialize / deserialize each chunk
        let mut parsed: Vec<Chunk> = chunks
            .iter()
            .map(|c| Chunk::from_bytes(&c.to_bytes()).unwrap())
            .collect();

        // Shuffle to verify reassembly handles out-of-order chunks
        use rand::seq::SliceRandom;
        parsed.shuffle(&mut SmallRng::seed_from_u64(42));

        let (fname, reassembled) = reassemble_chunks(&parsed).unwrap();
        assert_eq!(fname, filename);
        assert_eq!(reassembled, data);
    }

    #[test]
    fn test_round_trip_small() {
        // Fits in a single chunk
        let mut rng = SmallRng::seed_from_u64(1);
        let data: Vec<u8> = (0..100).map(|_| rng.random()).collect();
        round_trip(&data, "small.bin");
    }

    #[test]
    fn test_round_trip_exact_chunk_boundary() {
        // Data that lands exactly on a chunk boundary
        let header_overhead = 4 + 1 + 32 + 4 + 4 + 2;
        let filename = "boundary.bin";
        let first_cap = MAX_CHUNK_PAYLOAD - header_overhead - filename.len();
        let other_cap = MAX_CHUNK_PAYLOAD - header_overhead;
        let size = first_cap + other_cap * 3; // exactly 4 chunks
        let mut rng = SmallRng::seed_from_u64(2);
        let data: Vec<u8> = (0..size).map(|_| rng.random()).collect();
        round_trip(&data, filename);
    }

    #[test]
    fn test_round_trip_large_random() {
        // ~370 KB of random data (~493 chunks, matching the real test.bin scenario)
        let mut rng = SmallRng::seed_from_u64(3);
        let data: Vec<u8> = (0..370_000).map(|_| rng.random()).collect();
        round_trip(&data, "test.bin");
    }

    #[test]
    fn test_round_trip_empty() {
        round_trip(&[], "empty.bin");
    }
}