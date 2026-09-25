import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import preact from "@preact/preset-vite";

// Separate build config for the design preview mode (see
// docs/DESIGN_PREVIEW.md). `pnpm build` (the production build, run by
// CI) never uses this file and never touches design.html or
// src/design-main.tsx, so the preview never ships. This is only for
// `pnpm build:design`, used by scripts/screenshots.mjs so shots can run
// against a built, non-dev-server page.
export default defineConfig({
  plugins: [preact()],

  resolve: {
    alias: {
      react: "preact/compat",
      "react-dom": "preact/compat",
      "react/jsx-runtime": "preact/jsx-runtime",
    },
  },

  build: {
    target: "es2022",
    outDir: "dist-design",
    sourcemap: false,
    rollupOptions: {
      input: fileURLToPath(new URL("./design.html", import.meta.url)),
    },
  },
});
