import * as anchor from "@coral-xyz/anchor";
import { BorshInstructionCoder, Idl, Program } from "@coral-xyz/anchor";
import { Connection, PublicKey } from "@solana/web3.js";

const bs58 = anchor.utils.bytes.bs58;

const DEFAULT_CHUNK_SIZE = 900;
const DEFAULT_MAX_RETRIES = 5;
const DEFAULT_BASE_DELAY_MS = 500;

export interface UploadOptions {
  /** Max bytes per chunk (default: 800) */
  chunkSize?: number;
  /** Max retry attempts per transaction (default: 5) */
  maxRetries?: number;
  /** Base delay in ms for exponential backoff (default: 500) */
  baseDelayMs?: number;
}

/**
 * Client for the Solana Data Transfer protocol.
 *
 * Uploads arbitrary data to Solana as a linked list of transactions.
 * Each transaction contains a chunk of data and a pointer to the previous
 * transaction, forming a reverse linked list traversable from the last tx.
 *
 * Works in both Node.js and browser environments.
 * Uses Web Crypto API (globalThis.crypto.subtle) for SHA-256 hashing
 * and Uint8Array instead of Buffer for universal compatibility.
 */
export class DataTransferClient {
  constructor(
    private program: Program<Idl>,
    private connection: Connection,
    private payer: PublicKey
  ) {}

  /**
   * Upload data to Solana.
   * Splits data into chunks and uploads as a linked list of transactions.
   * Returns the last transaction signature (use it to retrieve the data).
   */
  async uploadData(
    data: Uint8Array,
    opts: UploadOptions = {}
  ): Promise<string> {
    if (data.length === 0) {
      throw new Error("Data cannot be empty");
    }

    const chunkSize = opts.chunkSize ?? DEFAULT_CHUNK_SIZE;
    const maxRetries = opts.maxRetries ?? DEFAULT_MAX_RETRIES;
    const baseDelay = opts.baseDelayMs ?? DEFAULT_BASE_DELAY_MS;

    const sha256Hash = await sha256(data);
    const chunks = splitIntoChunks(data, chunkSize);

    // First block
    let prevSig = await this.sendWithRetry(
      () =>
        this.program.methods
          .uploadFirst(data.length, [...sha256Hash], asBuffer(chunks[0]))
          .accounts({ uploader: this.payer })
          .rpc({ commitment: "confirmed" }),
      maxRetries,
      baseDelay
    );

    // Continuation blocks
    for (let i = 1; i < chunks.length; i++) {
      const sigBytes = bs58.decode(prevSig);
      prevSig = await this.sendWithRetry(
        () =>
          this.program.methods
            .uploadContinuation([...sigBytes], asBuffer(chunks[i]))
            .accounts({ uploader: this.payer })
            .rpc({ commitment: "confirmed" }),
        maxRetries,
        baseDelay
      );
    }

    return prevSig;
  }

  /**
   * Retrieve data from Solana given the last transaction signature.
   * Traverses the linked list backwards to reconstruct the original data,
   * then verifies the SHA256 hash.
   */
  async retrieveData(lastTxSig: string): Promise<Uint8Array> {
    const chunks: Uint8Array[] = [];
    let currentSig = lastTxSig;
    let expectedHash: Uint8Array | null = null;
    let expectedSize: number | null = null;

    while (true) {
      const block = await this.fetchBlock(currentSig);

      if (block.name === "uploadFirst") {
        chunks.push(new Uint8Array(block.data.data));
        expectedHash = new Uint8Array(block.data.sha256Hash);
        expectedSize = block.data.totalSize;
        break;
      } else if (block.name === "uploadContinuation") {
        chunks.push(new Uint8Array(block.data.data));
        currentSig = bs58.encode(new Uint8Array(block.data.prevTxSig));
      } else {
        throw new Error(`Unknown block type: ${block.name}`);
      }
    }

    // Collected last→first, reverse to restore original order
    chunks.reverse();
    const data = concatBytes(chunks);

    // Verify size
    if (data.length !== expectedSize) {
      throw new Error(
        `Size mismatch: expected ${expectedSize} bytes, got ${data.length}`
      );
    }

    // Verify SHA256 hash
    const actualHash = await sha256(data);
    if (!bytesEqual(actualHash, expectedHash!)) {
      throw new Error(
        `SHA256 hash mismatch: expected ${toHex(expectedHash!)}, got ${toHex(actualHash)}`
      );
    }

    return data;
  }

  /**
   * Fetch and decode a program instruction from a transaction.
   */
  private async fetchBlock(
    txSig: string
  ): Promise<{ name: string; data: any }> {
    const tx = await this.connection.getParsedTransaction(txSig, {
      commitment: "confirmed",
      maxSupportedTransactionVersion: 0,
    });
    if (!tx) {
      throw new Error(`Transaction ${txSig} not found`);
    }

    for (const ix of tx.transaction.message.instructions) {
      if ("programId" in ix && ix.programId.equals(this.program.programId)) {
        const raw = bs58.decode((ix as any).data);
        const coder = this.program.coder.instruction as BorshInstructionCoder;
        const decoded = coder.decode(asBuffer(raw));
        if (!decoded) {
          throw new Error("Failed to decode instruction data");
        }
        return decoded;
      }
    }
    throw new Error("Program instruction not found in transaction");
  }

  /**
   * Retry a transaction with exponential backoff.
   */
  private async sendWithRetry(
    fn: () => Promise<string>,
    maxRetries: number,
    baseDelay: number
  ): Promise<string> {
    let lastError: Error | null = null;

    for (let attempt = 0; attempt <= maxRetries; attempt++) {
      try {
        return await fn();
      } catch (err: any) {
        lastError = err;

        // Don't retry program errors (validation failures) — they'll always fail
        if (isProgramError(err)) {
          throw err;
        }

        if (attempt === maxRetries) {
          break;
        }

        const delay = baseDelay * Math.pow(2, attempt);
        const jitter = Math.random() * delay * 0.5;
        await sleep(delay + jitter);
      }
    }

    throw new Error(
      `Transaction failed after ${maxRetries + 1} attempts: ${lastError?.message}`
    );
  }
}

// --- Buffer bridge for Anchor internals ---

/** Convert Uint8Array to Buffer without ambiguous overload issues */
function asBuffer(data: Uint8Array): Buffer {
  return Buffer.from(data.buffer, data.byteOffset, data.byteLength);
}

// --- Universal helpers (no Node.js APIs) ---

/**
 * SHA-256 hash using the Web Crypto API.
 * Works in browsers and Node.js 18.4+.
 */
async function sha256(data: Uint8Array): Promise<Uint8Array> {
  // Cast to ArrayBuffer to satisfy BufferSource typing in TS 5.7+
  // (Uint8Array.buffer is always ArrayBuffer in practice, never SharedArrayBuffer)
  const buf = data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) as ArrayBuffer;
  const hash = await globalThis.crypto.subtle.digest("SHA-256", buf);
  return new Uint8Array(hash);
}

function concatBytes(arrays: Uint8Array[]): Uint8Array {
  const totalLength = arrays.reduce((sum, arr) => sum + arr.length, 0);
  const result = new Uint8Array(totalLength);
  let offset = 0;
  for (const arr of arrays) {
    result.set(arr, offset);
    offset += arr.length;
  }
  return result;
}

function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

function toHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

function isProgramError(err: any): boolean {
  const msg = err?.toString() ?? "";
  return (
    msg.includes("AnchorError") ||
    msg.includes("Error Code:") ||
    msg.includes("Program log: Error")
  );
}

function splitIntoChunks(data: Uint8Array, chunkSize: number): Uint8Array[] {
  const chunks: Uint8Array[] = [];
  for (let i = 0; i < data.length; i += chunkSize) {
    chunks.push(data.subarray(i, Math.min(i + chunkSize, data.length)));
  }
  return chunks;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
