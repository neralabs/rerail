import { resolve } from "node:path";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

const playgroundRoot = resolve(import.meta.dirname, "..");
const checkoutRoot = resolve(playgroundRoot, "..");

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@protocol": resolve(playgroundRoot, "src/protocol.ts"),
      "@rerail/rule-schema": resolve(checkoutRoot, "src/rule-schema.ts"),
      "@rerail/code-rule-schema": resolve(checkoutRoot, "src/code-rule-schema.ts"),
      "@": resolve(import.meta.dirname, "src"),
    },
  },
  test: {
    root: import.meta.dirname,
    environment: "jsdom",
    include: ["src/**/*.test.tsx", "src/**/*.test.ts"],
  },
});
