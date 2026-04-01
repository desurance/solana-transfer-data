import { defineConfig } from "vite";

export default defineConfig({
  define: {
    // Required by @solana/web3.js in browser
    "process.env": {},
    global: "globalThis",
  },
  resolve: {
    alias: {
      buffer: "buffer",
    },
  },
  optimizeDeps: {
    esbuildOptions: {
      // Inject Buffer globally for dependencies that expect it
      define: { global: "globalThis" },
    },
  },
});
