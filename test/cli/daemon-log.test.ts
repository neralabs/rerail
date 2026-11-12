import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { stampLines } from "../../src/cli/daemon-log.js";
import { PACKAGE_ROOT } from "../../src/engine/index.js";

const TIME = String.raw`\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z`;

/** A stream that keeps each chunk written to it, and the stamp clock's time, which a test moves on. */
function capturedStream(): { stream: { write: (chunk: unknown, ...rest: unknown[]) => boolean }; chunks: unknown[]; clock: { now: number } } {
  const chunks: unknown[] = [];
  const clock = { now: Date.UTC(2026, 10, 11, 15, 33, 2, 123) };
  const stream = {
    write: (chunk: unknown) => {
      chunks.push(chunk);
      return true;
    },
  };
  stampLines(stream, () => new Date(clock.now));
  return { stream, chunks, clock };
}

describe("daemon.log lines", () => {
  it("each start with the time they were written, however the writes split them", () => {
    const { stream, chunks, clock } = capturedStream();
    stream.write("listening\nready\n");
    clock.now += 1_000;
    stream.write("half a ");
    clock.now += 1_000;
    stream.write("line\nand the next\n");
    expect(chunks.join("")).toBe([
      "2026-11-11T15:33:02.123Z listening",
      "2026-11-11T15:33:02.123Z ready",
      "2026-11-11T15:33:03.123Z half a line",
      "2026-11-11T15:33:04.123Z and the next",
      "",
    ].join("\n"));
  });

  it("stamps bytes as bytes, a character split across writes included, and leaves a write in another encoding alone", () => {
    const { stream, chunks } = capturedStream();
    const text = Buffer.from("café crashed\n");
    stream.write(text.subarray(0, 4));
    stream.write(text.subarray(4));
    stream.write("aGk=", "base64");
    expect(Buffer.concat(chunks.slice(0, 2) as Buffer[]).toString("utf8")).toBe("2026-11-11T15:33:02.123Z café crashed\n");
    expect(chunks[2]).toBe("aGk=");
  });

  it("are written by the daemon's entry with their time, after the secrets in them are taken out", { timeout: 120_000 }, () => {
    const token = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    const result = spawnSync(process.execPath, ["--import", "tsx", join(PACKAGE_ROOT, "src", "cli", "daemon-entry.ts"), token], {
      cwd: PACKAGE_ROOT,
      encoding: "utf8",
      timeout: 110_000,
    });
    expect(result.status).toBe(1);
    expect(result.stderr).toMatch(new RegExp(`^${TIME} rerail daemon: Unknown daemon argument: \\[redacted:`, "u"));
    expect(result.stderr).not.toContain(token);
  });
});
