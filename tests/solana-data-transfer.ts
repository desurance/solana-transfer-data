import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { SolanaDataTransfer } from "../target/types/solana_data_transfer";
import { DataTransferClient } from "../app/client";
import { assert } from "chai";
import * as crypto from "crypto";

describe("solana-data-transfer", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace
    .solanaDataTransfer as Program<SolanaDataTransfer>;

  const client = new DataTransferClient(
    program,
    provider.connection,
    provider.wallet.publicKey
  );

  const CHUNK_SIZE = 800;

  /** Convert Uint8Array to Buffer for test assertions */
  function toBuffer(data: Uint8Array): Buffer {
    return Buffer.from(data.buffer, data.byteOffset, data.byteLength);
  }

  describe("uploadData / retrieveData", () => {
    it("single-block data", async () => {
      const testData = Buffer.from("Hello, Solana data transfer protocol!");

      const lastTx = await client.uploadData(testData);
      console.log(`      tx: ${lastTx}`);

      const result = await client.retrieveData(lastTx);
      assert.deepEqual(toBuffer(result), testData);
    });

    it("two-block data", async () => {
      const testData = Buffer.alloc(CHUNK_SIZE + 50);
      for (let i = 0; i < testData.length; i++) {
        testData[i] = (i * 7 + 13) % 256;
      }

      const lastTx = await client.uploadData(testData, {
        chunkSize: CHUNK_SIZE,
      });
      const result = await client.retrieveData(lastTx);
      assert.deepEqual(toBuffer(result), testData);
    });

    it("multi-block data (4 blocks)", async () => {
      const testData = Buffer.alloc(CHUNK_SIZE * 3 + 100);
      for (let i = 0; i < testData.length; i++) {
        testData[i] = i % 256;
      }

      const lastTx = await client.uploadData(testData, {
        chunkSize: CHUNK_SIZE,
      });
      console.log(`      4 blocks, last tx: ${lastTx}`);

      const result = await client.retrieveData(lastTx);
      assert.deepEqual(toBuffer(result), testData);
    });

    it("verifies SHA256 hash on retrieval", async () => {
      const testData = Buffer.from("Integrity check test data");
      const expectedHash = crypto
        .createHash("sha256")
        .update(testData)
        .digest();

      const lastTx = await client.uploadData(testData);
      const result = await client.retrieveData(lastTx);

      const actualHash = crypto
        .createHash("sha256")
        .update(result)
        .digest();
      assert.deepEqual(actualHash, expectedHash);
    });

    it("accepts Uint8Array input (browser-style)", async () => {
      const testData = new TextEncoder().encode(
        "Browser-compatible Uint8Array test"
      );

      const lastTx = await client.uploadData(testData);
      const result = await client.retrieveData(lastTx);

      assert.equal(
        new TextDecoder().decode(result),
        "Browser-compatible Uint8Array test"
      );
    });
  });

  describe("program validation", () => {
    it("rejects empty data in first block", async () => {
      try {
        await program.methods
          .uploadFirst(10, new Array(32).fill(0), Buffer.from([]))
          .accounts({ uploader: provider.wallet.publicKey })
          .rpc();
        assert.fail("Expected error for empty data");
      } catch (err: any) {
        assert.include(err.toString(), "EmptyData");
      }
    });

    it("rejects zero total size", async () => {
      try {
        await program.methods
          .uploadFirst(0, new Array(32).fill(0), Buffer.from([1]))
          .accounts({ uploader: provider.wallet.publicKey })
          .rpc();
        assert.fail("Expected error for zero total size");
      } catch (err: any) {
        assert.include(err.toString(), "InvalidDataSize");
      }
    });

    it("rejects data exceeding total size", async () => {
      try {
        await program.methods
          .uploadFirst(2, new Array(32).fill(0), Buffer.from([1, 2, 3]))
          .accounts({ uploader: provider.wallet.publicKey })
          .rpc();
        assert.fail("Expected error for oversized data");
      } catch (err: any) {
        assert.include(err.toString(), "DataExceedsTotalSize");
      }
    });

    it("rejects empty data in continuation block", async () => {
      try {
        await program.methods
          .uploadContinuation(new Array(64).fill(0), Buffer.from([]))
          .accounts({ uploader: provider.wallet.publicKey })
          .rpc();
        assert.fail("Expected error for empty data");
      } catch (err: any) {
        assert.include(err.toString(), "EmptyData");
      }
    });
  });

  describe("client validation", () => {
    it("rejects empty input buffer", async () => {
      try {
        await client.uploadData(new Uint8Array(0));
        assert.fail("Expected error for empty data");
      } catch (err: any) {
        assert.include(err.message, "Data cannot be empty");
      }
    });
  });
});
