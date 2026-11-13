import { defineConfig } from "vitest/config";

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
    // Windows starts a process and opens a file several times slower than Linux and macOS: a test that starts git, a
    // session runtime and a daemon takes 2 to 9 s there in a whole run, past the 5 s default, so the limits are wider.
    ...(process.platform === "win32" ? { testTimeout: 30_000, hookTimeout: 60_000 } : {}),
    setupFiles: ["./test/support/setup-rerail-home.ts"],
    // Not vitest suites: upstream tools' copied tests are fixture data for the expected findings, real-code samples
    // extracted under .rerail/ include upstream projects' test files, and ensure-sqlite's and the transducer design's
    // tests run under `node --test` (`npm run test:scripts`). `npm test` scans with `--dir test`, against which a
    // pattern that starts with `test/` matches nothing: each pattern here starts with `**/` or a folder outside test/.
    exclude: [
      "**/node_modules/**",
      "**/dist/**",
      "fixtures/**",
      ".rerail/**",
      "packs/*/upstream/**",
      "packs/*/candidates/**",
      "**/scripts/ensure-sqlite.test.mjs",
      "**/docs/transducer-design.test.mjs",
    ],
  },
});
