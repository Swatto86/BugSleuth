import { defineConfig } from "vite";

// Vanilla TypeScript, no framework. The UI is a handful of panels over a Rust
// engine; React would be more machinery than the problem has.
export default defineConfig({
  root: "ui",
  build: { outDir: "dist", emptyOutDir: true, target: "esnext" },
  server: { port: 5173, strictPort: true },
  clearScreen: false,
});
