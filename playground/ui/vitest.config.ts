import { resolve } from "node:path";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

const playgroundRoot = resolve(import.meta.dirname, "..");
const neraLanding = resolve(playgroundRoot, "..");
const SINGLETONS = ["react", "react-dom", "@tanstack/react-query", "next-themes", "radix-ui"];

/** The client renders nera-landing components, so its tests need that checkout's React, not this package's. */
export default defineConfig({
  plugins: [react()],
  resolve: {
    dedupe: SINGLETONS,
    alias: {
      "@protocol": resolve(playgroundRoot, "src/protocol.ts"),
      "@rerail/rule-schema": resolve(neraLanding, "rerail/src/rule-schema.ts"),
      "@rerail/code-rule-schema": resolve(neraLanding, "rerail/src/code-rule-schema.ts"),
      "@": resolve(neraLanding, "web/src"),
      "@shared": resolve(neraLanding, "web/shared"),
      ...Object.fromEntries(SINGLETONS.map((name) => [name, resolve(neraLanding, "node_modules", name)])),
    },
  },
  test: {
    root: import.meta.dirname,
    environment: "jsdom",
    include: ["src/**/*.test.tsx", "src/**/*.test.ts"],
  },
});
