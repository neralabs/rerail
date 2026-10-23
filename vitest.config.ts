import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // Adapters autostart the daemon on session start; tests exercise sessions constantly and
    // must never spawn a real daemon into ~/.rerail. Subprocess tests inherit this through process.env.
    // RERAIL_DEVICE_SECRET_STORE keeps activation tests, and every CLI they spawn, out of the system keychain.
    env: { RERAIL_AUTOSTART: "0", RERAIL_DEVICE_SECRET_STORE: "file" },
    // The setup file mutates process.env to give every test file its own catalog home. Forked
    // workers prevent that environment from racing with concurrently executing test files.
    pool: "forks",
    setupFiles: ["./test/support/setup-rerail-home.ts"],
    // Upstream tools' copied tests are fixture data for conformance recordings.
    // SQLite setup and SQL profiling tests use `node --test` in their dedicated npm scripts.
    exclude: [
      "**/node_modules/**",
      "**/dist/**",
      "fixtures/**",
      "packs/*/upstream/**",
      "packs/*/candidates/**",
      "**/scripts/ensure-sqlite.test.mjs",
      "**/scripts/sql-profile.test.mjs",
    ],
  },
});
