# Solana Data Transfer — Frontend Example

A browser-based UI for uploading and retrieving data on Solana using the Data Transfer protocol.

## Prerequisites

- [Solana CLI](https://docs.solana.com/cli/install-solana-cli-tools) installed
- [Anchor CLI](https://www.anchor-lang.com/docs/installation) installed
- [Node.js](https://nodejs.org/) (v18+) and yarn/npm
- A browser wallet extension: [Phantom](https://phantom.app/) or [Solflare](https://solflare.com/)

## Setup

### 1. Build and deploy the program to localnet

From the project root:

```bash
anchor build
```

### 2. Start the local validator with the program

```bash
anchor localnet
```

This starts `solana-test-validator` and deploys the program. Keep this terminal running.

### 3. Configure Phantom for localnet

Phantom does not connect to `localhost` by default. You need to point it to your local validator:

1. Open Phantom in your browser
2. Go to **Settings** (gear icon)
3. **Developer Settings** → toggle on **Testnet Mode**
4. Go back to **Settings** → **Network** → choose **Add Custom Network** (or **Custom RPC**)
5. Enter the RPC URL: `http://127.0.0.1:8899`
6. Save and select this network

> **Note:** If Phantom does not allow custom RPC or shows errors, try Solflare instead — it has better custom RPC support.

### 4. Airdrop SOL to your wallet

Copy your Phantom wallet address and run:

```bash
solana airdrop 2 <YOUR_PHANTOM_ADDRESS> --url localhost
```

You can repeat this multiple times — localnet has unlimited SOL. Verify the balance:

```bash
solana balance <YOUR_PHANTOM_ADDRESS> --url localhost
```

If Phantom still shows 0 SOL, it is likely not connected to localnet (see step 3).

### 5. Install frontend dependencies

```bash
cd examples/frontend
npm install
```

### 6. Start the dev server

```bash
npm run dev
```

Opens at `http://localhost:5173`.

## Usage

### Send Data

1. Make sure **Localnet** is selected in the cluster dropdown (top-left)
2. Click **Phantom** (or **Solflare**) to connect your wallet
3. Go to the **Send Data** tab
4. Type or paste text into the input box
5. Click **Send** — your wallet will prompt you to approve the transaction(s)
6. Once complete, the last transaction signature is displayed — click **Copy TX** to copy it

### Retrieve Data

1. Go to the **TX to Data** tab
2. Paste a transaction signature into the input
3. Click **Retrieve** — no wallet connection needed (read-only)
4. The original data is displayed

## Troubleshooting

| Problem | Solution |
|---------|----------|
| Phantom shows 0 SOL after airdrop | Phantom is not on localnet. Check Settings → Network → make sure custom RPC is `http://127.0.0.1:8899` |
| "Failed to fetch" or network error | Local validator is not running. Run `anchor localnet` in another terminal |
| Wallet connection fails | Make sure the wallet extension is installed and unlocked |
| `Module "buffer" has been externalized` | Run `npm install` in `examples/frontend` to install the `buffer` polyfill |
| Transaction fails with insufficient funds | Airdrop more SOL: `solana airdrop 2 <ADDRESS> --url localhost` |

## Testing without a browser wallet

You can verify the program works on localnet without the frontend by running the test suite from the project root:

```bash
anchor test
```

This starts a local validator, deploys the program, and runs all integration tests automatically.
