import { defineConfig } from "vite";

export default defineConfig({
  root: "ui",
  build: {
    outDir: "../ui-dist",
    emptyOutDir: true
  },
  server: {
    strictPort: true
  }
});
