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
      // Mirrors the tsconfig paths the vendored @mistral/ui components use.
      "#components": r("./shared/ui/components"),
      "#utils": r("./shared/ui/utils"),
      "#hooks": r("./shared/ui/hooks"),
      "#branding": r("./shared/ui/branding"),
      "@ui": r("./shared/ui"),
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
