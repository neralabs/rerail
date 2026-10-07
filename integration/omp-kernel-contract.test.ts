import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { OmpHost } from "./omp-host.js";

describe("OMP native executable-policy contract", () => {
  it("delivers policy guidance before the next model call despite delayed execution notifications", async () => {
    const cwd = await mkdtemp(join(tmpdir(), "rerail-omp-kernel-"));
    let host: OmpHost | undefined;
    try {
      await mkdir(join(cwd, "api"));
      await writeFile(join(cwd, "api", "route.py"), "def route():\n    return 1\n");
      const input = { path: "api/route.py", content: "def route(cursor):\n    cursor.execute('SELECT 1')\n" };
      host = await OmpHost.start(cwd, [
        { content: [{ type: "toolCall", name: "write", arguments: input }] },
        { content: ["Diagnostic received."] },
      ], { extension: "kernel" });
      await host.prompt("Write api/route.py using the write tool.");
      const evidence = await host.snapshot();
      expect(await readFile(join(cwd, input.path), "utf8")).toBe(input.content);
      expect(evidence.notificationStarted).toBe(true);
      expect(evidence.notificationReleased).toBe(true);
      const firstPostToolCall = evidence.calls[1];
      expect(firstPostToolCall).toBeDefined();
      expect(evidence.calls).toHaveLength(2);
      expect(JSON.stringify(firstPostToolCall?.context.messages)).toContain("CURSOR_EXECUTE");
      expect(evidence.branch).toEqual(expect.arrayContaining([
        expect.objectContaining({ type: "custom_message", content: expect.stringContaining("CURSOR_EXECUTE") }),
      ]));
      expect(host.frames.filter(frame => frame.type === "extension_error")).toEqual([]);
    } finally {
      try { await host?.close(); } finally { await rm(cwd, { recursive: true, force: true }); }
    }
  }, 90_000);
});
