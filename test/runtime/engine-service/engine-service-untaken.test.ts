import { createHash } from "node:crypto";
import { writeFileSync } from "node:fs";
import { mkdtemp, readFile, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, onTestFinished } from "vitest";
import { type AdapterEvent, type CodeRuleAuditor, leaveUntaken, type UntakenPass } from "../../../src/engine/index.js";
import { engineRequestBody, ingestEngineEvent, startEngineService } from "../../../src/runtime/engine-service.js";
import { SqliteSessionReader } from "../../../src/runtime/index.js";
import { testEngineEndpoint } from "../../support/platform/engine-endpoint.js";
import { testEmbedder } from "../../support/hook-events.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { startTestRepository } from "../../support/started-repository.js";

type EditBatch = Parameters<CodeRuleAuditor["auditGeneratedEdits"]>[1];

/** Off Windows the checkout sits here: a Unix socket path must fit 104 bytes on macOS and 108 on Linux. */
const SOCKET_ROOT = "/tmp";

/** The steer a turn's checks find, once. */
const LATE = "late steer";
const SESSION = "left-untaken";

/**
 * An engine service over a checkout of its own that checks the events hooks leave in its home (the checkout's folder),
 * with a Claude Code session started: its auditor keeps each edit batch, and steers `LATE` once at the first turn end
 * whose checks it settles. Takes no event until the test asks (`untaken.taken()`) or sends one.
 */
async function serviceWithUntaken() {
  const root = await realpath(await mkdtemp(join(process.platform === "win32" ? tmpdir() : SOCKET_ROOT, "rerail-untaken-")));
  onTestFinished(async () => {
    await removeFolder(root);
  });
  writeFileSync(join(root, "a.py"), "def a():\n    return 1\n");
  const databasePath = join(root, "rerail.sqlite");
  startTestRepository(root, databasePath);
  const socketPath = testEngineEndpoint(root);
  const opened = Promise.withResolvers<void>();
  const steered = Promise.withResolvers<void>();
  const batches: EditBatch[] = [];
  const passes: UntakenPass[] = [];
  let steers = 0;
  const service = await startEngineService({
    socketPath,
    storageRoot: join(root, "state"),
    databasePath,
    embedder: testEmbedder,
    untakenEvents: { home: root, lateChecked: pass => passes.push(pass) },
    createAuditor: ({ steerDelivery }) => ({
      openWorkspace: async () => {
        opened.resolve();
      },
      acceptsPath: () => true,
      auditGeneratedEdits: async (_sessionId, batch) => {
        batches.push(batch);
        return { revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] };
      },
      settleGeneratedEdits: async () => {
        if (steers++ > 0) return;
        await steerDelivery.requestSteer(SESSION, "steer", LATE, { revision: 0, scope: [] });
        steered.resolve();
      },
      close: async () => undefined,
    }),
  });
  onTestFinished(async () => {
    await service.close();
  });
  const session = { host: "claude-code" as const, sessionId: SESSION, cwd: root };
  await service.ingest({ kind: "session-start", eventId: "start", session });
  await opened.promise;
  await new Promise<void>(done => setImmediate(done));
  const engine = { socketPath };
  const send = async (event: AdapterEvent) => (await ingestEngineEvent(engine, event)).map(delivery => delivery.content);
  const leave = async (event: AdapterEvent, arrivedAt: number) => await leaveUntaken(root, process.pid, engineRequestBody(event), arrivedAt);
  return { root, databasePath, service, send, leave, steered: steered.promise, batches, passes };
}

/** How many times the session's ledger records `content` as delivered to its agent. */
async function recordedDeliveries(root: string, content: string): Promise<number> {
  const ledger = await readFile(join(root, "state", createHash("sha256").update(SESSION).digest("hex"), "events.jsonl"), "utf8");
  return ledger.split("\n").filter(line => line.includes("\"type\":\"delivery\"") && line.includes(content)).length;
}

function recordedEvents(databasePath: string): { eventId: string; observedAt: number }[] {
  const reader = new SqliteSessionReader(databasePath);
  try {
    return reader.events(SESSION).filter(event => event.kind === "user-prompt").map(({ eventId, observedAt }) => ({ eventId, observedAt }));
  } finally {
    reader.close();
  }
}

const prompt = (eventId: string): AdapterEvent => ({ kind: "user-prompt", eventId, sessionId: SESSION, text: eventId });

// On every system, the Windows pipe included.
describe("an engine service beside the events hooks left while its daemon did not read", () => {
  it("checks each one, at the time its hook met it, before it routes an event sent on its socket or pipe", async () => {
    const held = await serviceWithUntaken();
    await held.leave(prompt("left"), 1_000);
    // No watch runs and nothing asked for a pass: only the step before the event sent takes what was left.
    expect(await held.send(prompt("sent"))).toEqual([]);
    const recorded = recordedEvents(held.databasePath);
    expect(recorded.map(event => event.eventId)).toEqual(["left", "sent"]);
    expect(recorded[0]!.observedAt).toBe(1_000);
  });

  it("checks a turn end left untaken as one sent, and the steer its checks find goes with the session's next answer, once", async () => {
    const held = await serviceWithUntaken();
    await held.leave({ kind: "turn-end", eventId: "stop", sessionId: SESSION, text: "" }, 1_000);
    await held.service.untaken.taken();
    await held.steered;
    // Checked as one sent but answered to no one, it took the steer into an answer nobody read, and the next got none.
    expect(await held.send(prompt("next"))).toEqual([LATE]);
    expect(await held.send(prompt("after"))).toEqual([]);
    expect(await recordedDeliveries(held.root, LATE)).toBe(1);
    expect(held.passes).toEqual([{ events: 1, from: 1_000, to: 1_000, failed: 0 }]);
  });

  it("checks an edit left untaken against what the session last saw of its file, not against the edit already made", async () => {
    const held = await serviceWithUntaken();
    const tool = { sessionId: SESSION, toolCallId: "edit-1", toolName: "Edit" } as const;
    // The hook left the call and gave the agent its answer; the tool ran; then the hook left the result.
    const ran = Date.now();
    await held.leave({ kind: "call", eventId: "edit-1:call", ...tool, input: { file_path: "a.py", old_string: "1", new_string: "2" } }, ran - 60_000);
    writeFileSync(join(held.root, "a.py"), "def a():\n    return 2\n");
    await held.leave({ kind: "result", eventId: "edit-1:result", ...tool, output: "ok" }, ran + 60_000);
    await held.send(prompt("next"));
    const edits = held.batches.flatMap(batch => batch.edits).filter(edit => edit.path === "a.py");
    // A call begun late read its file's "before" from the file as already edited, and found no edit to check. A walk
    // of the session's folder may read the edit once more (seen on Windows); each read is of the edit as made.
    expect(edits.length).toBeGreaterThan(0);
    expect(new Set(edits.map(edit => edit.generatedText))).toEqual(new Set(["def a():\n    return 2\n"]));
  });

  it("takes what a shell command left untaken wrote as the session's own, from when its hooks met it", async () => {
    const held = await serviceWithUntaken();
    const tool = { sessionId: SESSION, toolCallId: "shell-1", toolName: "Bash" } as const;
    // The hooks met the call and its result around the command's run, a minute apart; the check comes after both.
    const ran = Date.now();
    await held.leave({ kind: "call", eventId: "shell-1:call", ...tool, input: { command: "python generate.py" } }, ran - 60_000);
    writeFileSync(join(held.root, "b.py"), "def b():\n    return 3\n");
    await held.leave({ kind: "result", eventId: "shell-1:result", ...tool, output: "ok" }, ran + 60_000);
    await held.send(prompt("next"));
    // Checked by the clock as it was checked, the call ran after the file was written, which read as another writer's.
    const edits = held.batches.flatMap(batch => batch.edits).filter(edit => edit.path === "b.py");
    expect(edits.map(edit => edit.generatedText)).toEqual(["def b():\n    return 3\n"]);
  });
});
