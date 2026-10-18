import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import type { Plugin } from "vite";
import { defineConfig } from "vitest/config";
import { brandIcons, favicon } from "./brand-icons.ts";

/**
 * Serves and emits /favicon.svg from @hyperstructure/design, and PNG icons
 * drawn from its mark (brand-icons.ts): one source for the mark.
 */
function brandAssets(): Plugin {
  let icons: Record<string, Buffer> | undefined;
  const pngs = () => {
    icons ??= brandIcons();
    return icons;
  };
  return {
    name: "brand-assets",
    configureServer(server) {
      server.middlewares.use("/favicon.svg", (_req, res) => {
        res.setHeader("content-type", "image/svg+xml");
        res.end(favicon());
      });
      server.middlewares.use((req, res, next) => {
        const icon = pngs()[req.url?.slice(1) ?? ""];
        if (!icon) return next();
        res.setHeader("content-type", "image/png");
        res.end(icon);
      });
    },
    generateBundle() {
      this.emitFile({ type: "asset", fileName: "favicon.svg", source: favicon() });
      for (const [fileName, source] of Object.entries(pngs()))
        this.emitFile({ type: "asset", fileName, source });
    },
  };
}

// In development the control plane runs on :8787 (`cargo xtask dev`).
const api = process.env.FCTL_API ?? "http://127.0.0.1:8787";

export default defineConfig({
  plugins: [react(), tailwindcss(), brandAssets()],
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
