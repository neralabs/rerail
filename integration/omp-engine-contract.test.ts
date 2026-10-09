import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { linkSteers, steerBlock } from "../src/engine/index.js";
import { dashboardOrigin } from "../src/presentation/index.js";
import { CONTRACT_CURSOR_EXECUTE_MESSAGE, CONTRACT_CURSOR_EXECUTE_RULE_ID } from "./support/contract-code-rule.js";
import { OmpHost } from "./support/omp-host.js";

describe("OMP native code rule contract", () => {
  it("delivers code rule guidance before the next model call despite delayed execution notifications", async () => {
    const cwd = await mkdtemp(join(tmpdir(), "rerail-omp-kernel-"));
    let host: OmpHost | undefined;
    try {
      await mkdir(join(cwd, "api"));
      await writeFile(join(cwd, "api", "route.py"), "def route():\n    return 1\n");
      const input = { path: "api/route.py", content: "def route(cursor):\n    cursor.execute('SELECT 1')\n" };
      host = await OmpHost.start(cwd, [
        { content: [{ type: "toolCall", name: "write", arguments: input }] },
        { content: ["Diagnostic received."] },
      ], { extension: "engine" });
      await host.prompt("Write api/route.py using the write tool.");
      const evidence = await host.snapshot();
      expect(await readFile(join(cwd, input.path), "utf8")).toBe(input.content);
      expect(evidence.notificationStarted).toBe(true);
      expect(evidence.notificationReleased).toBe(true);
      const firstPostToolCall = evidence.calls[1];
      expect(firstPostToolCall).toBeDefined();
      expect(evidence.calls).toHaveLength(2);
      expect(JSON.stringify(firstPostToolCall?.context.messages)).toContain(CONTRACT_CURSOR_EXECUTE_MESSAGE);
      // The OMP adapter links against the daemon of the host's Rerail home, not this process's.
      vi.stubEnv("RERAIL_HOME", join(cwd, "home"));
      const steer = linkSteers(
        steerBlock(CONTRACT_CURSOR_EXECUTE_RULE_ID, `${CONTRACT_CURSOR_EXECUTE_MESSAGE} (api/route.py:2:5)`),
        await dashboardOrigin(cwd),
      );
      vi.unstubAllEnvs();
      expect(evidence.branch).toEqual(expect.arrayContaining([
        expect.objectContaining({ type: "custom_message", content: expect.stringContaining(steer) }),
      ]));
      expect(host.frames.filter(frame => frame.type === "extension_error")).toEqual([]);
    } finally {
      try { await host?.close(); } finally { await rm(cwd, { recursive: true, force: true }); }
    }
  }, 90_000);
});
