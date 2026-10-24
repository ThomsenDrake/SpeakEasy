import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";

const r = (p: string) => fileURLToPath(new URL(p, import.meta.url));

export default defineConfig({
  // Relative asset paths so the built app loads from Tauri's webview protocol.
  base: "./",
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      // shadcn-style alias used by the vendored motion-primitives components.
      "@": r("./src"),
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  server: {
    port: 5173,
    strictPort: true,
  },
});
