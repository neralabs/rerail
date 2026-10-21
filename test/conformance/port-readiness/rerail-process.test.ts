import assert from "node:assert/strict";
import { test } from "vitest";

import { ProcessCommandRunner } from "../../../conformance/port-readiness/process.js";

test("a child that exits without reading its input settles the run instead of ending the process with EPIPE", async () => {
  // More than a pipe buffer holds, so the write is still pending when the child exits.
  const input = "x".repeat(8 << 20);
  const settled = await new ProcessCommandRunner()
    .run([process.execPath, "-e", "process.exit(3)"], { input })
    .then((result) => result.exitCode, (error: NodeJS.ErrnoException) => error.code);
  assert.ok(settled === 3 || settled === "EPIPE", `settled with ${String(settled)}`);
});
