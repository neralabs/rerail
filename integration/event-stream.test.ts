import { afterEach, describe, expect, test } from "vitest";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { z } from "zod";
import { RERAIL_EVENT_PROTOCOL_PREFACE } from "../src/event-protocol.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).flatMap(path => [
    rm(path, { recursive: true, force: true }),
    rm(`${path}.home`, { recursive: true, force: true }),
  ]));
});

async function runFrames(storageRoot: string, frames: readonly string[]): Promise<readonly Record<string, unknown>[]> {
  const { promise, resolve, reject } = Promise.withResolvers<readonly Record<string, unknown>[]>();
  const child = spawn(process.execPath, ["dist/event-stream.js"], {
    env: { ...process.env, RERAIL_STORAGE_ROOT: storageRoot, RERAIL_HOME: `${storageRoot}.home` },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const stdout: Buffer[] = [];
  const stderr: Buffer[] = [];
  child.stdout.on("data", (chunk) => stdout.push(Buffer.from(chunk)));
  child.stderr.on("data", (chunk) => stderr.push(Buffer.from(chunk)));
  child.once("error", reject);
  child.once("exit", (code) => {
    if (code !== 0) {
      reject(new Error(`event stream exited ${String(code)}: ${Buffer.concat(stderr).toString("utf8")}`));
      return;
    }
    const results = Buffer.concat(stdout)
      .toString("utf8")
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line) as Record<string, unknown>);
    resolve(results);
  });
  child.stdin.end(`${[RERAIL_EVENT_PROTOCOL_PREFACE, ...frames].join("\n")}\n`);
  return await promise;
}

function frame(event: Readonly<Record<string, unknown>>): string {
  return JSON.stringify({ event });
}

describe("Rerail persistent event stream", () => {
  test("ingests lifecycle events without MCP and restores idempotency after restart", async () => {
    const storageRoot = await mkdtemp(join(tmpdir(), "rerail-event-stream-"));
    temporaryDirectories.push(storageRoot);
    const session = { host: "claude-code", sessionId: "stream-session", cwd: storageRoot };

    const firstRun = await runFrames(storageRoot, [
      frame({ kind: "session-start", eventId: "start", session }),
      frame({
        kind: "user-prompt",
        eventId: "prompt",
        sessionId: session.sessionId,
        session,
        text: "Persistent content",
      }),
    ]);
    expect(firstRun[0]).toEqual({ eventId: "start", deliveries: [] });
    expect(firstRun[1]).toEqual(expect.objectContaining({ eventId: "prompt" }));
    z.array(z.object({ requested: z.literal("observe"), mechanism: z.literal("observe") }).passthrough())
      .parse(firstRun[1]?.deliveries);

    const replay = await runFrames(storageRoot, [
        frame({
          kind: "user-prompt",
          eventId: "prompt",
          sessionId: session.sessionId,
          session,
          text: "Duplicate replay",
        }),
        frame({ kind: "session-end", eventId: "end", sessionId: session.sessionId, session }),
    ]);
    expect(replay.map(response => response.eventId)).toEqual(["prompt", "end"]);
    z.array(z.object({ requested: z.literal("observe"), mechanism: z.literal("observe") }).passthrough())
      .parse(replay[0]?.deliveries);
    expect(replay[1]?.deliveries).toEqual([]);


    const sessionDirectories = (await readdir(storageRoot, { withFileTypes: true }))
      .filter((entry) => entry.isDirectory())
      .map((entry) => entry.name);
    expect(sessionDirectories).toHaveLength(1);
    const ledger = await readFile(join(storageRoot, sessionDirectories[0]!, "events.jsonl"), "utf8");
    const entries = ledger
      .trim()
      .split("\n")
      .map((line) =>
        z.object({ type: z.string(), eventId: z.string().optional() }).passthrough().parse(JSON.parse(line)),
      );
    expect(entries.filter((entry) => entry.type === "event").map((entry) => entry.eventId)).toEqual([
      "start",
      "prompt",
      "end",
    ]);
  }, 30_000);

  test("contains malformed frames without stopping the stream", async () => {
    const storageRoot = await mkdtemp(join(tmpdir(), "rerail-event-errors-"));
    temporaryDirectories.push(storageRoot);
    const valid = frame({
      kind: "session-start",
      eventId: "start",
      session: { host: "codex", sessionId: "s1", cwd: storageRoot },
    });
    expect(await runFrames(storageRoot, ["not-json", valid])).toEqual([
      expect.objectContaining({ eventId: null, error: expect.objectContaining({ code: "invalid-event" }) }),
      { eventId: "start", deliveries: [] },
    ]);
  }, 30_000);
});
