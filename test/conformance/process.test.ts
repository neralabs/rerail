import assert from "node:assert/strict";
import { test } from "vitest";

import { isolatedGitEnv, ProcessCommandRunner } from "../../conformance/process.js";

test("a child that exits without reading its input settles the run instead of ending the process with EPIPE", async () => {
  // More than a pipe buffer holds, so the write is still pending when the child exits.
  const input = "x".repeat(8 << 20);
  const settled = await new ProcessCommandRunner()
    .run([process.execPath, "-e", "process.exit(3)"], { input })
    .then((result) => result.exitCode, (error: NodeJS.ErrnoException) => error.code);
  // Windows reports the write to a pipe the child closed as EOF, where Linux and macOS report EPIPE.
  const closedPipe = process.platform === "win32" ? "EOF" : "EPIPE";
  assert.ok(settled === 3 || settled === closedPipe, `settled with ${String(settled)}`);
});

test("a Git command on a repository of its own reads none of the variables a hook exports for another one", () => {
  const env = isolatedGitEnv({ GIT_AUTHOR_NAME: "rerail" }, { PATH: "/bin", GIT_DIR: "/outer/.git", GIT_WORK_TREE: "/outer", GIT_INDEX_FILE: "/outer/.git/index" });
  assert.deepEqual(env, { PATH: "/bin", GIT_CONFIG_GLOBAL: "/dev/null", GIT_CONFIG_NOSYSTEM: "1", GIT_AUTHOR_NAME: "rerail" });
});
