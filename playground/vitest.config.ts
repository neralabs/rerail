import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // Rerail adapters autostart the daemon on session start; nothing here may spawn one into ~/.rerail.
    env: { RERAIL_AUTOSTART: "0" },
    // Sandbox and process tests mutate process state; forked workers keep test files from racing.
    pool: "forks",
  },
});
