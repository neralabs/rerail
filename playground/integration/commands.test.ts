import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { setTimeout as sleep } from "node:timers/promises";
import { executeSandboxCommand } from "../src/commands.js";

describe.skipIf(process.platform !== "darwin")("sandbox command cancellation", () => {
  it("reclaims background jobs even when Bash job control changes their process group", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-command-cancel-"));
    const controller = new AbortController();
    let backgroundPid: number | undefined;
    let cleanupError: unknown;
    const running = executeSandboxCommand(
      root,
      'set -m; (end=$((SECONDS+15)); while [ "$SECONDS" -lt "$end" ]; do :; done) & printf "%s" "$!" > background.pid; wait',
      controller.signal,
    ).then(
      () => false,
      () => true,
    );
    try {
      const deadline = Date.now() + 2_000;
      while (Date.now() < deadline) {
        const text = await readFile(join(root, "background.pid"), "utf8").catch(() => "");
        if (Number(text) > 0) {
          backgroundPid = Number(text);
          break;
        }
        await sleep(10);
      }
      if (backgroundPid === undefined) throw new Error("Command did not start its background job.");
      const pid = backgroundPid;
      process.kill(pid, 0);
      controller.abort(new Error("Stop requested"));
      const cancelled = await Promise.race([running, sleep(2_000).then(() => "still-running")]);
      expect(cancelled).toBe(true);
      expect(() => process.kill(pid, 0)).toThrow(expect.objectContaining({ code: "ESRCH" }));
    } finally {
      controller.abort();
      // Keep the regression safe even against the old group-only cleanup.
      if (backgroundPid !== undefined) {
        try {
          process.kill(backgroundPid, "SIGKILL");
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code !== "ESRCH") cleanupError = error;
        }
      }
      await running;
      await rm(root, { recursive: true, force: true });
    }
    if (cleanupError !== undefined) throw cleanupError;
  });
});
