import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In dev, API and media calls go to a running `courseshelf-serve`.
export default defineConfig({
  plugins: [react()],
  build: { outDir: "dist", emptyOutDir: true },
  server: { proxy: { "/api": "http://127.0.0.1:8765", "/media": "http://127.0.0.1:8765" } },
});
