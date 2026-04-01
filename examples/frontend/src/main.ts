import { Buffer } from "buffer";
globalThis.Buffer = Buffer as any;
import { Connection } from "@solana/web3.js";
import { AnchorProvider, Program } from "@coral-xyz/anchor";
import { BaseMessageSignerWalletAdapter } from "@solana/wallet-adapter-base";
import { PhantomWalletAdapter } from "@solana/wallet-adapter-phantom";
import { SolflareWalletAdapter } from "@solana/wallet-adapter-solflare";
import { DataTransferClient } from "./client";
import idl from "./idl.json";

// --- DOM helpers ---
const $ = (id: string) => document.getElementById(id)!;

function setResult(
  id: string,
  msg: string,
  type: "idle" | "info" | "success" | "error"
) {
  const el = $(id);
  el.textContent = msg;
  el.className = `result ${type}`;
}

// --- State ---
let client: DataTransferClient | null = null;
let activeAdapter: BaseMessageSignerWalletAdapter | null = null;

// --- Wallets ---
const wallets: BaseMessageSignerWalletAdapter[] = [
  new PhantomWalletAdapter(),
  new SolflareWalletAdapter(),
];

function getConnection(): Connection {
  return new Connection(($("cluster") as HTMLSelectElement).value, "confirmed");
}

function updateWalletUI() {
  const info = $("wallet-info");
  if (activeAdapter?.publicKey) {
    const addr = activeAdapter.publicKey.toBase58();
    info.textContent = addr.slice(0, 4) + ".." + addr.slice(-4);
  } else {
    info.textContent = "";
  }

  const connected = !!activeAdapter?.publicKey;
  ($("send-btn") as HTMLButtonElement).disabled = !connected;
  // Retrieve doesn't require a wallet connection (read-only)
  ($("retrieve-btn") as HTMLButtonElement).disabled = false;

  if (!connected) {
    setResult("send-result", "Connect a wallet to send data.", "idle");
  }
}

async function connectWallet(adapter: BaseMessageSignerWalletAdapter) {
  if (activeAdapter?.connected) {
    await activeAdapter.disconnect();
  }

  try {
    await adapter.connect();
  } catch (err: any) {
    setResult("send-result", `Connection failed: ${err.message}`, "error");
    return;
  }

  if (!adapter.publicKey) {
    setResult("send-result", "Wallet connected but returned no public key.", "error");
    return;
  }

  activeAdapter = adapter;
  rebuildClient();
  updateWalletUI();
  setResult(
    "send-result",
    `Wallet connected: ${adapter.publicKey.toBase58()}`,
    "success"
  );
}

async function disconnectWallet() {
  if (activeAdapter?.connected) {
    await activeAdapter.disconnect();
  }
  activeAdapter = null;
  client = null;
  updateWalletUI();
}

function rebuildClient() {
  if (!activeAdapter?.publicKey) return;
  const connection = getConnection();
  const provider = new AnchorProvider(connection, activeAdapter as any, {
    commitment: "confirmed",
  });
  const program = new Program(idl as any, provider);
  client = new DataTransferClient(program, connection, activeAdapter.publicKey);
}

// Rebuild client when cluster changes (keep wallet connected)
$("cluster").addEventListener("change", () => {
  if (activeAdapter?.publicKey) rebuildClient();
});

// --- Send ---
async function send() {
  if (!client || !activeAdapter?.publicKey) {
    setResult("send-result", "Connect a wallet first.", "error");
    return;
  }

  const input = ($("send-input") as HTMLTextAreaElement).value;
  if (!input) {
    setResult("send-result", "Enter some data to send.", "error");
    return;
  }

  const data = new TextEncoder().encode(input);
  const numChunks = Math.ceil(data.length / 800);
  setResult(
    "send-result",
    `Uploading ${data.length} bytes (${numChunks} transaction${numChunks > 1 ? "s" : ""})...`,
    "info"
  );

  try {
    const lastTx = await client.uploadData(data);

    setResult(
      "send-result",
      `Sent ${data.length} bytes in ${numChunks} transaction${numChunks > 1 ? "s" : ""}.`,
      "success"
    );

    // Show TX with copy button
    const txRow = $("send-tx-row");
    txRow.style.display = "flex";
    ($("send-tx") as HTMLInputElement).value = lastTx;
  } catch (err: any) {
    setResult("send-result", `Send failed: ${err.message}`, "error");
  }
}

// --- Retrieve ---
async function retrieve() {
  const txSig = ($("retrieve-input") as HTMLInputElement).value.trim();
  if (!txSig) {
    setResult("retrieve-result", "Enter a transaction signature.", "error");
    return;
  }

  setResult("retrieve-result", "Retrieving data from chain...", "info");

  try {
    // Retrieve is read-only — use a bare connection + program (no wallet needed)
    const connection = getConnection();
    const provider = new AnchorProvider(
      connection,
      // Dummy wallet for read-only — retrieveData never signs
      { publicKey: null, signTransaction: async (t: any) => t, signAllTransactions: async (t: any) => t } as any,
      { commitment: "confirmed" }
    );
    const program = new Program(idl as any, provider);
    const readClient = new DataTransferClient(
      program,
      connection,
      null as any
    );

    const data = await readClient.retrieveData(txSig);
    const text = new TextDecoder().decode(data);
    setResult(
      "retrieve-result",
      `Retrieved ${data.length} bytes:\n\n${text}`,
      "success"
    );
  } catch (err: any) {
    setResult("retrieve-result", `Retrieval failed: ${err.message}`, "error");
  }
}

// --- Tabs ---
function initTabs() {
  const tabs = $("tabs").querySelectorAll("button");
  tabs.forEach((tab) => {
    tab.addEventListener("click", () => {
      tabs.forEach((t) => t.classList.remove("active"));
      tab.classList.add("active");
      document
        .querySelectorAll(".panel")
        .forEach((p) => p.classList.remove("active"));
      $(`panel-${tab.dataset.tab}`).classList.add("active");
    });
  });
}

// --- Wallet buttons ---
function initWalletButtons() {
  const area = $("wallet-area");

  for (const adapter of wallets) {
    const btn = document.createElement("button");
    btn.textContent = adapter.name;
    btn.addEventListener("click", () => {
      if (activeAdapter === adapter && adapter.connected) {
        disconnectWallet();
      } else {
        connectWallet(adapter);
      }
    });
    area.insertBefore(btn, $("wallet-info"));
  }
}

// --- Copy TX ---
$("send-tx-copy").addEventListener("click", () => {
  const tx = ($("send-tx") as HTMLInputElement).value;
  navigator.clipboard.writeText(tx);
  $("send-tx-copy").textContent = "Copied!";
  setTimeout(() => {
    $("send-tx-copy").textContent = "Copy TX";
  }, 1500);
});

// --- Init ---
initTabs();
initWalletButtons();
updateWalletUI();
$("send-btn").addEventListener("click", send);
$("retrieve-btn").addEventListener("click", retrieve);
