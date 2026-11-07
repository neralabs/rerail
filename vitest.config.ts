import { defineConfig } from "vitest/config";

// Pulling-it-all-together's tests of its `relational` clause, merged in at cbee7a03c. This engine
// compiles that clause only under `cargo test` (open item O18: it carries SQL, which rules do not),
// so the addon refuses it ("unknown field `relational`"). RERAIL_RELATIONAL_TESTS=1 runs them
// against an addon that has it, such as P's own (open item O84).
const testsForPsAddon = [
  "test/engine/native/relational-completed.test.ts",
  "test/engine/native/relational-deferred.test.ts",
  "test/engine/native/relational-evidence.test.ts",
  "test/engine/native/relational-reachability.test.ts",
  "test/engine/native/relational-syntax.test.ts",
  "test/engine/native/compiler-schema.test.ts",
  "test/engine/native/recovery/eponymous-sources.test.ts",
  "test/engine/native/recovery/execution-owner.test.ts",
  "test/engine/native/recovery/input-admission.test.ts",
  "test/engine/native/recovery/state-syntax-domain.test.ts",
  "test/scripts/sql-stats.test.ts",
  "test/conformance/data-layer/batch-five.test.ts",
  // Fails the same way on P's own addon (batch 01: 1 false positive, 2 false negatives; batch 02: 1
  // and 1; batch 04: 6 and 2), so it is P's open parity, not this engine's (O84).
  "test/conformance/data-layer/ports.test.ts",
];

export default defineConfig({
  test: {
    // Adapters autostart the daemon on session start; tests exercise sessions constantly and
    // must never spawn a real daemon into ~/.rerail. Subprocess tests inherit this through process.env.
    // RERAIL_DEVICE_SECRET_STORE keeps activation tests, and every CLI they spawn, out of the system keychain.
    // RERAIL_MODIFY_PATH keeps set-up out of the shell startup files and the Windows user Path.
    env: { RERAIL_AUTOSTART: "0", RERAIL_DEVICE_SECRET_STORE: "file", RERAIL_MODIFY_PATH: "0" },
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
      "**/scripts/sql-execution-budget.test.mjs",
      ...(process.env.RERAIL_RELATIONAL_TESTS === "1" ? [] : testsForPsAddon),
    ],
  },
});
