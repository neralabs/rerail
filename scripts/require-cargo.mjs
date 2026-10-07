// Preflight for build:native: napi reports a missing cargo only as "cargo metadata failed to run".
import { spawnSync } from "node:child_process";

if (spawnSync("cargo", ["--version"], { stdio: "ignore" }).error !== undefined) { // NOSONAR - this preflight exists to check cargo is on PATH
  console.error('cargo not found on PATH; run `export PATH="$HOME/.cargo/bin:$PATH"` (or `just rerail-native` to install Rust)');
  process.exit(1);
}
