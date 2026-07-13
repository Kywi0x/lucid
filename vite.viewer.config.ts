import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// Build du viewer public (site/s) — mêmes composants que l'app, cible navigateur.
// `base: "./"` : servi sous /lucid/s/ sur GitHub Pages, les assets sont relatifs.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  base: "./",
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  build: {
    outDir: "dist-viewer",
    rollupOptions: { input: path.resolve(__dirname, "viewer.html") },
  },
});
