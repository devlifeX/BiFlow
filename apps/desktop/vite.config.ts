import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";
import { detectLocalHiddify, detectLocalMihomo } from "./local-deps";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const appVersion = readFileSync(resolve(root, "version"), "utf8").trim();

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  define: {
    __APP_VERSION__: JSON.stringify(appVersion),
    __MOCK_HIDDIFY_INSTALLED__: detectLocalHiddify(),
    __MOCK_MIHOMO_INSTALLED__: detectLocalMihomo(),
  },
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: ["es2021", "chrome100", "safari13"],
    minify: process.env.TAURI_DEBUG ? false : "esbuild",
    sourcemap: Boolean(process.env.TAURI_DEBUG),
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes("/node_modules/")) return undefined;
          if (/\/node_modules\/(react|react-dom|scheduler)\//.test(id)) {
            return "react-vendor";
          }
          return "vendor";
        },
      },
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
    isolate: true,
    css: true,
    exclude: ["**/node_modules/**", "**/e2e/**", "**/dist/**"],
    coverage: {
      provider: "v8",
      reporter: ["text", "html"],
    },
  },
});
