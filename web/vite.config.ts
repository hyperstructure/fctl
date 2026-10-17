import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// In development the control plane runs on :8787 (`cargo xtask dev`).
const api = process.env.FCTL_API ?? "http://127.0.0.1:8787";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  server: {
    port: 5173,
    proxy: { "/api": { target: api, changeOrigin: false, ws: true } },
  },
  build: {
    target: "es2023",
    sourcemap: true,
    // Never inline assets as data: URIs (the CSP allows fonts and images from 'self' only).
    assetsInlineLimit: 0,
    chunkSizeWarningLimit: 900,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes("node_modules/react") || id.includes("scheduler")) return "react";
          if (id.includes("@tanstack")) return "tanstack";
          return undefined;
        },
      },
    },
  },
  test: { environment: "happy-dom" },
});
