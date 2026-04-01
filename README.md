# Solana Data Transfer

Store and retrieve arbitrary data on Solana by chaining transactions into a reverse linked list. Each transaction holds a chunk of data and a pointer to the previous transaction. Only the last transaction signature is needed to reconstruct the full payload.

## How It Works

```
TX 1 (first)          TX 2 (continuation)      TX 3 (continuation)
┌──────────────────┐  ┌──────────────────────┐  ┌──────────────────────┐
│ total_size       │  │ prev_tx_sig → TX 1   │  │ prev_tx_sig → TX 2   │
│ sha256_hash      │  │ data (chunk 2)       │  │ data (chunk 3)       │
│ data (chunk 1)   │  └──────────────────────┘  └──────────────────────┘
└──────────────────┘
```

**Upload:** Data is split into ~900-byte chunks. The first transaction stores the total size and a SHA-256 hash. Each subsequent transaction references the previous one by its signature.

**Retrieve:** Given the last transaction signature, the client walks the chain backwards, collects all chunks, reverses them, and verifies the SHA-256 hash.

Data is stored entirely in transaction instruction data — no accounts or PDAs are created, so there are no rent costs.

## Project Structure

```
├── programs/solana-data-transfer/   Anchor on-chain program (Rust)
│   └── src/lib.rs                   Two instructions: upload_first, upload_continuation
├── app/client.ts                    TypeScript client library (Node.js + browser)
├── tests/                           Integration tests (Mocha/Chai)
├── examples/frontend/               Browser UI with Phantom/Solflare wallet support
└── Anchor.toml                      Anchor configuration
```

## Prerequisites

- [Rust](https://rustup.rs/) (toolchain 1.89.0, managed via `rust-toolchain.toml`)
- [Solana CLI](https://docs.solana.com/cli/install-solana-cli-tools)
- [Anchor CLI](https://www.anchor-lang.com/docs/installation) (0.32.x)
- [Node.js](https://nodejs.org/) (v18+)
- Yarn or npm

## Quick Start

### Build

```bash
anchor build
```

### Test

```bash
anchor test
```

This starts a local validator, deploys the program, and runs all 9 integration tests.

### Deploy to localnet

```bash
# Terminal 1: start local validator
solana-test-validator

# Terminal 2: deploy
anchor deploy --provider.cluster localnet
```

### Deploy to devnet

```bash
solana config set --url devnet
solana airdrop 2
anchor deploy --provider.cluster devnet
```

## Client Library

The TypeScript client in `app/client.ts` works in both Node.js and browser environments.

### Upload

```typescript
import { DataTransferClient } from "./app/client";

const client = new DataTransferClient(program, connection, walletPublicKey);

const data = new TextEncoder().encode("Hello, Solana!");
const lastTxSig = await client.uploadData(data);
// Save lastTxSig — it's the key to retrieve the data
```

### Retrieve

```typescript
const data = await client.retrieveData(lastTxSig);
const text = new TextDecoder().decode(data);
// "Hello, Solana!"
```

### Options

```typescript
await client.uploadData(data, {
  chunkSize: 950,     // bytes per transaction (default: 900, max ~986)
  maxRetries: 5,      // retry attempts per transaction (default: 5)
  baseDelayMs: 500,   // base delay for exponential backoff (default: 500)
});
```

## On-Chain Program

The program (`Emw5eLRyfwQrKnqt3P1UA2WwBrGgBRqaUjEv2Pu3YKGc`) exposes two instructions:

| Instruction | Parameters | Description |
|---|---|---|
| `upload_first` | `total_size: u32`, `sha256_hash: [u8; 32]`, `data: Vec<u8>` | First chunk — includes size and hash for verification |
| `upload_continuation` | `prev_tx_sig: [u8; 64]`, `data: Vec<u8>` | Subsequent chunks — links to previous transaction |

Both require a single `uploader` signer account. Validation errors:

| Error | Condition |
|---|---|
| `EmptyData` | Data chunk is empty |
| `InvalidDataSize` | Total size is zero |
| `DataExceedsTotalSize` | Chunk is larger than declared total size |

## Transaction Size Limits

Solana transactions are capped at 1232 bytes. After accounting for signatures, accounts, and instruction overhead:

| Instruction | Max data per chunk |
|---|---|
| `upload_first` | ~1014 bytes |
| `upload_continuation` | ~986 bytes |

The default chunk size of 900 bytes leaves a safe margin.

## Recommended: Automated Script

Browser wallets (Phantom, Solflare) require manual approval for **every transaction**. For multi-chunk uploads this means clicking "Approve" dozens or hundreds of times — one per chunk.

For anything beyond small payloads, use a Node.js script with a keypair file instead:

```typescript
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { DataTransferClient } from "./app/client";
import * as fs from "fs";

// Setup provider using local keypair (no wallet popups)
const provider = anchor.AnchorProvider.env();
anchor.setProvider(provider);

const program = anchor.workspace.solanaDataTransfer as Program;
const client = new DataTransferClient(
  program,
  provider.connection,
  provider.wallet.publicKey
);

// Upload a file
const fileData = new Uint8Array(fs.readFileSync("./my-file.txt"));
const lastTx = await client.uploadData(fileData);
console.log("Done! Retrieve with:", lastTx);

// Retrieve it back
const result = await client.retrieveData(lastTx);
fs.writeFileSync("./output.txt", result);
```

Run with:

```bash
ANCHOR_PROVIDER_URL=http://127.0.0.1:8899 \
ANCHOR_WALLET=~/.config/solana/id.json \
npx ts-node script.ts
```

This signs all transactions automatically using your local keypair — no manual approvals needed.

| Method | Best for | Approval flow |
|---|---|---|
| Frontend (browser wallet) | Small payloads, demos | Manual per transaction |
| Node.js script (keypair) | Large files, automation, CI | Fully automatic |

## Frontend Example

A browser-based UI for sending and retrieving data with wallet integration. See [`examples/frontend/README.md`](examples/frontend/README.md) for setup instructions.

