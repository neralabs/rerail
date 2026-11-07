import { resolve } from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

/**
 * The client is self-contained: its few shadcn-style primitives live in `ui/src/components/ui`
 * (`@` aliases to `ui/src`), React and friends come from this package's own `node_modules`, and
 * the colour tokens are the daemon page's `ui/tokens.generated.css` at the checkout root. Rule
 * shapes come from the built package's `@neralabs/rerail/schemas` entry point, so the client parses
 * exactly what the daemon saves.
 */
const playgroundRoot = resolve(import.meta.dirname, "..");
const checkoutRoot = resolve(playgroundRoot, "..");
const outDir = resolve(playgroundRoot, "dist/ui");

/**
 * `dev` serves this client on its own port and forwards the control socket, rule list, and OpenAPI
 * document to a running supervisor. The built client is served by the supervisor itself, so both
 * modes keep the browser on one origin.
 */
const supervisor = `http://127.0.0.1:${process.env["RERAIL_PLAYGROUND_PORT"] ?? "4741"}`;

export default defineConfig({
  root: import.meta.dirname,
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@protocol": resolve(playgroundRoot, "src/protocol.ts"),
      "@neralabs/rerail/schemas": resolve(checkoutRoot, "dist/engine/schemas.js"),
      "@": resolve(import.meta.dirname, "src"),
    },
  },
  publicDir: false,
  build: {
    outDir,
    emptyOutDir: true,
    sourcemap: true,
  },
  server: {
    port: 4742,
    host: "127.0.0.1",
    strictPort: true,
    fs: { allow: [playgroundRoot, resolve(checkoutRoot, "ui"), resolve(checkoutRoot, "dist")] },
    proxy: {
      "/__rerail_playground": { target: supervisor, ws: true },
    },
  },
});
