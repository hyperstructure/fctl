import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import type { Plugin } from "vite";
import { defineConfig } from "vitest/config";

const require = createRequire(import.meta.url);

/** The favicon and app icons at the root, and where they are in @hyperstructure/design. */
export const BRAND_ASSETS: Record<string, string> = {
  "favicon.svg": "assets/favicon.svg",
  "apple-touch-icon.png": "assets/icons/apple-touch-icon.png",
  "icon-192.png": "assets/icons/icon-192.png",
  "icon-512.png": "assets/icons/icon-512.png",
  "icon-maskable-512.png": "assets/icons/icon-maskable-512.png",
  "icon-maskable-1024.png": "assets/icons/icon-maskable-1024.png",
};

/** Serves and emits the favicon and app icons from @hyperstructure/design (one source for the mark). */
function brandAssets(): Plugin {
  const read = (file: string) =>
    readFileSync(require.resolve(`@hyperstructure/design/${BRAND_ASSETS[file]}`));
  return {
    name: "brand-assets",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const file = req.url?.split("?")[0]?.slice(1) ?? "";
        if (!Object.hasOwn(BRAND_ASSETS, file)) return next();
        res.setHeader("content-type", file.endsWith(".svg") ? "image/svg+xml" : "image/png");
        res.end(read(file));
      });
    },
    generateBundle() {
      for (const file of Object.keys(BRAND_ASSETS)) {
        this.emitFile({ type: "asset", fileName: file, source: read(file) });
      }
    },
  };
}

/** fctl's guide for agents, which every instance serves at /llms.txt as fctl.sh does: one copy, in docs/. */
export const LLMS_TXT = fileURLToPath(new URL("../docs/llms.txt", import.meta.url));

/** Serves and emits /llms.txt from docs/llms.txt. */
export function llmsTxt(): Plugin {
  return {
    name: "llms-txt",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (req.url?.split("?")[0] !== "/llms.txt") return next();
        res.setHeader("content-type", "text/plain; charset=utf-8");
        res.end(readFileSync(LLMS_TXT));
      });
    },
    generateBundle() {
      this.emitFile({ type: "asset", fileName: "llms.txt", source: readFileSync(LLMS_TXT) });
    },
  };
}

// In development the control plane runs on :8787 (`cargo xtask dev`).
const api = process.env.FCTL_API ?? "http://127.0.0.1:8787";

export default defineConfig({
  plugins: [react(), tailwindcss(), brandAssets(), llmsTxt()],
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
