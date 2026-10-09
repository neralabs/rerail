import { afterAll, afterEach, beforeAll, describe, expect, test } from "vitest";
import { spawn } from "node:child_process";
import { mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { startEngineService, type EngineService } from "../src/engine-service.js";
import { startTestRepository } from "../test/support/started-repository.js";
import { sessionStartNote } from "../src/rerail-steer.js";
import { dashboardOrigin, presentedSessionNoteRules } from "../src/session-presentation.js";

const temporaryDirectories: string[] = [];
const engineServices: EngineService[] = [];
const originalRerailHome = process.env.RERAIL_HOME;
let isolatedRerailHome: string;

beforeAll(async () => {
  isolatedRerailHome = await mkdtemp(join(tmpdir(), "rerail-cursor-home-"));
  process.env.RERAIL_HOME = isolatedRerailHome;
});

afterAll(async () => {
  if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = originalRerailHome;
  await rm(isolatedRerailHome, { recursive: true, force: true });
});

afterEach(async () => {
  await Promise.all(engineServices.splice(0).map(engineService => engineService.close()));
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function invokeCursorHook(executable: string, input: Readonly<Record<string, unknown>>, kernelSocketPath: string): Promise<Record<string, unknown>> {
  const { promise, resolve, reject } = Promise.withResolvers<Record<string, unknown>>();
  const child = spawn(process.execPath, [executable], { stdio: ["pipe", "pipe", "pipe"], env: { ...process.env, RERAIL_ENGINE_SOCKET: kernelSocketPath } });
  const output: Buffer[] = [];
  const errors: Buffer[] = [];
  child.stdout.on("data", chunk => output.push(Buffer.from(chunk)));
  child.stderr.on("data", chunk => errors.push(Buffer.from(chunk)));
  child.once("error", reject);
  child.once("exit", code => {
    if (code !== 0) {
      reject(new Error(`Cursor hook exited ${String(code)}: ${Buffer.concat(errors).toString("utf8")}`));
      return;
    }
    resolve(JSON.parse(Buffer.concat(output).toString("utf8")) as Record<string, unknown>);
  });
  child.stdin.end(JSON.stringify(input));
  return await promise;
}

describe("Cursor packaged event adapter", () => {
  test("persists normalized hook events without an MCP server", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-cursor-events-"));
    temporaryDirectories.push(root);
    startTestRepository(root);
    const executable = join(process.cwd(), "dist", "cursor-hook.js");
    const common = { conversation_id: "cursor-session", generation_id: "generation-1", cwd: root };
    const kernelSocketPath = join(root, "engine.sock");
    engineServices.push(await startEngineService({ socketPath: kernelSocketPath, storageRoot: join(root, ".rerail/sessions"), modelPath: join(process.cwd(), "model") }));

    expect(await invokeCursorHook(executable, { ...common, hook_event_name: "sessionStart" }, kernelSocketPath)).toEqual({
      additional_context: sessionStartNote(await dashboardOrigin(root), "cursor", common.conversation_id, await presentedSessionNoteRules({ host: "cursor", sessionId: common.conversation_id, cwd: root })),
    });
    expect(await invokeCursorHook(executable, {
      ...common,
      hook_event_name: "beforeSubmitPrompt",
      prompt: "Keep the session active",
    }, kernelSocketPath)).toEqual({ continue: true });
    expect(await invokeCursorHook(executable, { ...common, hook_event_name: "sessionEnd" }, kernelSocketPath)).toEqual({});

    const sessionRoot = join(root, ".rerail", "sessions");
    const [sessionDirectory] = await readdir(sessionRoot);
    const ledger = await readFile(join(sessionRoot, sessionDirectory!, "events.jsonl"), "utf8");
    const eventIds = ledger.trim().split("\n").map(line => JSON.parse(line) as { type: string; eventId?: string })
      .filter(entry => entry.type === "event")
      .map(entry => entry.eventId);
    expect(eventIds).toEqual([
      "cursor-session:session-start:generation-1",
      "cursor-session:prompt:generation-1",
      "cursor-session:session-end:generation-1",
    ]);
  }, 30_000);

  test("does not contact the engine service when this repository is not started", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-cursor-not-started-"));
    temporaryDirectories.push(root);
    const executable = join(process.cwd(), "dist", "cursor-hook.js");

    expect(await invokeCursorHook(
      executable,
      { conversation_id: "cursor-session", generation_id: "generation-1", cwd: root, hook_event_name: "sessionStart" },
      join(root, "missing-engine.sock"),
    )).toEqual({});
    await expect(readdir(join(root, ".rerail", "sessions"))).rejects.toMatchObject({ code: "ENOENT" });
  });
});
