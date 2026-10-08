import { cpSync } from "node:fs";
import { resolve } from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, type Plugin } from "vite";

/**
 * The playground owns no design system. `@` aliases to nera-landing's `web/src`, so components,
 * theme tokens, and fonts all come from the enclosing checkout; this package holds only the
 * playground feature and this shell. React and friends resolve from nera-landing's root
 * `node_modules` (web is its workspace), which also means `npm ci` at the repo root is a
 * prerequisite — `just rerail-playground` runs it.
 *
 * Both this package and nera-landing carry a React; Vite would otherwise resolve this shell's bare
 * `react` import here while aliased components resolve theirs one level up. Two Reacts means every
 * hook in a borrowed component throws "Invalid hook call" at runtime while the build stays green,
 * so anything carrying shared state is pinned to the nera-landing copy.
 */
const playgroundRoot = resolve(import.meta.dirname, "..");
const neraLanding = resolve(playgroundRoot, "..");
const neraWeb = resolve(neraLanding, "web");
const SINGLETONS = ["react", "react-dom", "@tanstack/react-query", "next-themes", "radix-ui"];
const outDir = resolve(playgroundRoot, "dist/ui");

const copyBundledPublicAssets = (): Plugin => ({
  name: "playground-copy-bundled-public-assets",
  apply: "build",
  closeBundle() {
    cpSync(resolve(neraWeb, "public/assets/fonts"), resolve(outDir, "assets/fonts"), { recursive: true });
    cpSync(resolve(neraWeb, "public/favicon.svg"), resolve(outDir, "favicon.svg"));
  },
});

/**
 * `dev` serves this client on its own port and forwards the control socket, rule list, and OpenAPI
 * document to a running supervisor. The built client is served by the supervisor itself, so both
 * modes keep the browser on one origin.
 */
const supervisor = `http://127.0.0.1:${process.env["RERAIL_PLAYGROUND_PORT"] ?? "4741"}`;

export default defineConfig({
  root: import.meta.dirname,
  plugins: [react(), tailwindcss(), copyBundledPublicAssets()],
  resolve: {
    dedupe: SINGLETONS,
    alias: {
      "@protocol": resolve(playgroundRoot, "src/protocol.ts"),
      "@rerail/rule-schema": resolve(neraLanding, "rerail/src/rule-schema.ts"),
      "@rerail/code-rule-schema": resolve(neraLanding, "rerail/src/code-rule-schema.ts"),
      "@": resolve(neraWeb, "src"),
      "@shared": resolve(neraWeb, "shared"),
      ...Object.fromEntries(SINGLETONS.map((name) => [name, resolve(neraLanding, "node_modules", name)])),
    },
  },
  // Serves nera-landing's /assets/fonts/*, which its fonts.css references by absolute path.
  publicDir: resolve(neraWeb, "public"),
  build: {
    outDir,
    emptyOutDir: true,
    sourcemap: true,
    // nera-landing's public dir also carries its service worker, runtime config, and marketing
    // images. Only the fonts and favicon belong here; copyBundledPublicAssets copies those.
    copyPublicDir: false,
  },
  server: {
    port: 4742,
    host: "127.0.0.1",
    strictPort: true,
    fs: { allow: [neraLanding] },
    proxy: {
      "/__rerail_playground": { target: supervisor, ws: true },
    },
  },
});
