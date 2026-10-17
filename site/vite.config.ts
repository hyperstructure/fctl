import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

const page = (p: string) => fileURLToPath(new URL(p, import.meta.url));

export default defineConfig({
  plugins: [tailwindcss()],
  build: {
    target: "es2023",
    // Fonts and images come from 'self' only (see public/_headers).
    assetsInlineLimit: 0,
    rollupOptions: {
      // docs.html is the template scripts/docs.mjs fills for every page in docs/.
      input: { index: page("./index.html"), docs: page("./docs.html"), notFound: page("./404.html") },
    },
  },
  server: { port: 5174, fs: { allow: [".."] } },
});
