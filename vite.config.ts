import { defineConfig } from "vite";
import preact from "@preact/preset-vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [preact()],

  // Alias react-dom and react to preact/compat so existing imports keep working
  // without rewriting components. Preact + compat is ~6 KB gzipped vs React's ~45 KB.
  resolve: {
    alias: {
      react: "preact/compat",
      "react-dom": "preact/compat",
      "react/jsx-runtime": "preact/jsx-runtime",
    },
  },

  build: {
    target: "es2022",
    // Smaller output without sourcemaps in release; Tauri picks up dist/
    sourcemap: false,
    minify: "esbuild",
    cssCodeSplit: false,
    rollupOptions: {
      output: {
        manualChunks: undefined,
      },
    },
  },

  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
