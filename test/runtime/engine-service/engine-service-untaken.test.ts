import { createHash } from "node:crypto";
import { writeFileSync } from "node:fs";
import { mkdtemp, readdir, readFile, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it, onTestFailed, onTestFinished, vi } from "vitest";
import { type AdapterEvent, type CodeRuleAuditor, leaveUntaken, UNTAKEN_EVENTS_DIR, type UntakenPass } from "../../../src/engine/index.js";
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

/** The steer for a line of an edit of `path` that holds a print the call wrote. */
const printSteer = (path: string, line: number): string => `print in ${path}:${line}`;

/**
 * The lines (from 1) holding a print the call wrote: each line of its text with one that what the file held before did
 * not hold; every such line when that is unknown, as the rules then read the whole text the call left.
 */
function printedLines(edit: EditBatch["edits"][number]): number[] {
  const before = new Set(edit.previousReadError === undefined ? edit.previousText?.split("\n") : []);
  const lines = edit.generatedText?.split("\n") ?? [];
  return lines.flatMap((line, index) => (line.includes("print(") && !before.has(line) ? [index + 1] : []));
}

/**
 * An engine service over a checkout of its own that checks the events hooks leave in its home (the checkout's folder),
 * with a Claude Code session started: its auditor keeps each edit batch, steers once for each line holding a print an
 * edit of the session wrote (`printSteer`), and steers `LATE` once at the first turn end whose checks it settles. Its
 * evaluator opens once `opening` settles; `walking` settles once a walk of the session's folder asks about a file.
 * Takes no event until the test asks (`untaken.taken()`) or sends one.
 */
async function serviceWithUntaken(opening: Promise<void> = Promise.resolve()) {
  const root = await realpath(await mkdtemp(join(process.platform === "win32" ? tmpdir() : SOCKET_ROOT, "rerail-untaken-")));
  onTestFinished(async () => {
    await removeFolder(root);
  });
  writeFileSync(join(root, "a.py"), "def a():\n    return 1\n");
  writeFileSync(join(root, ".gitignore"), "*.log\n");
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
        const steering = batch.mode === "log" ? [] : batch.edits.flatMap(edit => printedLines(edit).map(line => ({ path: edit.path, line })));
        for (const { path, line } of steering) await steerDelivery.requestSteer(SESSION, "steer", printSteer(path, line), { revision: batch.revision, scope: [path] });
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

type HeldService = Awaited<ReturnType<typeof serviceWithUntaken>>;

/** The events hooks left for a call of `toolName` that ran at `ran`: the call a minute before, the result a minute after. */
async function leftCall(held: HeldService, toolCallId: string, toolName: "Write" | "Edit", input: unknown, ran: number): Promise<void> {
  const tool = { sessionId: SESSION, toolCallId, toolName } as const;
  await held.leave({ kind: "call", eventId: `${toolCallId}:call`, ...tool, input }, ran - 60_000);
  await held.leave({ kind: "result", eventId: `${toolCallId}:result`, ...tool, output: "ok" }, ran + 60_000);
}

/** An Edit of a.py the session's hooks met live: its call, the tool leaving `written`, then its result. */
async function liveEdit(held: HeldService, oldString: string, newString: string, written: string): Promise<void> {
  const edit = { sessionId: SESSION, toolCallId: "live", toolName: "Edit" } as const;
  await held.send({ kind: "call", eventId: "live:call", ...edit, input: { file_path: "a.py", old_string: oldString, new_string: newString } });
  writeFileSync(join(held.root, "a.py"), written);
  await held.send({ kind: "result", eventId: "live:result", ...edit, output: "ok" });
}

/** The events hooks left for a Write call that ran at `ran`. */
async function leftWrite(held: HeldService, toolCallId: string, path: string, content: string, ran: number): Promise<void> {
  await leftCall(held, toolCallId, "Write", { file_path: path, content }, ran);
}

/**
 * Waits for `settles`; a test that fails meanwhile, as by its time limit on a loaded machine, says it was still waiting
 * for `what`, so a slow step is told from a wrong result.
 */
async function waitingFor<T>(what: string, settles: Promise<T>): Promise<T> {
  let waiting = true;
  onTestFailed(() => {
    if (waiting) throw new Error(`still waiting for ${what}`);
  });
  try {
    return await settles;
  } finally {
    waiting = false;
  }
}

/** Waits for the session's first view of its folder: its walk asks about a file, and a turn's end waits for that walk. */
async function firstView(held: HeldService): Promise<void> {
  await waitingFor("the session's first walk to ask about a file", held.walking);
  await held.send({ kind: "turn-end", eventId: "view", sessionId: SESSION, text: "" });
}

/** The edits of `path` the session's checks read with steers (not as another writer's, logged only). */
function checkedEdits(held: HeldService, path: string): EditBatch["edits"] {
  return held.batches.filter(batch => batch.mode !== "log").flatMap(batch => batch.edits).filter(edit => edit.path === path);
}

/**
 * Lets the session's evaluator open once the tool has written `files`, and waits for its first view of the folder (a
 * turn's end waits for it), which takes them as another writer's: they changed since the session started, while no
 * call of it ran, as its calls are still untaken.
 */
async function firstViewAfter(held: HeldService, opening: () => void, files: Record<string, string>): Promise<void> {
  for (const [path, content] of Object.entries(files)) writeFileSync(join(held.root, path), content);
  opening();
  // Once the walk runs, the evaluator is open, so the turn's end waits for that walk rather than going on without it.
  await firstView(held);
}

function recordedEvents(databasePath: string): { eventId: string; observedAt: number }[] {
  const reader = new SqliteSessionReader(databasePath);
  try {
    return reader.events(SESSION).filter(event => event.kind === "user-prompt").map(({ eventId, observedAt }) => ({ eventId, observedAt }));
  } finally {
    reader.close();
  }
}

/** The payloads of the session's recorded events of `kind`, in order. */
function sessionFacts(databasePath: string, kind: string): unknown[] {
  const reader = new SqliteSessionReader(databasePath);
  try {
    return reader.events(SESSION).filter(event => event.kind === kind).map(event => event.payload);
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

  it("checks each one, at the time its hook met it, before an event given to the service in its own process", async () => {
    const held = await serviceWithUntaken();
    await held.leave(prompt("left"), 1_000);
    // No watch runs and nothing asked for a pass: only the step before the event given takes what was left.
    expect(await held.service.ingest(prompt("given"))).toEqual([]);
    const recorded = recordedEvents(held.databasePath);
    expect(recorded.map(event => event.eventId)).toEqual(["left", "given"]);
    expect(recorded[0]!.observedAt).toBe(1_000);
  });

  it("takes each one at the minute's sweep though no notice of the folder told of it, so hooks stop skipping the daemon", async () => {
    const held = await serviceWithUntaken();
    await held.leave(prompt("left"), 1_000);
    const left = async (): Promise<string[]> => (await readdir(join(held.root, UNTAKEN_EVENTS_DIR))).filter(name => name.endsWith(".json"));
    expect(await left()).toHaveLength(1);
    // No watch runs, as when the notice of the file was lost, and nothing else asked for a pass.
    await held.service.closeIdleSessions();
    expect(await left()).toEqual([]);
    await held.send(prompt("sent"));
    expect(recordedEvents(held.databasePath).map(event => event.eventId)).toEqual(["left", "sent"]);
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
    expect(await held.send(prompt("next"))).toEqual(expect.arrayContaining([printSteer("w1.py", 2), printSteer("w2.py", 2)]));
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
    expect(await held.send(prompt("next"))).toContain(printSteer("w3.py", 2));
    const edits = checkedEdits(held, "w3.py");
    expect(edits.map(edit => edit.previousReadError === undefined ? edit.previousText : "unknown")).toEqual(["unknown", "unknown"]);
  });

  it("reads an Edit left untaken of a file the session's own call edited against that call's text, so it tells only its own lines", async () => {
    const held = await serviceWithUntaken();
    await firstView(held);
    const live = "def a():\n    print(1)\n    return 1\n";
    const edit = { sessionId: SESSION, toolCallId: "live", toolName: "Edit" } as const;
    await held.send({ kind: "call", eventId: "live:call", ...edit, input: { file_path: "a.py", old_string: "    return 1", new_string: "    print(1)\n    return 1" } });
    writeFileSync(join(held.root, "a.py"), live);
    await held.send({ kind: "result", eventId: "live:result", ...edit, output: "ok" });
    // The next Edit's hooks left it while the daemon did not read, and the tool wrote the file.
    const left = "def a():\n    print(1)\n    print(2)\n    return 1\n";
    writeFileSync(join(held.root, "a.py"), left);
    await leftCall(held, "left", "Edit", { file_path: "a.py", old_string: "    return 1", new_string: "    print(2)\n    return 1" }, Date.now());
    await held.service.untaken.taken();
    // Its "before" is the text the session's own call left, so line 2, which that call wrote, is not told again.
    const told = await held.send(prompt("next"));
    expect(told.filter(steer => steer.startsWith("print in a.py"))).toEqual([printSteer("a.py", 3)]);
    expect(checkedEdits(held, "a.py").filter(read => read.generatedText === left).map(read => read.previousText)).toEqual([live]);
  });

  it("reads an Edit left untaken of a file the session's walk saw changed against what the walk saw", async () => {
    const held = await serviceWithUntaken();
    await firstView(held);
    // A person changes the file, and the walk at the turn's end keeps what it saw: another writer's text.
    const seen = "def a():\n    print(1)\n    return 2\n";
    writeFileSync(join(held.root, "a.py"), seen);
    await held.send({ kind: "turn-end", eventId: "seen", sessionId: SESSION, text: "" });
    writeFileSync(join(held.root, "a.py"), "def a():\n    print(1)\n    print(2)\n    return 2\n");
    await leftCall(held, "left", "Edit", { file_path: "a.py", old_string: "    return 2", new_string: "    print(2)\n    return 2" }, Date.now());
    await held.service.untaken.taken();
    expect((await held.send(prompt("next"))).filter(steer => steer.startsWith("print in a.py"))).toEqual([printSteer("a.py", 3)]);
    expect(checkedEdits(held, "a.py").map(read => read.previousText)).toEqual([seen]);
  });

  it("reads an Edit left untaken whole where another writer removed a line since the session last saw the file, so the line it wrote back is told", async () => {
    const held = await serviceWithUntaken();
    await firstView(held);
    writeFileSync(join(held.root, "a.py"), "def a():\n    print(9)\n    return 1\n");
    await liveEdit(held, "    return 1", "    print(100)\n    return 1", "def a():\n    print(9)\n    print(100)\n    return 1\n");
    // Another writer removes line 2 while no walk runs; the next Edit, left while the daemon did not read, writes it back.
    writeFileSync(join(held.root, "a.py"), "def a():\n    print(100)\n    return 1\n");
    const left = "def a():\n    print(9)\n    print(100)\n    print(303)\n    return 1\n";
    writeFileSync(join(held.root, "a.py"), left);
    await leftCall(held, "left", "Edit", { file_path: "a.py", old_string: "    print(100)\n    return 1", new_string: "    print(9)\n    print(100)\n    print(303)\n    return 1" }, Date.now());
    await held.service.untaken.taken();
    // Undone, its edits give a text the session never saw: whether another write came between is unknown, so it is
    // read whole, its two lines among the rest.
    const told = (await held.send(prompt("next"))).filter(steer => steer.startsWith("print in a.py"));
    expect(told).toEqual(expect.arrayContaining([printSteer("a.py", 2), printSteer("a.py", 4)]));
    expect(checkedEdits(held, "a.py").filter(read => read.generatedText === left).map(read => read.previousText)).toEqual([undefined]);
  });

  it("reads an Edit left untaken whole where the file no longer holds its new text", async () => {
    const held = await serviceWithUntaken();
    await firstView(held);
    const live = "def a():\n    print(1)\n    return 1\n";
    await liveEdit(held, "    return 1", "    print(1)\n    return 1", live);
    // The left Edit wrote print(2), and another writer turned it into print(3) before the daemon read the call.
    const now = "def a():\n    print(1)\n    print(3)\n    return 1\n";
    writeFileSync(join(held.root, "a.py"), now);
    await leftCall(held, "left", "Edit", { file_path: "a.py", old_string: "    return 1", new_string: "    print(2)\n    return 1" }, Date.now());
    await held.service.untaken.taken();
    expect((await held.send(prompt("next"))).filter(steer => steer.startsWith("print in a.py"))).toEqual([printSteer("a.py", 2), printSteer("a.py", 3)]);
    expect(checkedEdits(held, "a.py").filter(read => read.generatedText === now).map(read => read.previousText)).toEqual([undefined]);
  });

  it("tells each of two Edits left untaken that write the same text into one file its line, reading both whole", async () => {
    const held = await serviceWithUntaken();
    await firstView(held);
    const edit = { file_path: "a.py", old_string: "    return 1", new_string: "    print(460)\n    return 1" };
    // The first call ran minutes before the second, each between its own two hooks.
    writeFileSync(join(held.root, "a.py"), "def a():\n    print(460)\n    return 1\n");
    await leftCall(held, "first", "Edit", edit, Date.now() - 200_000);
    const left = "def a():\n    print(460)\n    print(460)\n    return 1\n";
    writeFileSync(join(held.root, "a.py"), left);
    await leftCall(held, "second", "Edit", edit, Date.now());
    await held.service.untaken.taken();
    // The text left holds the second call's edit besides the first's: undone, the first one's gives a text the session
    // never saw, and its line 2 is told with the rest.
    const told = (await held.send(prompt("next"))).filter(steer => steer.startsWith("print in a.py"));
    expect(told).toEqual(expect.arrayContaining([printSteer("a.py", 2), printSteer("a.py", 3)]));
    const reads = checkedEdits(held, "a.py").filter(read => read.generatedText === left);
    expect(reads.length).toBeGreaterThanOrEqual(2);
    expect(new Set(reads.map(read => read.previousText))).toEqual(new Set([undefined]));
  });

  it("reads an Edit of every place left untaken whole where its new text was in the file before it", async () => {
    const held = await serviceWithUntaken();
    writeFileSync(join(held.root, "a.py"), "def a():\n    pass\n    y = 2\n    pass\n    return 1\n");
    await firstView(held);
    await liveEdit(held, "    y = 2", "    y = 2\n    print(0)", "def a():\n    pass\n    y = 2\n    print(0)\n    pass\n    return 1\n");
    const left = "def a():\n    print(0)\n    y = 2\n    print(0)\n    print(0)\n    return 1\n";
    writeFileSync(join(held.root, "a.py"), left);
    await leftCall(held, "left", "Edit", { file_path: "a.py", old_string: "    pass", new_string: "    print(0)", replace_all: true }, Date.now());
    await held.service.untaken.taken();
    // Undone at every place, its edit also puts back the line the live call wrote: not what the session saw.
    expect((await held.send(prompt("next"))).filter(steer => steer.startsWith("print in a.py"))).toEqual(expect.arrayContaining([printSteer("a.py", 2), printSteer("a.py", 5)]));
    expect(checkedEdits(held, "a.py").filter(read => read.generatedText === left).map(read => read.previousText)).toEqual([undefined]);
  });

  it("reads a Write left untaken whole where another writer changed its file since the session last saw it", async () => {
    const held = await serviceWithUntaken();
    await firstView(held);
    await liveEdit(held, "    return 1", "    print(9)\n    return 1", "def a():\n    print(9)\n    return 1\n");
    // Another writer removes line 2; the Write, left while the daemon did not read, writes it back with a line of its own.
    writeFileSync(join(held.root, "a.py"), "def a():\n    return 1\n");
    const left = "def a():\n    print(9)\n    print(10)\n    return 1\n";
    writeFileSync(join(held.root, "a.py"), left);
    await leftWrite(held, "left", "a.py", left, Date.now());
    await held.service.untaken.taken();
    // Read against what the session last saw, line 2, which the Write wrote, was no change of it.
    expect((await held.send(prompt("next"))).filter(steer => steer.startsWith("print in a.py"))).toEqual([printSteer("a.py", 2), printSteer("a.py", 3)]);
    expect(checkedEdits(held, "a.py").filter(read => read.generatedText === left).map(read => read.previousText)).toEqual([undefined]);
  });

  it("reads an Edit seen live against the text it captured as it began, whatever its input asked for", async () => {
    const held = await serviceWithUntaken();
    await firstView(held);
    const edit = { sessionId: SESSION, toolCallId: "live", toolName: "Edit" } as const;
    await held.send({ kind: "call", eventId: "live:call", ...edit, input: { file_path: "a.py", old_string: "    return 1", new_string: "    print(1)\n    return 1" } });
    // The tool writes its edit, and another writer adds a line before the call's result.
    const after = "def a():\n    print(1)\n    return 1\n\nprint(7)\n";
    writeFileSync(join(held.root, "a.py"), after);
    await held.send({ kind: "result", eventId: "live:result", ...edit, output: "ok" });
    expect(checkedEdits(held, "a.py").filter(read => read.generatedText === after).map(read => read.previousText)).toEqual(["def a():\n    return 1\n"]);
  });

  it("records a .gitignore a Write left untaken made ignore more as the tool's change, compared with what it held before", async () => {
    const held = await serviceWithUntaken();
    await firstView(held);
    writeFileSync(join(held.root, ".gitignore"), "*.log\nout/\n");
    await leftWrite(held, "ignore", ".gitignore", "*.log\nout/\n", Date.now());
    await held.service.untaken.taken();
    await held.send(prompt("next"));
    // Its baseline is the patterns Rerail read before the tool wrote the file, so the pattern the call added is recorded.
    expect(sessionFacts(held.databasePath, "ignore-rules-changed")).toEqual([{ path: ".gitignore", via: "tool", ignoresMore: ["out/"] }]);
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

  it("starts a session whose start was left untaken at the time its hook met it, before the events it sent after", async () => {
    const held = await serviceWithUntaken();
    const late = { host: "claude-code" as const, sessionId: "late-start", cwd: held.root };
    await held.leave({ kind: "session-start", eventId: "late:start", session: late }, 1_000);
    await held.leave({ kind: "user-prompt", eventId: "late:prompt", sessionId: late.sessionId, session: late, text: "hi" }, 2_000);
    await held.service.untaken.taken();
    await vi.waitFor(() => expect(held.passes).toEqual([{ events: 2, from: 1_000, to: 2_000, failed: 0 }]));
    const reader = new SqliteSessionReader(held.databasePath);
    try {
      // Before, the session started, and its start was recorded, when the daemon checked them: after its prompt.
      expect(reader.session(late.sessionId)?.startedAt).toBe(1_000);
      const events = reader.events(late.sessionId).filter(event => event.kind === "session-start" || event.kind === "user-prompt");
      expect(events.map(({ kind, observedAt }) => ({ kind, observedAt }))).toEqual([
        { kind: "session-start", observedAt: 1_000 },
        { kind: "user-prompt", observedAt: 2_000 },
      ]);
    } finally {
      reader.close();
    }
  });

  it("records a boundary a call left untaken met before the session's code rules opened at the time its hook met it", async () => {
    const opening = Promise.withResolvers<void>();
    const held = await serviceWithUntaken(opening.promise);
    try {
      const call = { kind: "call", eventId: "early:call", sessionId: SESSION, toolCallId: "early", toolName: "Bash", input: { command: "ls" } } as const;
      await held.leave(call, 1_000);
      await held.service.untaken.taken();
      await vi.waitFor(() => expect(held.passes).toHaveLength(1));
      // Before, the boundary's row took the time the daemon checked the call.
      const reader = new SqliteSessionReader(held.databasePath);
      try {
        expect(reader.events(SESSION).filter(event => event.kind === "code-rules-pending").map(event => event.observedAt)).toEqual([1_000]);
      } finally {
        reader.close();
      }
    } finally {
      opening.resolve();
    }
  });
});
