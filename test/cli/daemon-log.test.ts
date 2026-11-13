import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { stampLines } from "../../src/cli/daemon-log.js";
import { PACKAGE_ROOT } from "../../src/engine/index.js";

const TIME = String.raw`\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z`;

type Writer = { write: (chunk: unknown, ...rest: unknown[]) => boolean };

/**
 * Two streams writing to one file, as the daemon's stdout and stderr do, which keeps each write with its encoding, and
 * the stamp clock's time, which a test moves on.
 */
function capturedStreams(): { out: Writer; err: Writer; writes: Array<{ chunk: unknown; encoding: unknown }>; clock: { now: number } } {
  const writes: Array<{ chunk: unknown; encoding: unknown }> = [];
  const clock = { now: Date.UTC(2026, 10, 11, 15, 33, 2, 123) };
  const writer = (): Writer => ({
    write: (chunk: unknown, encoding?: unknown) => {
      writes.push({ chunk, encoding });
      return true;
    },
  });
  const out = writer();
  const err = writer();
  stampLines([out, err], () => new Date(clock.now));
  return { out, err, writes, clock };
}

/** The text of `writes`: each string as it is, which Node writes in its encoding, and each run of bytes as UTF-8. */
function textOf(writes: ReadonlyArray<{ chunk: unknown }>): string {
  return writes.map(({ chunk }) => (typeof chunk === "string" ? chunk : Buffer.from(chunk as Uint8Array).toString("utf8"))).join("");
}

describe("daemon.log lines", () => {
  it("each start with the time they were written, however the writes split them", () => {
    const { out, writes, clock } = capturedStreams();
    out.write("listening\nready\n");
    clock.now += 1_000;
    out.write("half a ");
    clock.now += 1_000;
    out.write("line\nand the next\n");
    expect(textOf(writes)).toBe([
      "2026-11-11T15:33:02.123Z listening",
      "2026-11-11T15:33:02.123Z ready",
      "2026-11-11T15:33:03.123Z half a line",
      "2026-11-11T15:33:04.123Z and the next",
      "",
    ].join("\n"));
  });

  it("each start with one time when stdout and stderr each leave a line unfinished for the other", () => {
    const { out, err, writes, clock } = capturedStreams();
    out.write("half a ");
    clock.now += 1_000;
    err.write("warning\n");
    clock.now += 1_000;
    out.write("line\n");
    err.write("an error at ");
    out.write("its end\n");
    expect(textOf(writes)).toBe([
      "2026-11-11T15:33:02.123Z half a warning",
      "2026-11-11T15:33:04.123Z line",
      "2026-11-11T15:33:04.123Z an error at its end",
      "",
    ].join("\n"));
  });

  it("stamps bytes as bytes, a character split across writes included", () => {
    const { out, writes } = capturedStreams();
    const text = Buffer.from("café crashed\n");
    out.write(text.subarray(0, 4));
    out.write(text.subarray(4));
    expect(Buffer.concat(writes.map(({ chunk }) => chunk as Buffer)).toString("utf8")).toBe("2026-11-11T15:33:02.123Z café crashed\n");
  });

  it("stamps a string in any encoding Node writes, and bytes spelled out as the bytes they are", () => {
    const { out, err, writes } = capturedStreams();
    out.write("café", "latin1");
    err.write(" crashed\n", "utf16le");
    // "hi\n", spelled out in base64.
    out.write("aGkK", "base64");
    out.write("next\n");
    expect(writes.map(({ encoding }) => encoding)).toEqual(["latin1", "utf16le", "base64", undefined]);
    expect(textOf(writes)).toBe([
      "2026-11-11T15:33:02.123Z café crashed",
      "2026-11-11T15:33:02.123Z hi",
      "2026-11-11T15:33:02.123Z next",
      "",
    ].join("\n"));
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
