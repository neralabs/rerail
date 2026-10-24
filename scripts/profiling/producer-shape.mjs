#!/usr/bin/env node
// Run the execution-shape gate. A production-only addon cannot silently skip it.
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
const result = spawnSync(process.execPath, [
  fileURLToPath(new URL("../../node_modules/vitest/vitest.mjs", import.meta.url)),
  "run", "test/engine/native/frontier-determinism.test.ts", "test/engine/native/shared-producers.test.ts", "test/engine/native/compiler-schema.test.ts", "test/engine/native/candidate-publication.test.ts", "test/engine/native/candidate-admission.test.ts", "test/engine/native/relational-batch.test.ts", "test/engine/native/relational-completed.test.ts",
], {
  cwd: fileURLToPath(new URL("../../", import.meta.url)),
  env: { ...process.env, RERAIL_REQUIRE_PRODUCER_SHAPE: "1" },
  stdio: "inherit", timeout: 110_000,
});
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
