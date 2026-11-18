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

/** The steer for an edit of `path` whose lines hold a print the call wrote. */
const printSteer = (path: string): string => `print in ${path}`;

/** Whether the call wrote a print: its text holds one, and what the file held before it did not, or is unknown. */
function wrotePrint(edit: EditBatch["edits"][number]): boolean {
  if (edit.generatedText?.includes("print(") !== true) return false;
  return edit.previousReadError !== undefined || edit.previousText?.includes("print(") !== true;
}

/**
 * An engine service over a checkout of its own that checks the events hooks leave in its home (the checkout's folder),
 * with a Claude Code session started: its auditor keeps each edit batch, steers once for each edit of the session that
 * wrote a print (`printSteer`), and steers `LATE` once at the first turn end whose checks it settles. Its evaluator
 * opens once `opening` settles; `walking` settles once a walk of the session's folder asks about a file. Takes no event
 * until the test asks (`untaken.taken()`) or sends one.
 */
async function serviceWithUntaken(opening: Promise<void> = Promise.resolve()) {
  const root = await realpath(await mkdtemp(join(process.platform === "win32" ? tmpdir() : SOCKET_ROOT, "rerail-untaken-")));
  onTestFinished(async () => {
    await removeFolder(root);
  });
  writeFileSync(join(root, "a.py"), "def a():\n    return 1\n");
  const databasePath = join(root, "rerail.sqlite");
  startTestRepository(root, databasePath);
  const socketPath = testEngineEndpoint(root);
  const opened = Promise.withResolvers<void>();
  const walking = Promise.withResolvers<void>();
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
        await opening;
      },
      acceptsPath: () => {
        walking.resolve();
        return true;
      },
      auditGeneratedEdits: async (_sessionId, batch) => {
        batches.push(batch);
        const steering = batch.mode === "log" ? [] : batch.edits.filter(wrotePrint);
        for (const { path } of steering) await steerDelivery.requestSteer(SESSION, "steer", printSteer(path), { revision: batch.revision, scope: [path] });
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
  return { root, databasePath, service, send, leave, steered: steered.promise, walking: walking.promise, batches, passes };
}

/** The session's ledger, entry by entry. */
async function ledgerEntries(root: string): Promise<{ type?: string; content?: string; toolCallId?: string; preimages?: unknown }[]> {
  const ledger = await readFile(join(root, "state", createHash("sha256").update(SESSION).digest("hex"), "events.jsonl"), "utf8");
  return ledger.split("\n").filter(line => line !== "").map(line => JSON.parse(line) as { type?: string });
}

/** How many times the session's ledger records `content` as delivered to its agent. */
async function recordedDeliveries(root: string, content: string): Promise<number> {
  return (await ledgerEntries(root)).filter(entry => entry.type === "delivery" && JSON.stringify(entry).includes(content)).length;
}

/** The events hooks left for a Write call that ran at `ran`: the call a minute before, the result a minute after. */
async function leftWrite(held: Awaited<ReturnType<typeof serviceWithUntaken>>, toolCallId: string, path: string, content: string, ran: number): Promise<void> {
  const tool = { sessionId: SESSION, toolCallId, toolName: "Write" } as const;
  await held.leave({ kind: "call", eventId: `${toolCallId}:call`, ...tool, input: { file_path: path, content } }, ran - 60_000);
  await held.leave({ kind: "result", eventId: `${toolCallId}:result`, ...tool, output: "ok" }, ran + 60_000);
}

/**
 * Lets the session's evaluator open once the tool has written `files`, and waits for its first view of the folder (a
 * turn's end waits for it), which takes them as another writer's: they changed since the session started, while no
 * call of it ran, as its calls are still untaken.
 */
async function firstViewAfter(held: Awaited<ReturnType<typeof serviceWithUntaken>>, opening: () => void, files: Record<string, string>): Promise<void> {
  for (const [path, content] of Object.entries(files)) writeFileSync(join(held.root, path), content);
  opening();
  // Once the walk runs, the evaluator is open, so the turn's end waits for that walk rather than going on without it.
  await held.walking;
  await held.send({ kind: "turn-end", eventId: "view", sessionId: SESSION, text: "" });
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

  it("steers each Write left before the session first saw its folder with the next answer, not as an edit already seen", async () => {
    const opening = Promise.withResolvers<void>();
    const held = await serviceWithUntaken(opening.promise);
    // The session started, and its evaluator still opens; the hooks left two Write calls, and the tool wrote both files.
    const ran = Date.now();
    const files = { "w1.py": "def w1():\n    print(1)\n", "w2.py": "def w2():\n    print(1)\n" };
    await firstViewAfter(held, opening.resolve, files);
    for (const [name, content] of Object.entries(files)) await leftWrite(held, name, name, content, ran);
    await held.service.untaken.taken();
    // Read against that view, each edit was no change, and no steer for it ever reached the agent.
    expect(await held.send(prompt("next"))).toEqual(expect.arrayContaining([printSteer("w1.py"), printSteer("w2.py")]));
    // Nothing of what the files held is taken as the calls' "before": their tool had already written them.
    const calls = (await ledgerEntries(held.root)).filter(entry => entry.type === "policy-call");
    expect(calls.map(({ toolCallId, preimages }) => ({ toolCallId, preimages }))).toEqual([
      { toolCallId: "w1.py", preimages: {} }, { toolCallId: "w2.py", preimages: {} },
    ]);
  });

  it("reads a second Write left for a file whole, as the first, since the session's read of the file came after both", async () => {
    const opening = Promise.withResolvers<void>();
    const held = await serviceWithUntaken(opening.promise);
    const ran = Date.now();
    await firstViewAfter(held, opening.resolve, { "w3.py": "def w3():\n    print(1)\n    print(2)\n" });
    await leftWrite(held, "first", "w3.py", "def w3():\n    print(1)\n", ran);
    await leftWrite(held, "second", "w3.py", "def w3():\n    print(1)\n    print(2)\n", ran + 1);
    await held.service.untaken.taken();
    // The first call's read of the file holds what the second wrote: taken as the second's "before", it was no change,
    // and the first call's steer, overtaken by the second's check of the file, never reached the agent.
    expect(await held.send(prompt("next"))).toContain(printSteer("w3.py"));
    const edits = held.batches.filter(batch => batch.mode !== "log").flatMap(batch => batch.edits).filter(edit => edit.path === "w3.py");
    expect(edits.map(edit => edit.previousReadError === undefined ? edit.previousText : "unknown")).toEqual(["unknown", "unknown"]);
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
