import { type ChildProcess, execFileSync, spawn, spawnSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import {
  chmodSync, closeSync, copyFileSync, cpSync, existsSync, ftruncateSync, lstatSync, mkdirSync, openSync, readdirSync, readFileSync, realpathSync, renameSync,
  rmSync, statSync, symlinkSync, writeFileSync, writeSync,
} from "node:fs";
import { appendFile, mkdir, readdir, readFile, rename, rm, symlink, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { basename, dirname, join, relative, resolve, sep } from "node:path";
import { Readable, Writable } from "node:stream";
import { setTimeout as delay } from "node:timers/promises";
import Database from "better-sqlite3";
import { DrizzleQueryError } from "drizzle-orm";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  adoptRemote, type AdapterDelivery, defaultRerailDatabasePath, lookupCheckout, openRerailDatabase, recordCheckout, sessionStartNote, START_MARK, startMarkFound,
  STUCK_MARK,
} from "../../src/engine/index.js";
import {
  encodeRerailEventResult,
  type EngineEndpoint,
  makeAnswersFolder,
  removeEarlierAnswers,
  ServedAsks,
  setCheckoutStarted,
  setRepositoryStarted,
  startStateReader,
  stopRerail,
  workspaceStartState,
} from "../../src/runtime/index.js";
import {
  emitHookOutput,
  holdStopSteers,
  pendingPath,
  persistPending,
  readPending,
  replacePending,
  routeDeliveries,
  scrubbedPendingText,
  serveHook,
  takePending,
  withPendingFileLock,
  withSessionLock,
  type HookInput,
  type HookOutput,
} from "../../src/harness/hook-adapter.js";
import { hookRerailStarted, type HookOptions } from "../../src/harness/hook-host.js";
import { HOOK_FAILURES_FILE } from "../../src/harness/hook-failures.js";
import { runClaudeHook } from "../../src/hosts/claude/index.js";
import { runCodexHook } from "../../src/hosts/codex/index.js";
import { runCursorHook } from "../../src/hosts/cursor/index.js";
import { hookOptions, serveText, temporaryDirectories, sessionDatabaseQuery } from "../support/hook-events.js";
import { holdEngineEndpoint, testEngineEndpoint } from "../support/platform/engine-endpoint.js";
import { pipeAt } from "../support/platform/named-pipe.js";
import { removeFolderSync } from "../support/platform/remove-folder.js";
import { startTestRepository, stopTestRepository } from "../support/started-repository.js";

// The lookup of a named engine socket, and a later hook's lookup of the running daemon, work as they do; a test stands
// in for one call of either at a time. `autostartOffError` makes the error an off autostart throws, of the class the
// hook host holds: the start checks load modules afresh (`vi.resetModules`), and daemon-control loaded in a test after
// that would give another class, whose error the hook host would log as a failure.
const lookups = vi.hoisted(() => ({
  engineEndpointAt: vi.fn<(socketPath: string) => Promise<EngineEndpoint>>(),
  runningDaemonEngineEndpoint: vi.fn<(deadline?: number) => Promise<unknown>>(),
  autostartOffError: (message: string): Error => {
    throw new Error(`daemon-control is not loaded yet, so there is no AutostartOffError for: ${message}`);
  },
}));
vi.mock("../../src/runtime/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
  lookups.engineEndpointAt.mockImplementation(socketPath => actual.engineEndpointAt(socketPath));
  lookups.runningDaemonEngineEndpoint.mockImplementation(deadline => actual.runningDaemonEngineEndpoint(deadline));
  lookups.autostartOffError = message => new actual.AutostartOffError(message);
  return { ...actual, engineEndpointAt: lookups.engineEndpointAt, runningDaemonEngineEndpoint: lookups.runningDaemonEngineEndpoint };
});

const temporaryDirectory = temporaryDirectories("rerail-hook-host-");

function delivery(content: string, ruleIds: readonly string[] = []): AdapterDelivery {
  return { sessionId: "session-1", requested: "steer", mechanism: "deferred-context", content, ruleIds, rules: [] };
}

interface HeldEngine {
  socketPath: string;
  /** The ids of the events that reached it, in the order they came. */
  received: string[];
  firstArrived: Promise<void>;
  answerFirst(): void;
  close(): Promise<void>;
}

/**
 * A daemon's engine socket that answers the first event only once `answerFirst` is called, as a busy daemon checking an
 * edit does, and every later event at once.
 */
async function heldEngine(root: string): Promise<HeldEngine> {
  const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-held-engine-${randomUUID()}` : join(root, "engine.sock");
  const received: string[] = [];
  const firstArrived = Promise.withResolvers<void>();
  const first = Promise.withResolvers<void>();
  const server = createServer(async (request, response) => {
    let body = "";
    for await (const chunk of request) body += String(chunk);
    const { event } = JSON.parse(body) as { event: { eventId: string } };
    received.push(event.eventId);
    if (received.length === 1) {
      firstArrived.resolve();
      await first.promise;
    }
    response.end(encodeRerailEventResult({ eventId: event.eventId, deliveries: [] }));
  });
  await new Promise<void>(resolve => server.listen(socketPath, resolve));
  return {
    socketPath,
    received,
    firstArrived: firstArrived.promise,
    answerFirst: () => first.resolve(),
    close: () => new Promise<void>(resolve => server.close(() => resolve())),
  };
}

function receiptCount(root: string): number {
  return sessionDatabaseQuery<{ total: number }>(root, "SELECT COUNT(*) AS total FROM events WHERE kind = 'steering-delivered'")[0]!.total;
}

function receiptPayloads(root: string): unknown[] {
  const sql = "SELECT payload_json AS payloadJson FROM events WHERE kind = 'steering-delivered' ORDER BY sequence";
  return sessionDatabaseQuery<{ payloadJson: string }>(root, sql).map(row => JSON.parse(row.payloadJson));
}

// SessionStart carries Rerail's session-start note ahead of any guidance; only the guidance earns a receipt.
const sessionStart = (host: "claude-code" | "codex") => ({
  input: (cwd: string): HookInput => ({ cwd, session_id: "session-1", hook_event_name: "SessionStart", source: "startup" }),
  expected: {
    hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: `${sessionStartNote(undefined, host, "session-1", { setUp: true, ruleCount: null })}\n\nfirst\n\nsecond` },
  },
});
const hookCases: Array<{
  host: "claude-code" | "codex" | "cursor";
  run: (input: HookInput, options: HookOptions) => Promise<HookOutput>;
  input: (root: string) => HookInput;
  expected: HookOutput;
}> = [
  { host: "claude-code", run: runClaudeHook, ...sessionStart("claude-code") },
  { host: "codex", run: runCodexHook, ...sessionStart("codex") },
  {
    host: "cursor", run: runCursorHook,
    input: cwd => ({ workspace_roots: [cwd], conversation_id: "session-1", hook_event_name: "sessionStart" }),
    expected: { additional_context: `${sessionStartNote(undefined, "cursor", "session-1", { setUp: true, ruleCount: null })}\n\nfirst\n\nsecond` },
  },
];

describe("hook steers", () => {
  it.each(hookCases)("counts one $host output only after bundled guidance is written", async ({ host, run, input, expected }) => {
    const root = await temporaryDirectory();
    const options = hookOptions(root);
    const path = pendingPath(options.storageRoot!, host, "session-1");
    await persistPending(path, [
      delivery("first", ["rule:a"]),
      delivery("second", ["rule:b", "rule:a"]),
      delivery(" ", ["rule:blank"]),
      { ...delivery("recorded only", ["rule:log"]), requested: "log" },
    ]);
    const output = await run(input(root), options);
    expect(output).toEqual(expected);
    expect(receiptCount(root)).toBe(0);

    let finish: (() => void) | undefined;
    let text = "";
    const stdout = new Writable({
      highWaterMark: 1,
      write(chunk: Buffer, _encoding, callback) {
        text += chunk.toString();
        finish = callback;
      },
    });
    const emitted = emitHookOutput(output, stdout);
    expect(receiptCount(root)).toBe(0);
    finish!();
    await emitted;
    expect(JSON.parse(text)).toEqual(expected);
    expect(receiptCount(root)).toBe(1);
    // The receipt credits only the rules behind the guidance that was written, in first-seen order.
    expect(receiptPayloads(root)).toEqual([{ ruleIds: ["rule:a", "rule:b"] }]);
    expect(await readPending(path)).toEqual([]);

    await persistPending(path, [delivery(" "), { ...delivery("log only"), requested: "log" }]);
    const empty = await run(input(root), options);
    await emitHookOutput(empty, new Writable({ write(_chunk, _encoding, callback) { callback(); } }));
    expect(receiptCount(root)).toBe(1);
  });

  it.each(hookCases)("counts no $host output whose only delivery is a note naming no rule (a Merge Assist note)", async ({ host, run, input }) => {
    const root = await temporaryDirectory();
    const options = hookOptions(root);
    const conflict = { id: "conflict:overlap:abc:12345678", text: "your edit overlaps jbarnes-1: retry.ts" };
    await persistPending(pendingPath(options.storageRoot!, host, "session-1"), [{ ...delivery("Your edit overlaps open work."), conflict }]);
    const output = await run(input(root), options);
    expect(JSON.stringify(output)).toContain("Your edit overlaps open work.");
    await emitHookOutput(output, new Writable({ write(_chunk, _encoding, callback) { callback(); } }));
    expect(receiptCount(root)).toBe(0);
  });

  it("hands a held note on whose conflict is not one, without its line", async () => {
    const root = await temporaryDirectory();
    const options = hookOptions(root);
    const path = pendingPath(options.storageRoot!, "claude-code", "session-1");
    const conflict = { id: "conflict:overlap:abc:12345678", text: "your edit overlaps jbarnes-1: retry.ts" };
    await mkdir(join(path, ".."), { recursive: true });
    await writeFile(path, JSON.stringify([{ ...delivery("kept"), conflict }, { ...delivery("unlinked"), conflict: { ...conflict, url: null } }, { ...delivery("unnamed"), conflict: "x" }]));
    expect(await readPending(path)).toEqual([{ ...delivery("kept"), conflict }, delivery("unlinked"), delivery("unnamed")]);
    const output = await runClaudeHook({ cwd: root, session_id: "session-1", hook_event_name: "SessionStart", source: "startup" }, options);
    expect(output).toMatchObject({ hookSpecificOutput: { additionalContext: expect.stringContaining("kept\n\nunlinked\n\nunnamed") } });
  });

  it("keeps a held steer's rules, so its lines can still be drawn, and reads a steer held before they were kept as naming none", async () => {
    const root = await temporaryDirectory();
    const path = pendingPath(hookOptions(root).storageRoot!, "claude-code", "session-1");
    const held: AdapterDelivery = { ...delivery("first", ["rule:a"]), rules: [{ id: "rule:a", state: "steered", origin: "From your rule library." }] };
    await persistPending(path, [held]);
    expect(await readPending(path)).toEqual([held]);
    const { rules: _rules, ...older } = delivery("older", ["rule:b"]);
    await writeFile(path, JSON.stringify([older, { ...older, rules: [{ id: 3 }, { id: "rule:b", state: "steered" }] }]));
    expect(await readPending(path)).toEqual([{ ...older, rules: [] }, { ...older, rules: [{ id: "rule:b", state: "steered" }] }]);
  });

  it("reads a steer the 0.1.2 hook held at Stop, marked shown, as held late, and drops a mark that is not true", async () => {
    const path = pendingPath(await temporaryDirectory(), "claude-code", "session-1");
    await mkdir(join(path, ".."), { recursive: true });
    await writeFile(path, JSON.stringify([{ ...delivery("old"), shown: true }, { ...delivery("new"), late: true }, { ...delivery("plain"), late: "yes" }]));
    expect(await readPending(path)).toEqual([{ ...delivery("old"), late: true }, { ...delivery("new"), late: true }, delivery("plain")]);
  });

  it("does not count synchronous, asynchronous or serialization failures", async () => {
    const root = await temporaryDirectory();
    const options = hookOptions(root);
    const path = pendingPath(options.storageRoot!, "claude-code", "session-1");
    await persistPending(path, [{ ...delivery("guidance"), requested: "steer" }]);
    const output = await runClaudeHook({ cwd: root, session_id: "session-1", hook_event_name: "SessionStart" }, options);
    await expect(emitHookOutput(output, { write() { throw new Error("write failed"); } })).rejects.toThrow("write failed");
    await expect(emitHookOutput(output, new Writable({
      write(_chunk, _encoding, callback) { callback(new Error("flush failed")); },
    }))).rejects.toThrow("flush failed");
    let failedWrite = "";
    await serveHook(async () => output, {
      stdin: Readable.from(["{}"]),
      stdout: new Writable({
        write(chunk: Buffer, _encoding, callback) {
          failedWrite += chunk.toString();
          callback(new Error("flush failed"));
        },
      }),
      stderr: { write() {} },
    });
    expect(JSON.parse(failedWrite)).toEqual({ hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: `${sessionStartNote(undefined, "claude-code", "session-1", { setUp: true, ruleCount: null })}\n\nguidance` } });
    output.circular = output;
    await expect(emitHookOutput(output)).rejects.toThrow(/circular/i);
    const fallback = await serveText(async () => output, "{}");
    expect(JSON.parse(fallback.stdout)).toEqual({});
    expect(fallback.stderr).toMatch(/^rerail hook: /);
    expect(receiptCount(root)).toBe(0);
  });

  it("does not emit a second JSON answer when receipt persistence fails after stdout succeeds", async () => {
    const root = await temporaryDirectory();
    const options = hookOptions(root);
    // A delivery that names a rule, so writing the answer records a receipt: one that names none is never counted.
    await persistPending(pendingPath(options.storageRoot!, "claude-code", "session-1"), [
      { ...delivery("guidance", ["rule:a"]), requested: "steer" },
    ]);
    const output = await runClaudeHook({ cwd: root, session_id: "session-1", hook_event_name: "SessionStart" }, options);
    const databasePath = join(root, "rerail.sqlite");
    await rename(databasePath, `${databasePath}.backup`);
    await mkdir(databasePath);
    const served = await serveText(async () => output, "{}");
    expect(JSON.parse(served.stdout)).toEqual({ hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: `${sessionStartNote(undefined, "claude-code", "session-1", { setUp: true, ruleCount: null })}\n\nguidance` } });
    expect(served.stderr).toMatch(/^rerail hook: /);
  });

  it("does nothing on a Node older than Rerail runs on, and says so in one line on stderr", async () => {
    const written = { stdout: "", stderr: "" };
    let ran = false;
    await serveHook(async () => {
      ran = true;
      return { decision: "block" };
    }, {
      stdin: Readable.from([JSON.stringify({ session_id: "session-1", hook_event_name: "SessionStart" })]),
      stdout: { write: (chunk, callback) => { written.stdout += chunk; callback(); } },
      stderr: { write: chunk => (written.stderr += chunk) },
    }, "22.23.4");
    expect(ran).toBe(false);
    expect(written.stdout).toBe("{}\n");
    expect(written.stderr).toMatch(/^rerail hook: Rerail needs Node 24 or newer and this is Node 22\.23\.4[^\n]*\n$/u);
  });
});

describe("hook host durable state", () => {
  it("preserves queued deliveries across appends until the receiving hook consumes them", async () => {
    const root = await temporaryDirectory();
    const path = pendingPath(root, "claude-code", "session-1");

    await persistPending(path, [delivery("first")]);
    await persistPending(path, [delivery("second")]);

    expect(await readPending(path)).toEqual([delivery("first"), delivery("second")]);
    expect(JSON.parse(await readFile(path, "utf8"))).toEqual([delivery("first"), delivery("second")]);
    expect(await takePending(path)).toEqual([delivery("first"), delivery("second")]);
    expect(await readPending(path)).toEqual([]);
  });

  it("keeps only steer entries from a pending file", async () => {
    const root = await temporaryDirectory();
    const path = pendingPath(root, "codex", "session-1");
    await mkdir(join(path, ".."), { recursive: true });
    await writeFile(path, JSON.stringify(["steer", "block", "log", "followUp", "nextTurn", "observe"].map(requested => ({
      ...delivery(requested), requested,
    }))));

    expect(await readPending(path)).toEqual([delivery("steer"), { ...delivery("block"), requested: "block" }]);
  });

  it.skipIf(process.platform === "win32")("refuses held steers through a symlink under .rerail, says so, and writes nothing where it leads", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, ".rerail", "sessions");
    const elsewhere = join(root, "elsewhere");
    await mkdir(storageRoot, { recursive: true });
    await mkdir(elsewhere);
    await symlink(elsewhere, join(storageRoot, "claude-code-pending"));
    const path = pendingPath(storageRoot, "claude-code", "session-1");
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    try {
      await expect(persistPending(path, [delivery("first")])).rejects.toThrow(`through ${join(storageRoot, "claude-code-pending")}: it is a symbolic link`);
      expect(await readdir(elsewhere)).toEqual([]);
      // Nor is a steer planted there read back as one Rerail held.
      await writeFile(join(elsewhere, basename(path)), JSON.stringify([delivery("planted")]));
      await expect(readPending(path)).rejects.toThrow("it is a symbolic link");
      expect(stderr).toHaveBeenCalledWith(expect.stringContaining("it is a symbolic link"));
    } finally {
      stderr.mockRestore();
    }
  });

  it("serializes one-shot work per host session and releases the lock after failure", async () => {
    const root = await temporaryDirectory();
    const order: string[] = [];
    const firstEntered = Promise.withResolvers<void>();
    const releaseFirst = Promise.withResolvers<void>();

    const first = withSessionLock(root, "codex", "session-1", async () => {
      order.push("first-start");
      firstEntered.resolve();
      await releaseFirst.promise;
      order.push("first-end");
    });
    await firstEntered.promise;
    const second = withSessionLock(root, "codex", "session-1", async () => {
      order.push("second");
    });
    releaseFirst.resolve();
    await Promise.all([first, second]);

    expect(order).toEqual(["first-start", "first-end", "second"]);
    await expect(withSessionLock(root, "codex", "session-1", async () => {
      throw new Error("ingest failed");
    })).rejects.toThrow("ingest failed");
    await expect(withSessionLock(root, "codex", "session-1", async () => "recovered")).resolves.toBe("recovered");
  });

  it("lets the session's other hooks reach the daemon while one waits for the daemon's answer, as parallel subagents' hooks do", { timeout: 20_000 }, async () => {
    const root = await temporaryDirectory();
    const engine = await heldEngine(root);
    const hooks: Promise<unknown>[] = [];
    try {
      const { embedder: _embedder, ...options } = hookOptions(root);
      // Every subagent's tool call reaches Rerail under its parent's session id.
      const toolCall = (agent: string) => runClaudeHook({
        session_id: "parent-session", cwd: root, hook_event_name: "PreToolUse", agent_id: agent, agent_type: "general-purpose",
        tool_name: "Write", tool_use_id: `${agent}-write`, tool_input: { file_path: join(root, `${agent}.py`), content: "x = 1\n" },
      }, { ...options, kernelSocketPath: engine.socketPath });
      hooks.push(toolCall("agent-a"));
      await engine.firstArrived;
      hooks.push(toolCall("agent-b"));
      await vi.waitFor(() => expect(engine.received).toEqual(["parent-session:tool-call:agent-a-write", "parent-session:tool-call:agent-b-write"]), { timeout: 5_000 });
      engine.answerFirst();
      await expect(Promise.all(hooks)).resolves.toEqual([{}, {}]);
    } finally {
      engine.answerFirst();
      await Promise.allSettled(hooks);
      await engine.close();
    }
  });

  it("finds the daemon before it takes the session's lock, so a hook waiting on a busy daemon's probe holds up no other hook", { timeout: 20_000 }, async () => {
    const root = await temporaryDirectory();
    const engine = await heldEngine(root);
    engine.answerFirst();
    const lookupStarted = Promise.withResolvers<void>();
    const lookupDone = Promise.withResolvers<void>();
    // The first hook's lookup waits as one does on a daemon too busy to answer its health probe at once.
    lookups.engineEndpointAt.mockImplementationOnce(async socketPath => {
      lookupStarted.resolve();
      await lookupDone.promise;
      return { socketPath };
    });
    const hooks: Promise<unknown>[] = [];
    try {
      const { embedder: _embedder, ...options } = hookOptions(root);
      const toolCall = (agent: string) => runClaudeHook({
        session_id: "parent-session", cwd: root, hook_event_name: "PreToolUse", agent_id: agent, agent_type: "general-purpose",
        tool_name: "Write", tool_use_id: `${agent}-write`, tool_input: { file_path: join(root, `${agent}.py`), content: "x = 1\n" },
      }, { ...options, kernelSocketPath: engine.socketPath });
      hooks.push(toolCall("agent-a"));
      await lookupStarted.promise;
      hooks.push(toolCall("agent-b"));
      await vi.waitFor(() => expect(engine.received).toEqual(["parent-session:tool-call:agent-b-write"]), { timeout: 5_000 });
      lookupDone.resolve();
      await expect(Promise.all(hooks)).resolves.toEqual([{}, {}]);
      expect(engine.received).toEqual(["parent-session:tool-call:agent-b-write", "parent-session:tool-call:agent-a-write"]);
    } finally {
      lookupDone.resolve();
      await Promise.allSettled(hooks);
      await engine.close();
    }
  });

  it("keeps a Cursor session's lock while its transcript's lines are checked, so a prompt meanwhile still turns transcript reading off", { timeout: 20_000 }, async () => {
    const root = await temporaryDirectory();
    const engine = await heldEngine(root);
    const transcript = join(root, "transcript.jsonl");
    const reply = (text: string) => `${JSON.stringify({ role: "assistant", message: { content: [{ type: "text", text }] } })}\n`;
    await writeFile(transcript, reply("Reading the file."));
    const { embedder: _embedder, ...options } = hookOptions(root);
    const session = { conversation_id: "session-1", workspace_roots: [root], transcript_path: transcript };
    const run = (input: HookInput) => runCursorHook({ ...session, ...input }, { ...options, kernelSocketPath: engine.socketPath });
    const toolResult = (id: string) => run({ hook_event_name: "postToolUse", tool_use_id: id, tool_name: "Read", tool_output: "x = 1" });
    const hooks: Promise<unknown>[] = [];
    try {
      // A tool result's hook checks the transcript's new reply, and the daemon holds that check.
      hooks.push(toolResult("read-1"));
      await engine.firstArrived;
      // The person's next prompt reaches Rerail through Cursor's own prompt hook, so the transcript is read no more.
      hooks.push(run({ hook_event_name: "beforeSubmitPrompt", prompt: "Next." }));
      // Time for the prompt's hook to run, were the session's lock free.
      await delay(500);
      engine.answerFirst();
      await Promise.all(hooks);
      await appendFile(transcript, reply("Done."));
      await toolResult("read-2");
      expect(engine.received.filter(id => id.includes(":transcript:"))).toHaveLength(1);
    } finally {
      engine.answerFirst();
      await Promise.allSettled(hooks);
      await engine.close();
    }
  });

  it("holds steers on disk scrubbed of secrets and hands the hook their original text", async () => {
    const root = await temporaryDirectory();
    const path = pendingPath(root, "claude-code", "session-1");
    // Assembled from parts, so no secret scanner takes this file for one holding a key.
    const secret = ["glpat", "-x7Kq2mZ9vL4pR8tN1wB6"].join("");
    const leaky = delivery(`Move the key in \`const key = "${secret}"\` to the environment.`);
    const stored = { ...leaky, content: leaky.content.replace(secret, `[redacted:gitlab-token:${createHash("sha256").update(secret).digest("hex").slice(0, 6)}]`) };

    await holdStopSteers(path, [leaky]);
    expect(await readFile(path, "utf8")).not.toContain(secret);
    expect(await readPending(path)).toEqual([{ ...stored, late: true }]);

    await replacePending(path, [leaky, delivery("plain")]);
    expect(await readFile(path, "utf8")).not.toContain(secret);
    expect(await readPending(path)).toEqual([stored, delivery("plain")]);

    // A context event delivers the steers it carries as they came; the one held earlier comes back as it was stored.
    expect(await routeDeliveries(path, "SessionStart", [leaky], { SessionStart: true })).toEqual([stored, delivery("plain"), leaky]);
    await persistPending(path, [leaky]);
    expect(await readFile(path, "utf8")).not.toContain(secret);
  });

  it("rewrites a pending file held before steers were scrubbed, and leaves a clean or foreign one", async () => {
    const secret = ["glpat", "-x7Kq2mZ9vL4pR8tN1wB6"].join("");
    const older = JSON.stringify([delivery(`key ${secret}`), delivery("plain"), "stray"]);
    const rewritten = scrubbedPendingText(older);
    expect(rewritten).not.toContain(secret);
    expect(JSON.parse(rewritten!)).toEqual([
      delivery(`key [redacted:gitlab-token:${createHash("sha256").update(secret).digest("hex").slice(0, 6)}]`), delivery("plain"), "stray",
    ]);
    expect(scrubbedPendingText(rewritten!)).toBeUndefined();
    expect(scrubbedPendingText("{\"not\": \"an array\"}")).toBeUndefined();
    expect(scrubbedPendingText("not json")).toBeUndefined();
  });

  it("locks a pending file with the lock its session's hooks hold", async () => {
    const root = await temporaryDirectory();
    const order: string[] = [];
    const hookEntered = Promise.withResolvers<void>();
    const releaseHook = Promise.withResolvers<void>();

    const hook = withSessionLock(root, "cursor", "session-1", async () => {
      order.push("hook-start");
      hookEntered.resolve();
      await releaseHook.promise;
      order.push("hook-end");
    });
    await hookEntered.promise;
    const scrub = withPendingFileLock(pendingPath(root, "cursor", "session-1"), async () => {
      order.push("scrub");
    });
    releaseHook.resolve();
    await Promise.all([hook, scrub]);

    expect(order).toEqual(["hook-start", "hook-end", "scrub"]);
    await expect(withPendingFileLock(join(root, "cursor-pending", "notes.json"), async () => undefined)).rejects.toThrow("Not a pending steer file");
  });
});

interface CountingDaemon {
  /** The path of every request it got, in order. */
  requests: string[];
  /** The folder each start check asked about, as the hook spelled it, in order. */
  asked: string[];
  /** How many start checks (`/api/presentation`) it held were ended by the hook before it answered them. */
  ended: () => number;
  close: () => Promise<void>;
}

/**
 * A daemon recorded in the Rerail home `home` that answers as a live one does and counts every request it gets: a
 * repository's start state is `repositoryState`, or what it gives for the folder asked about. With `answerWhen`, it
 * holds each start check until the promise `answerWhen` gives for its address settles, as a daemon busy with other
 * checkouts does, and `close` lets every held one go. On Windows a recorded daemon counts as alive only while its
 * engine pipe exists (`recordedDaemonAlive`), so it holds one. It keeps the Rerail database open where there is one, as
 * a running daemon does, so the database's WAL and the WAL's index stay beside it, unless `holdsDatabase` is false: a
 * test whose daemon's connection is another process's gives that. With an answer of stopped it names the checkout it
 * placed the folder in, as a live daemon does (`scopeOf`, by default `daemonScope`). `daemon.json` names it by `pid`
 * and `startedAt`, this process and 1 unless given, and the database it reads (`databasePath`).
 */
async function countingDaemon(
  home: string,
  repositoryState: string | ((repository: string) => string) = "stopped",
  answerWhen?: (url: URL) => Promise<unknown>,
  { holdsDatabase = true, pid = process.pid, startedAt = 1, scopeOf = daemonScope, databasePath = join(home, "rerail.sqlite"), answersFolder = true, marks = "removed" }: StandInOptions = {},
): Promise<CountingDaemon> {
  const asks = new ServedAsks();
  const requests: string[] = [];
  const asked: string[] = [];
  let ended = 0;
  const closing = Promise.withResolvers<void>();
  const answers: Record<string, (url: URL) => unknown> = {
    "/healthz": () => ({ status: "ok", pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
    "/api/presentation": url => {
      const folder = url.searchParams.get("repository") ?? "";
      const state = typeof repositoryState === "string" ? repositoryState : repositoryState(folder);
      return { presence: "compact", repositoryState: state, scope: state === "stopped" ? scopeOf(folder) : null };
    },
  };
  const server = createServer(async (request, response) => {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    requests.push(url.pathname);
    if (url.pathname === "/api/presentation") asked.push(url.searchParams.get("repository") ?? "");
    if (url.pathname === "/api/presentation" && answerWhen !== undefined) {
      let gone = false;
      response.on("close", () => {
        gone = !response.writableFinished;
        if (gone) ended += 1;
      });
      await Promise.race([answerWhen(url), closing.promise]);
      if (marks === "removed") asks.served(url.searchParams.get("repository") ?? "");
      if (gone) return;
    } else if (url.pathname === "/api/presentation" && marks === "removed") {
      asks.served(url.searchParams.get("repository") ?? "");
    }
    const answer = Object.hasOwn(answers, url.pathname) ? answers[url.pathname] : undefined;
    response.writeHead(answer === undefined ? 404 : 200, { "content-type": "application/json" });
    response.end(JSON.stringify(answer === undefined ? { error: "not-found" } : answer(url)));
  });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", () => resolve()));
  const { port } = server.address() as AddressInfo;
  const kernelSocketPath = testEngineEndpoint(home);
  const releaseEndpoint = await holdEngineEndpoint(kernelSocketPath);
  await mkdir(home, { recursive: true });
  await writeFile(join(home, "daemon.json"), JSON.stringify({
    pid, host: "127.0.0.1", port, url: `http://127.0.0.1:${port}/`, version: "test",
    databasePath, kernelSocketPath, ui: "bundled", startedAt, packageRoot: home,
  }));
  if (answersFolder) makeAnswersFolder(home);
  if (answersFolder && marks !== "none") asks.open(home, { pid, startedAt });
  const database = holdsDatabase ? heldOpen(join(home, "rerail.sqlite")) : undefined;
  return {
    requests,
    asked,
    ended: () => ended,
    close: async () => {
      closing.resolve();
      database?.close();
      await releaseEndpoint();
      await new Promise<void>(resolve => server.close(() => resolve()));
    },
  };
}

/** The checkout a daemon names for the folder it placed: its repository and its top. */
type DaemonScope = { repositoryKey: string; checkoutPath: string } | null;

/** How a stand-in daemon (`countingDaemon`) runs, where a test needs it otherwise than a plain one. */
interface StandInOptions {
  readonly holdsDatabase?: boolean;
  readonly pid?: number | undefined;
  readonly startedAt?: number | undefined;
  readonly scopeOf?: (folder: string) => DaemonScope;
  /** The database `daemon.json` names; the Rerail home's unless given. */
  readonly databasePath?: string;
  /** Whether it makes the folder of kept answers at its start, as a daemon does (`makeAnswersFolder`); true unless given. */
  readonly answersFolder?: boolean;
  /**
   * Its folder of marks of start checks hooks gave up on (`ServedAsks`): `removed` as it serves each start check, as a
   * daemon does (unless given); `kept`, made but never removed; `none`, never made, as by a daemon of an older build.
   */
  readonly marks?: "removed" | "kept" | "none";
}

/** The checkout a live daemon names for `folder` in its answer (`lookupCheckout`); null where it knows none. */
function daemonScope(folder: string): DaemonScope {
  if (!existsSync(defaultRerailDatabasePath())) return null;
  const database = openRerailDatabase(defaultRerailDatabasePath(), { readonly: true });
  try {
    const checkout = lookupCheckout(database, folder);
    return checkout === undefined ? null : { repositoryKey: checkout.repositoryKey, checkoutPath: checkout.checkoutPath };
  } finally {
    database.$client.close();
  }
}

/**
 * A read-only connection to the database at `path` that has read it once, which makes its WAL and the WAL's index;
 * undefined where there is no database yet, or another process holds it so that a read fails at once. Read-only, so
 * it never writes the file nor runs a schema step on it, and closing it leaves the WAL and the index there.
 */
function heldOpen(path: string): Database.Database | undefined {
  if (!existsSync(path)) return undefined;
  let database: Database.Database | undefined;
  try {
    database = new Database(path, { readonly: true, fileMustExist: true, timeout: 0 });
    database.prepare("SELECT count(*) FROM sqlite_master").get();
    return database;
  } catch {
    database?.close();
    return undefined;
  }
}

/** A daemon busy with other checkouts the whole test: it answers no start check until it closes. */
const busyTheWholeTest = (): Promise<never> => new Promise<never>(() => undefined);

/**
 * What `pending` gives, once it has: within 1 s, half the 2 s a hook waits on its read of a busy daemon before it gives
 * up and reads the database through Git, so an answer here is one the hook gave without the daemon.
 */
async function answeredWellBeforeReadLimit<T>(pending: Promise<T>): Promise<T> {
  const begun = performance.now();
  const value = await pending;
  expect(performance.now() - begun).toBeLessThan(1_000);
  return value;
}

/**
 * How many start checks `daemon` got and how many of them the hook ended, once every one has ended or 1 s has passed:
 * well inside the 2 s a hook would otherwise wait on its read before giving up.
 */
async function endedStartChecks(daemon: CountingDaemon): Promise<{ asked: number; ended: number }> {
  const asked = (): number => daemon.requests.filter(path => path === "/api/presentation").length;
  for (let waited = 0; daemon.ended() < asked() && waited < 1_000; waited += 20) await delay(20);
  return { asked: asked(), ended: daemon.ended() };
}

// Each host's session start, the one hook that may start a daemon, and a tool call, the hook every tool call runs.
const hooksWhereRerailIsOff: Array<{ host: string; event: string; run: (cwd: string) => Promise<HookOutput> }> = [
  { host: "claude-code", event: "SessionStart", run: cwd => runClaudeHook({ cwd, session_id: "session-1", hook_event_name: "SessionStart", source: "startup" }) },
  {
    host: "claude-code", event: "PreToolUse",
    run: cwd => runClaudeHook({ cwd, session_id: "session-1", hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: "tool-1", tool_input: { file_path: "README.md" } }),
  },
  { host: "codex", event: "SessionStart", run: cwd => runCodexHook({ cwd, session_id: "session-1", hook_event_name: "SessionStart", source: "startup" }) },
  {
    host: "codex", event: "PreToolUse",
    run: cwd => runCodexHook({ cwd, session_id: "session-1", hook_event_name: "PreToolUse", tool_name: "shell", tool_use_id: "tool-1", tool_input: { command: "ls" } }),
  },
  { host: "cursor", event: "sessionStart", run: cwd => runCursorHook({ workspace_roots: [cwd], conversation_id: "session-1", hook_event_name: "sessionStart" }) },
  {
    host: "cursor", event: "beforeShellExecution",
    run: cwd => runCursorHook({ workspace_roots: [cwd], conversation_id: "session-1", hook_event_name: "beforeShellExecution", command: "ls", cwd }),
  },
];

describe("hooks where Rerail does not run", () => {
  beforeEach(() => {
    // The hook takes CLAUDE_PROJECT_DIR over the event's folder; a test run from inside a Claude Code session has one.
    vi.stubEnv("CLAUDE_PROJECT_DIR", undefined);
  });
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it.each(hooksWhereRerailIsOff)("answer $host's $event in a stopped checkout without waiting for a busy daemon that last answered stopped there, ending their request", async ({ run }) => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const answered = await countingDaemon(process.env.RERAIL_HOME!, "stopped");
    try {
      expect(await run(app)).toEqual({});
    } finally {
      await answered.close();
    }
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "started", busyTheWholeTest);
    try {
      expect(await answeredWellBeforeReadLimit(run(app))).toEqual({});
      const { asked, ended } = await endedStartChecks(daemon);
      expect(ended).toBe(asked);
    } finally {
      await daemon.close();
    }
  });

  it("starts no daemon from a stopped checkout when none runs", async () => {
    const root = await temporaryDirectory({ started: false });
    startTestRepository(root);
    stopTestRepository(root);
    for (const { run } of hooksWhereRerailIsOff) expect(await run(root)).toEqual({});
    await expect(readFile(join(process.env.RERAIL_HOME!, "daemon.json"))).rejects.toMatchObject({ code: "ENOENT" });
  });
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", ...args], { cwd, stdio: "ignore" });
}

/** A Git repository at `path` with one commit. */
function gitRepository(path: string): string {
  mkdirSync(path, { recursive: true });
  writeFileSync(join(path, "README.md"), "app\n");
  git(path, "init", "-q", "-b", "main");
  git(path, "add", ".");
  git(path, "commit", "-q", "-m", "base");
  return path;
}

const REMOTE = "https://example.invalid/team/app.git";

/** A started repository at `<root>/app` whose `origin` is `REMOTE`, recorded through this process's Git. */
function startedApp(root: string): string {
  const app = gitRepository(join(root, "app"));
  git(app, "remote", "add", "origin", REMOTE);
  startTestRepository(app);
  return app;
}

/** A new clone at `path` of `startedApp`'s repository, which Rerail finds by its remote alone. */
function newClone(path: string): string {
  const clone = gitRepository(path);
  git(clone, "remote", "add", "origin", REMOTE);
  return clone;
}

/** A Git config file at `path` that gives every address under https://example.invalid/ another, as a mirror setting does. */
function remoteRewrite(path: string): string {
  writeFileSync(path, "[url \"https://mirror.invalid/\"]\n\tinsteadOf = https://example.invalid/\n");
  return path;
}

/** A started repository and a new clone of it under `root`, with a config file named `file` that rewrites the remote. */
function cloneWithRewrite(file: string): (root: string) => { workspace: string; value: string } {
  return root => {
    startedApp(root);
    return { workspace: newClone(join(root, "clone")), value: remoteRewrite(join(root, file)) };
  };
}

/**
 * A started repository recorded while `variable` names a folder whose Git config file `file` rewrites the remote, and a
 * new clone of it; the agent's value names an empty folder. Git reads the files under `HOME` only while
 * `XDG_CONFIG_HOME` names none.
 */
function personalFileRewrite(root: string, variable: string, file: string): { workspace: string; value: string } {
  vi.stubEnv("XDG_CONFIG_HOME", undefined);
  const person = join(root, "person");
  mkdirSync(dirname(join(person, file)), { recursive: true });
  mkdirSync(join(root, "agent"));
  remoteRewrite(join(person, file));
  vi.stubEnv(variable, person);
  startedApp(root);
  return { workspace: newClone(join(root, "clone")), value: join(root, "agent") };
}

/**
 * Settings of a person's own Git that the coding agent's environment may set otherwise than the daemon's, each with a
 * folder of a started repository that this process's Git then reads as stopped: a ceiling at the checkout's top hides
 * the checkout from a folder inside it; a config file that rewrites the remote while the repository is recorded, named
 * by `GIT_CONFIG_GLOBAL` or `GIT_CONFIG_SYSTEM` or found under `XDG_CONFIG_HOME` or `HOME`, gives a new clone another
 * address, and leaving out the system file does the same; `GIT_DIR` sends Git to another repository. A test cannot write
 * the machine's system file, so `GIT_CONFIG_SYSTEM` stands in for it while the repository is recorded.
 */
const agentGitSettings: Array<{ variable: string; arrange: (root: string) => { workspace: string; value: string } }> = [
  {
    variable: "GIT_CEILING_DIRECTORIES",
    arrange: root => {
      const app = startedApp(root);
      mkdirSync(join(app, "src"));
      return { workspace: join(app, "src"), value: realpathSync.native(app) };
    },
  },
  { variable: "GIT_CONFIG_GLOBAL", arrange: cloneWithRewrite("global.gitconfig") },
  { variable: "GIT_CONFIG_SYSTEM", arrange: cloneWithRewrite("system.gitconfig") },
  {
    variable: "GIT_CONFIG_NOSYSTEM",
    arrange: root => {
      vi.stubEnv("GIT_CONFIG_SYSTEM", remoteRewrite(join(root, "system.gitconfig")));
      startedApp(root);
      vi.stubEnv("GIT_CONFIG_SYSTEM", undefined);
      return { workspace: newClone(join(root, "clone")), value: "1" };
    },
  },
  { variable: "XDG_CONFIG_HOME", arrange: root => personalFileRewrite(root, "XDG_CONFIG_HOME", join("git", "config")) },
  { variable: "HOME", arrange: root => personalFileRewrite(root, "HOME", ".gitconfig") },
  {
    variable: "GIT_DIR",
    arrange: root => {
      const app = startedApp(root);
      mkdirSync(join(app, "src"));
      return { workspace: join(app, "src"), value: join(gitRepository(join(root, "other")), ".git") };
    },
  },
];

/**
 * What each check of the hook start check found, in order: whether it may repeat the daemon's last answer of stopped
 * (`repeatsStoppedAnswer`).
 */
interface ProofLog {
  readonly found: Array<boolean | "threw">;
  /** How long each proof took, in ms, in the same order. */
  readonly took: number[];
  run(proof: () => boolean): boolean;
  /** Marks the start of another start check: `ran` then waits for its proof. */
  nextCheck(): void;
  /** Settles once the proof of the start check under way has run. */
  ran(): Promise<void>;
}

function proofLog(): ProofLog {
  const found: Array<boolean | "threw"> = [];
  const took: number[] = [];
  const waiting: Array<{ count: number; resolve: () => void }> = [];
  let before = 0;
  const record = (result: boolean | "threw", begun: number): void => {
    took.push(performance.now() - begun);
    found.push(result);
    for (const each of waiting.filter(entry => found.length >= entry.count)) each.resolve();
  };
  return {
    found,
    took,
    run: proof => {
      const begun = performance.now();
      try {
        const result = proof();
        record(result, begun);
        return result;
      } catch (error) {
        record("threw", begun);
        throw error;
      }
    },
    nextCheck: () => {
      before = found.length;
    },
    ran: () => {
      const count = before + 1;
      return found.length >= count ? Promise.resolve() : new Promise(resolve => waiting.push({ count, resolve }));
    },
  };
}

/** The hook host with what its start check does recorded. */
interface StartCheckHost {
  hookRerailStarted: typeof hookRerailStarted;
  /** The first word of each command Rerail starts Git with. */
  git: string[];
  /** Each database it opens. */
  opened: string[];
  proofs: ProofLog;
  /** What the daemon answers for `folder` (`startStateReader`), with the folders' names spelled as this host spells them. */
  daemonReads: (folder: string) => string;
  /** The checkout the daemon names for `folder` in its answer (`daemonScope`), spelled as this host spells it. */
  daemonScope: (folder: string) => DaemonScope;
}

/**
 * Folders reached under another letter case than the disk stores, and the system whose realpaths a test reads them
 * with: `macos`, where the native realpath gives each name as stored and the JS realpath, which checkout keys go
 * through there, keeps it as typed; or `linux-case-insensitive`, a case-insensitive folder on Linux, where every
 * realpath keeps it as typed and only the kernel (Git's `getcwd`) names it as it cached it.
 */
interface Spelling {
  readonly system: "macos" | "linux-case-insensitive";
  /** Each folder as typed and as stored. On a case-sensitive filesystem the typed one is a link to the stored one. */
  readonly folders: ReadonlyArray<{ readonly typed: string; readonly stored: string }>;
}

/** `path` and below. */
function under(path: string, root: string): boolean {
  return path === root || path.startsWith(root + sep);
}

/**
 * What a realpath gave for `input` (`output`), read as `spelling`'s system gives it: a name typed in another letter
 * case stays as typed. Windows and macOS give that themselves; on a case-sensitive filesystem the typed name is a link
 * the realpath followed, and this puts the typed name back.
 */
function asTyped(spelling: Spelling, input: string, output: string): string {
  const folder = spelling.folders.find(({ typed, stored }) => under(resolve(input), typed) && under(output, stored));
  return folder === undefined ? output : folder.typed + output.slice(folder.stored.length);
}

/** The realpaths of `node:fs/promises` and `checkout-control.ts` as `spelling`'s system answers them. */
function mockSpelling(spelling: Spelling): void {
  vi.doMock("../../src/engine/catalog/checkout-control.js", async () => {
    const actual = await vi.importActual<typeof import("../../src/engine/catalog/checkout-control.js")>("../../src/engine/catalog/checkout-control.js");
    const keys = spelling.system === "macos" ? "darwin" : "linux";
    const stored = spelling.system === "macos" ? actual.storedPath : (path: string) => asTyped(spelling, path, actual.storedPath(path));
    return {
      ...actual,
      physicalPath: (path: string) => asTyped(spelling, path, actual.physicalPath(path, keys)),
      // Every system but Windows keys checkouts through the JS realpath.
      checkoutRealpath: (path: string) => asTyped(spelling, path, realpathSync(path)),
      storedPath: stored,
    };
  });
  if (spelling.system === "macos") return;
  vi.doMock("node:fs/promises", async () => {
    const actual = await vi.importActual<typeof import("node:fs/promises")>("node:fs/promises");
    const realpath = async (path: string): Promise<string> => asTyped(spelling, path, await actual.realpath(path));
    return { ...actual, default: { ...actual, realpath }, realpath };
  });
}

/**
 * The hook host over this machine's Git, database and folders, loaded afresh with its start check's seams recorded:
 * Git commands, database opens and checks, which `proof` replaces when given. With `spelling`, its realpaths answer as
 * that system's do.
 */
async function startCheckHost(proof?: (workspace: string) => boolean, spelling?: Spelling): Promise<StartCheckHost> {
  const seen = { git: [] as string[], opened: [] as string[], proofs: proofLog() };
  vi.resetModules();
  if (spelling !== undefined) mockSpelling(spelling);
  vi.doMock("../../src/engine/git.js", async () => {
    const actual = await vi.importActual<typeof import("../../src/engine/git.js")>("../../src/engine/git.js");
    return {
      ...actual,
      runGit: (cwd: string, args: readonly string[], options?: Parameters<typeof actual.runGit>[2]) => {
        seen.git.push(args[0] ?? "");
        return actual.runGit(cwd, args, options);
      },
      gitText: (cwd: string, args: readonly string[]) => {
        seen.git.push(args[0] ?? "");
        return actual.gitText(cwd, args);
      },
    };
  });
  vi.doMock("../../src/engine/storage/database.js", async () => {
    const actual = await vi.importActual<typeof import("../../src/engine/storage/database.js")>("../../src/engine/storage/database.js");
    const openRerailDatabase: typeof actual.openRerailDatabase = (path, options) => {
      seen.opened.push(path);
      return actual.openRerailDatabase(path, options);
    };
    const openRerailDatabaseAsIs: typeof actual.openRerailDatabaseAsIs = (...open) => {
      seen.opened.push(open[0]);
      return actual.openRerailDatabaseAsIs(...open);
    };
    return { ...actual, openRerailDatabase, openRerailDatabaseAsIs };
  });
  vi.doMock("../../src/runtime/start-check/stopped-answers.js", async () => {
    const actual = await vi.importActual<typeof import("../../src/runtime/start-check/stopped-answers.js")>("../../src/runtime/start-check/stopped-answers.js");
    const repeatsStoppedAnswer: typeof actual.repeatsStoppedAnswer = (workspace, now) =>
      seen.proofs.run(() => proof === undefined ? actual.repeatsStoppedAnswer(workspace, now) : proof(workspace));
    return { ...actual, repeatsStoppedAnswer };
  });
  const host = await import("../../src/harness/hook-host.js");
  const { startStateReader: reader } = await import("../../src/runtime/index.js");
  const { lookupCheckout: lookup } = await import("../../src/engine/index.js");
  const withDatabase = <T>(read: (database: ReturnType<typeof openRerailDatabase>) => T): T => {
    const database = openRerailDatabase(defaultRerailDatabasePath(), { readonly: true });
    try {
      return read(database);
    } finally {
      database.$client.close();
    }
  };
  const daemonReads = (folder: string): string => withDatabase(database => reader(database)(folder));
  const daemonScope = (folder: string): DaemonScope => withDatabase(database => {
    const checkout = lookup(database, folder);
    return checkout === undefined ? null : { repositoryKey: checkout.repositoryKey, checkoutPath: checkout.checkoutPath };
  });
  return { ...seen, hookRerailStarted: host.hookRerailStarted, daemonReads, daemonScope };
}

/**
 * A daemon recorded in the Rerail home that answers each start check only once the hook has run its own check for it,
 * as a daemon busy with other checkouts answers after the hook's grace: the hook's repeat of the daemon's last answer,
 * where it gives one, comes first, and anywhere else it takes this daemon's. It names the checkout it placed a folder in
 * as `host` reads it.
 */
async function daemonAnsweringAfterProof(
  host: StartCheckHost, repositoryState: string | ((repository: string) => string), options: StandInOptions = {},
): Promise<CountingDaemon> {
  return await countingDaemon(process.env.RERAIL_HOME!, repositoryState, () => host.proofs.ran(), { scopeOf: host.daemonScope, ...options });
}

/**
 * The file of the daemon's last answer of stopped the hook keeps for `folder`, from the daemon `daemon.json` names by
 * `pid` and `startedAt` (`countingDaemon`'s, unless given).
 */
function keptAnswerFile(folder: string, { pid = process.pid, startedAt = 1 }: Pick<StandInOptions, "pid" | "startedAt"> = {}): string {
  const name = createHash("sha256").update(realpathSync.native(folder)).digest("hex").slice(0, 32);
  return join(process.env.RERAIL_HOME!, "stopped-answers", `${pid}-${startedAt}`, `${name}.json`);
}

/**
 * The marks of `folder`'s start checks in the folder of marks of the daemon `daemon.json` names by `pid` and `startedAt`
 * (`countingDaemon`'s, unless given): `asking`, `waiting`, or neither.
 */
function askMarks(folder: string, daemon: Pick<StandInOptions, "pid" | "startedAt"> = {}): string[] {
  const named = keptAnswerFile(folder, daemon).replace(/\.json$/u, "");
  return ["asking", "waiting"].filter(kind => existsSync(join(dirname(named), "asks", `${basename(named)}.${kind}`)));
}

/** A mark of `kind` for `folder`'s start checks in the daemon's folder of marks, made where it is not there yet. */
function plantMark(folder: string, kind: "asking" | "waiting", daemon: Pick<StandInOptions, "pid" | "startedAt"> = {}): void {
  const named = keptAnswerFile(folder, daemon).replace(/\.json$/u, "");
  mkdirSync(join(dirname(named), "asks"), { recursive: true });
  writeFileSync(join(dirname(named), "asks", `${basename(named)}.${kind}`), "");
}

/** What is in the daemon's folder of the answer kept for `folder`, but its folder of marks (`askMarks`). */
function answersBeside(folder: string): string[] {
  return readdirSync(dirname(keptAnswerFile(folder))).filter(name => name !== "asks");
}

/** The answer kept for `folder`, as it was kept but at `at`. */
function keptAt(folder: string, at: number): void {
  const file = keptAnswerFile(folder);
  writeFileSync(file, JSON.stringify({ ...JSON.parse(readFileSync(file, "utf8")) as object, at }));
}

/** Whether `text` is a whole kept answer: it parses, and holds every field one is written with. */
function wholeKeptAnswer(text: string): boolean {
  try {
    const kept = JSON.parse(text) as Record<string, unknown>;
    return kept.v === 4 && ["repository", "cloneId", "folder", "checkout", "git", "database", "wal", "walIndex", "mark"].every(field => typeof kept[field] === "string")
      && typeof kept.depth === "number" && typeof kept.at === "number";
  } catch {
    return false;
  }
}

/** The codes Windows gives a moment while another program holds a file or is deleting it (`heldOnWindows`). */
const WINDOWS_HOLDS: readonly string[] = ["EPERM", "EACCES", "EBUSY"];

/** What the hook's reads of kept answers gave and what its renames onto them did. */
interface KeptAnswerTraffic {
  /** Each read of a kept answer: the text it gave, or the code it failed with. */
  readonly reads: Array<{ readonly text: string } | { readonly code: string }>;
  /** Each rename onto a kept answer: `renamed`, or the code it failed with. */
  readonly renames: string[];
}

/**
 * Records {@link KeptAnswerTraffic} of the hook host the test loads next (`startCheckHost`), through `node:fs` and
 * `node:fs/promises`. The hook opens a kept answer refusing a link at its name (`openNoFollowSync`) and reads it by its
 * descriptor with one `readSync`, so an open that fails is a read that fails.
 */
function watchKeptAnswers(): KeptAnswerTraffic {
  const traffic: KeptAnswerTraffic = { reads: [], renames: [] };
  const kept = (path: unknown): boolean => typeof path === "string" && path.includes(`${sep}stopped-answers${sep}`) && path.endsWith(".json");
  vi.doMock("node:fs", async () => {
    const actual = await vi.importActual<typeof import("node:fs")>("node:fs");
    const descriptors = new Set<number>();
    const openSync = (...args: Parameters<typeof actual.openSync>): number => {
      if (!kept(args[0])) return actual.openSync(...args);
      try {
        const descriptor = actual.openSync(...args);
        descriptors.add(descriptor);
        return descriptor;
      } catch (error) {
        traffic.reads.push({ code: codeOf(error) });
        throw error;
      }
    };
    const closeSync = (descriptor: number): void => {
      descriptors.delete(descriptor);
      actual.closeSync(descriptor);
    };
    const readSync = (...args: Parameters<typeof actual.readSync>): number => {
      if (!descriptors.has(args[0])) return actual.readSync(...args);
      try {
        const read = actual.readSync(...args);
        // The hook reads a kept answer into the start of its buffer.
        traffic.reads.push({ text: Buffer.from(args[1].buffer, args[1].byteOffset, read).toString("utf8") });
        return read;
      } catch (error) {
        traffic.reads.push({ code: codeOf(error) });
        throw error;
      }
    };
    const replaced = { openSync, closeSync, readSync };
    return { ...actual, default: { ...actual, ...replaced }, ...replaced };
  });
  vi.doMock("node:fs/promises", async () => {
    const actual = await vi.importActual<typeof import("node:fs/promises")>("node:fs/promises");
    const rename = async (from: string, to: string): Promise<void> => {
      if (!kept(to)) return await actual.rename(from, to);
      try {
        await actual.rename(from, to);
        traffic.renames.push("renamed");
      } catch (error) {
        traffic.renames.push(codeOf(error));
        throw error;
      }
    };
    return { ...actual, default: { ...actual, rename }, rename };
  });
  return traffic;
}

/**
 * Each path under a Rerail home's `stopped-answers` folder that the hook host the test loads next (`startCheckHost`)
 * removes, through `node:fs` and `node:fs/promises`.
 */
function watchRemovals(): string[] {
  const removed: string[] = [];
  const answers = (path: unknown): boolean => typeof path === "string" && path.includes(`${sep}stopped-answers${sep}`);
  vi.doMock("node:fs", async () => {
    const actual = await vi.importActual<typeof import("node:fs")>("node:fs");
    const rmSync = (...args: Parameters<typeof actual.rmSync>): void => {
      if (answers(args[0])) removed.push(String(args[0]));
      actual.rmSync(...args);
    };
    return { ...actual, default: { ...actual, rmSync }, rmSync };
  });
  vi.doMock("node:fs/promises", async () => {
    const actual = await vi.importActual<typeof import("node:fs/promises")>("node:fs/promises");
    const rm = async (...args: Parameters<typeof actual.rm>): Promise<void> => {
      if (answers(args[0])) removed.push(String(args[0]));
      await actual.rm(...args);
    };
    return { ...actual, default: { ...actual, rm }, rm };
  });
  return removed;
}

/**
 * A stand-in daemon's timing for `hooksAtOnce`: it answers each Codex hook's start check at once, and that hook writes the
 * answer again over one kept over a minute ago; it holds each Claude Code hook's, as a busy daemon does, and that hook
 * repeats the answer kept.
 */
const codexAnsweredAtOnce = (url: URL): Promise<unknown> => (url.searchParams.get("host") === "codex" ? Promise.resolve() : busyTheWholeTest());

/** `count` start checks in `folder` at once, every other one a Codex hook's that waits for the daemon's answer however long it takes. */
function hooksAtOnce(host: StartCheckHost, folder: string, count: number): Array<Promise<boolean>> {
  return Array.from({ length: count }, (_, index) => (index % 2 === 0
    ? host.hookRerailStarted("codex", folder, undefined, GRACE_NO_ANSWER_MISSES)
    : host.hookRerailStarted("claude-code", folder)));
}

/** Each hook's answer, or what it threw. */
function settled(results: ReadonlyArray<PromiseSettledResult<boolean>>): Array<boolean | string> {
  return results.map(result => (result.status === "fulfilled" ? result.value : String(result.reason)));
}

/**
 * The first start check in `workspace` beside a daemon that answers stopped late, after the hook's own check: with no
 * answer of the daemon's kept there yet, the hook waits for it and takes it, opening no database, and keeps it.
 */
async function keptStopped(host: StartCheckHost, workspace: string, options: StandInOptions = {}): Promise<void> {
  const daemon = await daemonAnsweringAfterProof(host, "stopped", options);
  try {
    host.proofs.nextCheck();
    const opened = host.opened.length;
    expect({
      started: await host.hookRerailStarted("claude-code", workspace), checked: host.proofs.found.at(-1), requests: daemon.requests,
      ended: daemon.ended(), opened: host.opened.slice(opened), kept: existsSync(keptAnswerFile(workspace, options)),
    }).toEqual({ started: false, checked: false, requests: ["/api/presentation"], ended: 0, opened: [], kept: true });
  } finally {
    await daemon.close();
  }
}

/**
 * Each entry in the Rerail home and below it, sorted, but SQLite's own WAL and WAL index beside a database, which a read
 * of the database leaves as it does on main.
 */
function homeEntries(): string[] {
  return readdirSync(process.env.RERAIL_HOME!, { recursive: true }).map(String).filter(name => !/\.sqlite-(wal|shm)$/u.test(name)).sort();
}

/** What is at `path`, not followed through a link. */
function entryOf(path: string): "folder" | "link" | "file" {
  const found = lstatSync(path);
  if (found.isSymbolicLink()) return "link";
  return found.isDirectory() ? "folder" : "file";
}

/** A Git repository at `<root>/app`, started and then stopped in its checkout, as `rerail stop` leaves it. */
function stoppedApp(root: string): string {
  const app = gitRepository(join(root, "app"));
  startTestRepository(app);
  stopTestRepository(app);
  return app;
}

/**
 * A folder of each kind the start check meets, each with a daemon's answer that differs from the database's, so the
 * hook's answer shows which one gave it.
 */
const foldersOfEachKind = [
  { folder: "a stopped checkout", daemonSays: "started", started: true, make: (root: string) => stoppedApp(root) },
  { folder: "a started checkout", daemonSays: "stopped", started: false, make: (root: string) => startedApp(root) },
  {
    folder: "a repository never started", daemonSays: "started", started: true,
    make: (root: string) => { startedApp(root); return gitRepository(join(root, "never")); },
  },
];

/**
 * Folders of which the database alone cannot show that the daemon reads them stopped, each with the daemon's answer the
 * hook then takes. The daemon's answer differs from the database's, so the hook's answer shows which one gave it.
 */
const foldersTheDaemonDecides = [
  { folder: "a started checkout", daemonSays: "stopped", started: false, make: (root: string) => startedApp(root) },
  {
    folder: "a repository never started", daemonSays: "started", started: true,
    make: (root: string) => { startedApp(root); return gitRepository(join(root, "never")); },
  },
  {
    folder: "a new clone of a started repository", daemonSays: "stopped", started: false,
    make: (root: string) => { startedApp(root); return newClone(join(root, "clone")); },
  },
  {
    folder: "a folder outside Git that no record names", daemonSays: "started", started: true,
    make: (root: string) => {
      startedApp(root);
      const plain = join(root, "plain");
      mkdirSync(plain);
      return plain;
    },
  },
  {
    folder: "a linked worktree no record holds, of a repository stopped as a whole", daemonSays: "started", started: true,
    make: (root: string) => {
      const app = gitRepository(join(root, "app"));
      startTestRepository(app);
      stopRerail(app, silently, { repository: true });
      git(app, "worktree", "add", "-q", join(root, "wt"));
      return join(root, "wt");
    },
  },
];

const corruptDatabase = (): Error => new Database.SqliteError("database disk image is malformed", "SQLITE_CORRUPT");

/**
 * Failures of the hook's own read of the start state: a database SQLite cannot read, which a live daemon with its own
 * connection answers past, and a fault of the read itself, which is the hook's.
 */
const readFailures = [
  { what: "SQLite cannot read the database", error: corruptDatabase(), asked: true, outcome: "asks the daemon" },
  {
    what: "a Drizzle query fails on SQLite's error", error: new DrizzleQueryError("select 1", [], corruptDatabase()), asked: true,
    outcome: "asks the daemon",
  },
  { what: "the read itself fails", error: new TypeError("Cannot read properties of undefined"), asked: false, outcome: "throws and ends its request to the daemon" },
];

/** `change` over the Rerail database, open for writing. */
function inDatabase<T>(change: (database: ReturnType<typeof openRerailDatabase>) => T): T {
  const database = openRerailDatabase(defaultRerailDatabasePath());
  try {
    return change(database);
  } finally {
    database.$client.close();
  }
}

/** The device and file id of the database, its WAL and the WAL's index, or `missing` for one not there. */
function databaseFileIds(): string {
  return ["", "-wal", "-shm"].map(suffix => {
    const found = lstatSync(`${defaultRerailDatabasePath()}${suffix}`, { bigint: true, throwIfNoEntry: false });
    return found === undefined ? "missing" : `${found.dev}:${found.ino}`;
  }).join(" ");
}

/**
 * Marks the newest schema step the Rerail database records as not run, as in a file an older build wrote last, and
 * gives its id: a read-only open by this build, which reads the file in this build's shape, runs it first over a
 * writable connection.
 */
function newestStepNotRun(): string {
  const database = new Database(defaultRerailDatabasePath());
  try {
    const { id } = database.prepare("SELECT id FROM schema_migrations ORDER BY rowid DESC LIMIT 1").get() as { id: string };
    database.prepare("DELETE FROM schema_migrations WHERE id = ?").run(id);
    return id;
  } finally {
    database.close();
  }
}

/** Whether the Rerail database records the schema step `id` as run. */
function stepRun(id: string): boolean {
  const database = new Database(defaultRerailDatabasePath(), { readonly: true });
  try {
    return database.prepare("SELECT 1 FROM schema_migrations WHERE id = ?").get(id) !== undefined;
  } finally {
    database.close();
  }
}

/** What the daemon answers for `folder` (`startStateReader`, as its presentation store reads it per request). */
function daemonReads(folder: string): string {
  const database = openRerailDatabase(defaultRerailDatabasePath(), { readonly: true });
  try {
    return startStateReader(database)(folder);
  } finally {
    database.$client.close();
  }
}

/** A link at `path` to the folder `target`: a junction on Windows, which needs no privilege there. */
function folderLink(target: string, path: string): void {
  symlinkSync(target, path, process.platform === "win32" ? "junction" : "dir");
}

/**
 * A Git folder at `<app>/<name>/.git` Git passes over, as it lacks `part` (its HEAD, a HEAD that names a ref as Git
 * reads one, `objects` or `refs`), holding the clone id of `stopped`, a repository stopped as a whole: Git looks further
 * up and finds `app`.
 */
function passedOverGitFolder(app: string, name: string, part: "HEAD" | "ref" | "objects" | "refs", stopped: string): string {
  const folder = join(app, name);
  const gitFolder = join(folder, ".git");
  mkdirSync(gitFolder, { recursive: true });
  if (part !== "HEAD") writeFileSync(join(gitFolder, "HEAD"), part === "ref" ? "refs/heads/main\n" : "ref: refs/heads/main\n");
  for (const kept of ["objects", "refs"]) if (kept !== part) mkdirSync(join(gitFolder, kept));
  copyFileSync(join(stopped, ".git", "rerail-repository-id"), join(gitFolder, "rerail-repository-id"));
  return folder;
}

type RefusedShape = "objects-file" | "head-link" | "objects-unsearchable";

/**
 * Shapes of a whole `.git` folder that Git refuses all the same on POSIX: `objects` a plain file with no execute bit, an
 * `objects` folder with no search bit (Git asks `access` with `X_OK` of both), and a HEAD that is a link outside
 * `refs/`. On Windows, where `X_OK` means existence, Git takes the first, and the other two cannot be made there.
 */
const REFUSED_SHAPES: readonly RefusedShape[] = process.platform === "win32" ? ["objects-file"] : ["objects-file", "head-link", "objects-unsearchable"];

/** A Git folder at `<app>/<shape>/.git`, refused by Git as `shape` says, holding `stopped`'s clone id: Git finds `app`. */
function refusedGitFolder(app: string, shape: RefusedShape, stopped: string): string {
  const folder = passedOverGitFolder(app, shape, shape === "head-link" ? "HEAD" : "objects", stopped);
  const gitFolder = join(folder, ".git");
  if (shape === "head-link") symlinkSync(join("..", "not-refs", "main"), join(gitFolder, "HEAD"));
  if (shape === "objects-file") writeFileSync(join(gitFolder, "objects"), "");
  if (shape === "objects-unsearchable") {
    mkdirSync(join(gitFolder, "objects"));
    chmodSync(join(gitFolder, "objects"), 0o600);
  }
  return folder;
}

/**
 * A clone of `startedApp`'s repository, recorded, then moved aside to `<root>/moved-clone` while a fresh clone is
 * recorded in its place, which takes over its row: it keeps a clone id no row holds, so only its remote, which Git
 * reads, names its repository.
 */
function movedAsideClone(root: string): string {
  const place = join(root, "clone-place");
  inDatabase(database => recordCheckout(database, newClone(place)));
  const moved = join(root, "moved-clone");
  renameSync(place, moved);
  inDatabase(database => recordCheckout(database, newClone(place)));
  return moved;
}

/**
 * G, stopped as a whole, with two linked worktrees beside it: `g-s` started by itself, and `g-w`, whose own
 * `config.worktree` puts its checkout at `g-s`. Returns `g-w`, which Git reads as `g-s`.
 */
function worktreeConfigToStarted(root: string): string {
  const g = gitRepository(join(root, "g"));
  startTestRepository(g);
  for (const name of ["g-w", "g-s"]) git(g, "worktree", "add", "-q", join(root, name));
  stopRerail(g, silently, { repository: true });
  inDatabase(database => setCheckoutStarted(database, recordCheckout(database, join(root, "g-s")), true));
  git(g, "config", "extensions.worktreeConfig", "true");
  git(join(root, "g-w"), "config", "--worktree", "core.worktree", realpathSync.native(join(root, "g-s")));
  return join(root, "g-w");
}

const silently = { lane: "terminal" as const, say: (): void => undefined };

/**
 * Checkouts as people keep them, and the odd shapes Git's search meets, under `root`, each folder named. A is started as
 * a whole; B started, then stopped in its checkout (`rerail stop`); C and E stopped as a whole; D and F are clones nested
 * in B and A; G is stopped as a whole, one of its worktrees started; a repository is never started, and two folders are
 * outside Git.
 */
function generatedTrees(root: string): Array<[string, string]> {
  // A: folders, a linked worktree inside it and one beside it, a clone of A moved aside after it was recorded, and F, a
  // clone inside A stopped in its checkout.
  const a = startedApp(root);
  const moved = movedAsideClone(root);
  mkdirSync(join(a, "src"));
  git(a, "worktree", "add", "-q", join(a, ".wt", "inner"));
  git(a, "worktree", "add", "-q", join(root, "a-beside"));
  const f = gitRepository(join(a, "nested-stopped"));
  startTestRepository(f);
  stopTestRepository(f);
  mkdirSync(join(f, "src"));
  // B: worktrees of its own inside it and beside it, one stopped and one started by themselves, then B stopped; inside
  // it, a new clone of A, a worktree of A, and D, a clone started.
  const b = gitRepository(join(root, "b"));
  startTestRepository(b);
  mkdirSync(join(b, "src", "deep"), { recursive: true });
  git(b, "worktree", "add", "-q", join(b, ".wt", "inner"));
  for (const name of ["b-beside", "b-beside-stopped", "b-own-started"]) git(b, "worktree", "add", "-q", join(root, name));
  stopTestRepository(join(root, "b-beside-stopped"));
  inDatabase(database => setCheckoutStarted(database, recordCheckout(database, join(root, "b-own-started")), true));
  stopTestRepository(b);
  newClone(join(b, "fresh-clone"));
  git(a, "worktree", "add", "-q", join(b, "a-worktree"));
  const d = gitRepository(join(b, "nested-started"));
  startTestRepository(d);
  mkdirSync(join(d, "src"));
  // C, with a worktree beside it, and E, both stopped as a whole; E's config then puts its checkout at a worktree of B
  // started by itself.
  const c = gitRepository(join(root, "c"));
  startTestRepository(c);
  mkdirSync(join(c, "src"));
  git(c, "worktree", "add", "-q", join(root, "c-beside"));
  stopRerail(c, silently, { repository: true });
  const e = gitRepository(join(root, "e"));
  startTestRepository(e);
  stopRerail(e, silently, { repository: true });
  mkdirSync(join(e, "src"));
  git(e, "config", "core.worktree", realpathSync.native(join(root, "b-own-started")));
  // G, whose worktree's own config puts its checkout at a worktree started by itself.
  const g = worktreeConfigToStarted(root);
  // Inside A, Git folders Git passes over or refuses that hold C's clone id. Inside B, a bare repository whose config
  // puts its checkout at A, and a folder recorded as outside Git (recorded while a ceiling hid B), then started.
  const passedOver = [
    ...(["HEAD", "ref", "objects", "refs"] as const).map(part => passedOverGitFolder(a, `no-${part}`, part, c)),
    ...REFUSED_SHAPES.map(shape => refusedGitFolder(a, shape, c)),
  ];
  git(b, "init", "-q", "--bare", "mirror.git");
  git(join(b, "mirror.git"), "config", "core.bare", "false");
  git(join(b, "mirror.git"), "config", "core.worktree", realpathSync.native(a));
  const own = join(b, "own-folder");
  mkdirSync(own);
  vi.stubEnv("GIT_CEILING_DIRECTORIES", realpathSync.native(b));
  const ownRepository = inDatabase(database => recordCheckout(database, own).repositoryKey);
  vi.stubEnv("GIT_CEILING_DIRECTORIES", undefined);
  inDatabase(database => setRepositoryStarted(database, ownRepository, true));
  // A repository never started, folders outside Git, and links from each of A and B into the other.
  mkdirSync(join(gitRepository(join(root, "never")), "src"));
  mkdirSync(join(root, "plain"));
  mkdirSync(join(root, "plain-started", "sub"), { recursive: true });
  startTestRepository(join(root, "plain-started"));
  folderLink(join(b, "src"), join(a, "link-into-b"));
  folderLink(join(a, "src"), join(b, "link-into-a"));
  const folders: Array<[string, string]> = [
    ["app", a], ["app/src", join(a, "src")], ["app/.wt/inner", join(a, ".wt", "inner")], ["a-beside", join(root, "a-beside")],
    ["app/nested-stopped", f], ["app/nested-stopped/src", join(f, "src")], ["app/link-into-b", join(a, "link-into-b")],
    ...passedOver.map((folder): [string, string] => [`app/${basename(folder)}`, folder]),
    ["b", b], ["b/src", join(b, "src")], ["b/src/deep", join(b, "src", "deep")], ["b/.wt/inner", join(b, ".wt", "inner")],
    ["b-beside", join(root, "b-beside")], ["b-beside-stopped", join(root, "b-beside-stopped")], ["b-own-started", join(root, "b-own-started")],
    ["b/fresh-clone", join(b, "fresh-clone")], ["b/a-worktree", join(b, "a-worktree")], ["b/nested-started", d],
    ["b/nested-started/src", join(d, "src")], ["b/link-into-a", join(b, "link-into-a")], ["b/mirror.git/objects", join(b, "mirror.git", "objects")],
    ["b/own-folder", own], ["c", c], ["c/src", join(c, "src")], ["c-beside", join(root, "c-beside")], ["e", e], ["e/src", join(e, "src")],
    ["never", join(root, "never")], ["never/src", join(root, "never", "src")], ["plain", join(root, "plain")],
    ["plain-started", join(root, "plain-started")], ["plain-started/sub", join(root, "plain-started", "sub")],
    ["moved-clone", moved], ["g-w", g],
  ];
  // On Windows a folder named in another letter case is the same folder.
  if (process.platform === "win32") folders.push(["B/SRC", join(root.toUpperCase(), "B", "SRC")], ["APP", join(root.toUpperCase(), "APP")]);
  return folders;
}

/** A folder a person reaches under another letter case than the disk stores, and where in it a hook runs. */
interface MiscasedFolder {
  readonly name: string;
  /** The checkout's top folder, spelled as typed and as stored. */
  readonly typed: string;
  readonly stored: string;
  /** Where the hook runs, as typed and as stored. */
  readonly workspace: string;
  readonly storedWorkspace: string;
}

/**
 * Under `root`, each started as stored: `Notes`, outside Git, typed `notes`; the `src` folder of a Git checkout `Repo`,
 * typed `repo/src`; and `Wt`, a linked worktree of `Base`, typed `wt`. Where the filesystem tells letter case apart the
 * typed name is a link to the stored one, which `Spelling` has the realpaths read as typed.
 */
function miscasedFolders(root: string): MiscasedFolder[] {
  mkdirSync(join(root, "Notes"));
  mkdirSync(join(gitRepository(join(root, "Repo")), "src"));
  git(gitRepository(join(root, "Base")), "worktree", "add", "-q", join(root, "Wt"));
  const folders = [["outside Git", "Notes", []], ["Git checkout", "Repo", ["src"]], ["linked worktree", "Wt", []]] as const;
  return folders.map(([name, folder, inside]) => {
    const stored = join(root, folder);
    const typed = join(root, folder.toLowerCase());
    // A case-insensitive filesystem (macOS's, Windows') already finds the folder under its typed name.
    if (!existsSync(typed)) folderLink(stored, typed);
    startTestRepository(stored);
    return { name, typed, stored, workspace: join(typed, ...inside), storedWorkspace: join(stored, ...inside) };
  });
}

/**
 * Each way the rows of a checkout typed in another case can stand: the stored spelling's own on/off (null: none), a
 * row under the typed spelling with its own (none where undefined), and its repository's on/off.
 */
const SPELLING_ROWS = [true, false, null].flatMap(stored =>
  [true, false, undefined].flatMap(typed => [true, false].map(repository => ({ stored, typed, repository }))));

/** Sets the rows of `folder`'s checkout and its repository as `rows` says. */
function plantRows(folder: MiscasedFolder, rows: (typeof SPELLING_ROWS)[number]): void {
  inDatabase(({ $client: db }) => {
    const row = db.prepare("SELECT repository_key AS key, clone_id AS cloneId FROM checkouts WHERE checkout_path = ?").get(folder.stored) as
      { key: string; cloneId: string };
    db.prepare("UPDATE repositories SET started = ? WHERE repository_key = ?").run(Number(rows.repository), row.key);
    db.prepare("UPDATE checkouts SET started = ? WHERE checkout_path = ?").run(rows.stored === null ? null : Number(rows.stored), folder.stored);
    db.prepare("DELETE FROM checkouts WHERE checkout_path = ?").run(folder.typed);
    if (rows.typed === undefined) return;
    db.prepare("INSERT INTO checkouts (checkout_path, repository_key, clone_id, started, created_at, last_seen_at) VALUES (?, ?, ?, ?, 1, 1)")
      .run(folder.typed, row.key, row.cloneId, Number(rows.typed));
  });
}

/**
 * The hook's start check in each of `folders`, typed in another case, under each of `SPELLING_ROWS`, beside a daemon
 * that answers late as a live one reads the folder it was asked about: where the hook answered stopped itself though the
 * daemon reads otherwise or took another answer (`wrong`), where it waited though the daemon reads stopped, and the
 * folders it answered itself in.
 */
async function startChecksTypedOtherwise(host: StartCheckHost, daemon: CountingDaemon, folders: readonly MiscasedFolder[]): Promise<{
  wrong: string[]; askedThoughStopped: string[]; answeredItselfIn: string[];
}> {
  const found: TypedChecksFound = { wrong: [], askedThoughStopped: [], answeredItselfIn: new Set<string>() };
  for (const folder of folders) {
    for (const rows of SPELLING_ROWS) recordTypedCheck(found, folder, await startCheckTypedOtherwise(host, daemon, folder, rows));
  }
  return { ...found, answeredItselfIn: [...found.answeredItselfIn] };
}

/** What `startChecksTypedOtherwise` has found so far. */
interface TypedChecksFound {
  readonly wrong: string[];
  readonly askedThoughStopped: string[];
  readonly answeredItselfIn: Set<string>;
}

/** One start check in a folder typed in another case: its name, with what the daemon reads, and the hook's answer. */
interface TypedCheck {
  readonly name: string;
  readonly reads: string;
  readonly started: boolean;
  readonly answeredItself: boolean;
}

/**
 * The hook's start check in `folder` under `rows`, and what `daemon` reads of the folder this check asked it about.
 * The daemon's last request is this check's only when the check sent it exactly one. Where the hook gave up before its
 * request reached the daemon, the last request is an earlier check's, or none, so the check fails here rather than
 * judge the hook's answer by another folder.
 */
async function startCheckTypedOtherwise(
  host: StartCheckHost, daemon: CountingDaemon, folder: MiscasedFolder, rows: (typeof SPELLING_ROWS)[number],
): Promise<TypedCheck> {
  plantRows(folder, rows);
  const name = `${folder.name}: stored ${String(rows.stored)}, typed ${String(rows.typed)}, repository ${String(rows.repository)}`;
  const proofsBefore = host.proofs.found.length;
  const askedBefore = daemon.asked.length;
  host.proofs.nextCheck();
  const started = await host.hookRerailStarted("claude-code", folder.workspace);
  expect(daemon.asked.slice(askedBefore), `the folders the daemon was asked about by the start check in ${name}`).toHaveLength(1);
  const reads = host.daemonReads(daemon.asked.at(-1)!);
  const answeredItself = host.proofs.found.length > proofsBefore && host.proofs.found.at(-1) === true;
  return { name: `${name}: ${reads}`, reads, started, answeredItself };
}

/** Adds `check`, made in `folder`, to each of `found`'s lists it belongs to. */
function recordTypedCheck(found: TypedChecksFound, folder: MiscasedFolder, { name, reads, started, answeredItself }: TypedCheck): void {
  if (answeredItself ? reads !== "stopped" : started !== (reads !== "stopped")) found.wrong.push(name);
  if (!answeredItself && reads === "stopped") found.askedThoughStopped.push(name);
  if (answeredItself) found.answeredItselfIn.add(folder.name);
}

/**
 * The longest the hook's own check may take where it cannot read the database: it gives up at once there, where a read
 * took 1.5 to 5 s waiting on another process's lock, and about 1.4 s on Windows inside SQLite's retries of an open
 * Windows refused.
 */
const GIVEN_UP_WITHIN_MS = 200;

/**
 * The start check in `workspace`, beside `daemon`, which answers started only once the hook's own check has run: the
 * hook waits for it and takes its answer, well before its read of the daemon would give up, and its check gave up
 * within `GIVEN_UP_WITHIN_MS`.
 */
async function startedFromLateDaemon(host: StartCheckHost, daemon: CountingDaemon, workspace: string): Promise<void> {
  host.proofs.nextCheck();
  const started = await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", workspace));
  expect({ started, checked: host.proofs.found.at(-1), requests: daemon.requests, ended: daemon.ended() })
    .toEqual({ started: true, checked: false, requests: ["/api/presentation"], ended: 0 });
  expect(host.proofs.took.at(-1)).toBeLessThan(GIVEN_UP_WITHIN_MS);
}

/** Another process holding the Rerail database, until `end` ends it. */
interface DatabaseHolder {
  readonly pid?: number | undefined;
  end(): Promise<void>;
}

/** Settles once `child` has printed `line`; rejects with what it printed when it exits first or 30 s pass. */
async function printed(child: ChildProcess, line: string): Promise<void> {
  let output = "";
  let timer: NodeJS.Timeout | undefined;
  try {
    await new Promise<void>((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(`no "${line}" within 30 s: ${output}`)), 30_000);
      child.stdout?.on("data", (chunk: Buffer) => {
        output += String(chunk);
        if (output.includes(line)) resolve();
      });
      child.stderr?.on("data", (chunk: Buffer) => {
        output += String(chunk);
      });
      child.once("exit", code => reject(new Error(`exited ${String(code)} before "${line}": ${output}`)));
    });
  } finally {
    clearTimeout(timer);
  }
}

/** `command` with `args` once it prints `held`; `end` ends it and waits until it has exited. */
async function holdingProcess(command: string, args: readonly string[]): Promise<DatabaseHolder> {
  const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
  const exited = new Promise<void>(resolve => child.once("exit", () => resolve()));
  const end = async (): Promise<void> => {
    if (child.exitCode === null && child.signalCode === null) child.kill();
    await exited;
  };
  try {
    await printed(child, "held");
  } catch (error) {
    await end();
    throw error;
  }
  return { pid: child.pid, end };
}

/**
 * Another process holding the Rerail database locked against readers, as a writer that keeps it does: SQLite's
 * exclusive locking mode with a write open, until it ends or, with `releaseMs`, that long after it took the lock.
 */
async function lockedByAnotherProcess(releaseMs?: number): Promise<DatabaseHolder> {
  const script = [
    "const Database = require(require.resolve('better-sqlite3', { paths: [process.argv[1]] }));",
    "const database = new Database(process.argv[2]);",
    "database.pragma('locking_mode = EXCLUSIVE');",
    "database.exec('BEGIN EXCLUSIVE');",
    "database.exec('UPDATE repositories SET started = started');",
    "process.stdout.write('held\\n');",
    "if (process.argv[3] !== '') setTimeout(() => process.exit(0), Number(process.argv[3]));",
    "setInterval(() => undefined, 1 << 30);",
  ].join("\n");
  return await holdingProcess(process.execPath, ["-e", script, process.cwd(), defaultRerailDatabasePath(), releaseMs === undefined ? "" : String(releaseMs)]);
}

/** A Windows system tool by its full path: a shell's own tools (Git Bash's `whoami`) may come first on PATH. */
function windowsTool(...path: string[]): string {
  return join(process.env.SystemRoot ?? String.raw`C:\Windows`, "System32", ...path);
}

/** Another program holding the Rerail database file open with no sharing, as a backup or a copy can. */
async function heldWithoutSharing(): Promise<DatabaseHolder> {
  return await heldByAnotherProgram(defaultRerailDatabasePath(), "ReadWrite", "None");
}

/**
 * Another program holding the file at `path` open for `access`, sharing only what `share` names (.NET's `FileShare`):
 * while it shares no deleting, Windows refuses to delete the file or to rename another over it.
 */
async function heldByAnotherProgram(path: string, access: "Read" | "ReadWrite", share: "None" | "Read"): Promise<DatabaseHolder> {
  const command = `$f = [System.IO.File]::Open('${path.replaceAll("'", "''")}', 'Open', '${access}', '${share}'); `
    + "[Console]::Out.WriteLine('held'); [Console]::Out.Flush(); Start-Sleep -Seconds 600";
  return await holdingProcess(windowsTool("WindowsPowerShell", "v1.0", "powershell.exe"), ["-NoProfile", "-NonInteractive", "-Command", command]);
}

/**
 * Another program locking `length` bytes of the file at `path` from `offset` (.NET's `FileStream.Lock`), as a backup or a
 * scanner can: Windows then refuses every other read of those bytes, and SQLite retries such a read about 1.4 s.
 */
async function bytesLockedByAnotherProgram(path: string, offset: number, length: number): Promise<DatabaseHolder> {
  const command = `$f = [System.IO.File]::Open('${path.replaceAll("'", "''")}', 'Open', 'ReadWrite', 'ReadWrite'); $f.Lock(${offset}, ${length}); `
    + "[Console]::Out.WriteLine('held'); [Console]::Out.Flush(); Start-Sleep -Seconds 600";
  return await holdingProcess(windowsTool("WindowsPowerShell", "v1.0", "powershell.exe"), ["-NoProfile", "-NonInteractive", "-Command", command]);
}

/** The first GiB of a file: up to the bytes SQLite locks in the database file itself, which another program cannot lock while SQLite holds them. */
const BELOW_SQLITE_LOCKS = 1_073_741_824;

/**
 * Byte ranges of the Rerail database's files another program can lock, each of which SQLite meets when the hook's check
 * reads the checkout's row from the WAL, as it is after a write of the daemon's.
 */
const lockedRanges = [
  { what: "the whole database file", suffix: "", offset: 0, length: BELOW_SQLITE_LOCKS },
  { what: "the database's header", suffix: "", offset: 0, length: 100 },
  { what: "the database's first page past its header", suffix: "", offset: 100, length: 3_996 },
  { what: "the whole WAL", suffix: "-wal", offset: 0, length: BELOW_SQLITE_LOCKS },
];

/** A file of the Rerail database, the database itself unless `path` names another, denied to this user by an NTFS deny entry, which `end` takes off. */
function deniedToThisUser(path = defaultRerailDatabasePath()): Promise<DatabaseHolder> {
  const user = execFileSync(windowsTool("whoami.exe"), ["/user", "/fo", "csv", "/nh"], { encoding: "utf8" }).trim().split(",").at(-1)!.replaceAll('"', "");
  execFileSync(windowsTool("icacls.exe"), [path, "/deny", `*${user}:F`], { stdio: "ignore" });
  const end = (): Promise<void> => {
    execFileSync(windowsTool("icacls.exe"), [path, "/remove:d", `*${user}`], { stdio: "ignore" });
    return Promise.resolve();
  };
  return Promise.resolve({ end });
}

/**
 * Whether this process's Node opens `path` though a deny entry refuses it to this user: an enabled Backup privilege
 * (an elevated SSH session holds it so) lets Node's opens, which all use backup semantics, pass the entry SQLite's own
 * open meets.
 */
function nodeOpensDespiteDeny(path: string): boolean {
  try {
    closeSync(openSync(path, "r"));
    return true;
  } catch {
    return false;
  }
}

/** `step`, tried again while Windows refuses it for a moment (a scanner or the search indexer holding a folder). */
function retried<T>(step: () => T): T {
  for (let attempt = 1; ; attempt += 1) {
    try {
      return step();
    } catch (error) {
      if (attempt >= 50) throw error;
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 20);
    }
  }
}

/** How a folder is away: moved aside, or moved aside with a file left at its path. */
type Away = "moved" | "a file in its place";

/** `folder` moved away as `away` says, until {@link backFromAway}. */
function goAway(folder: string, away: Away): void {
  retried(() => renameSync(folder, `${folder}.aside`));
  if (away === "a file in its place") writeFileSync(folder, "");
}

function backFromAway(folder: string, away: Away): void {
  if (away === "a file in its place") retried(() => rmSync(folder));
  retried(() => renameSync(`${folder}.aside`, folder));
}

/** The error code of `error`, or its name. */
function codeOf(error: unknown): string {
  return error instanceof Error && "code" in error ? String(error.code) : String(error);
}

/** Ways a checkout's folder is away for a moment while the hook checks the answer it kept there. */
const checkoutPathsGoneAWhile = [
  { what: "the checkout's folder is moved away", away: "moved" as const, folderOf: (top: string) => top },
  { what: "the checkout's parent folder is replaced by a file", away: "a file in its place" as const, folderOf: (top: string) => dirname(top) },
];

/**
 * `checkout-control.ts` with `step` run as soon as the hook's check of the answer it kept has the real path of the folder
 * (`storedPath`, which only that check calls in a start check) in the start check numbered `check`, and `undo` on the
 * next turn of the event loop, once the check is over: a folder moved away and back, or a `.git` folder turning into a
 * `gitdir:` file as `git init --separate-git-dir` and `git submodule absorbgitdirs` leave a checkout for that moment.
 * `moves` gets the folder each step ran for.
 */
function onCheck(check: number, moves: string[], step: () => void, undo: () => void): void {
  let checks = 0;
  vi.doMock("../../src/engine/catalog/checkout-control.js", async () => {
    const actual = await vi.importActual<typeof import("../../src/engine/catalog/checkout-control.js")>("../../src/engine/catalog/checkout-control.js");
    return {
      ...actual,
      storedPath: (path: string) => {
        const stored = actual.storedPath(path);
        checks += 1;
        if (checks !== check) return stored;
        moves.push(path);
        step();
        setImmediate(undo);
        return stored;
      },
    };
  });
}

/**
 * Another process holding the Rerail database as the daemon's writer does, with `started` committed for every
 * repository only in the WAL: it checkpoints no more, and the database file itself says stopped.
 */
async function startedOnlyInTheWal(): Promise<DatabaseHolder> {
  const script = [
    "const Database = require(require.resolve('better-sqlite3', { paths: [process.argv[1]] }));",
    "const database = new Database(process.argv[2]);",
    "database.pragma('wal_autocheckpoint = 0');",
    "database.exec('UPDATE repositories SET started = 0');",
    "database.pragma('wal_checkpoint(TRUNCATE)');",
    "database.exec('UPDATE repositories SET started = 1');",
    "process.stdout.write('held\\n');",
    "setInterval(() => undefined, 1 << 30);",
  ].join("\n");
  return await holdingProcess(process.execPath, ["-e", script, process.cwd(), defaultRerailDatabasePath()]);
}

/**
 * The WAL and the WAL's index a reader left beside the database when it closed: another process can then take the file
 * alone, which it cannot while a daemon's connection holds it.
 */
function companionsLeftByAReader(): void {
  heldOpen(defaultRerailDatabasePath())?.close();
}

/** Another process holding the Rerail database open, as the daemon's connection does once it has read it. */
async function heldByAnotherProcess(): Promise<DatabaseHolder> {
  const script = [
    "const Database = require(require.resolve('better-sqlite3', { paths: [process.argv[1]] }));",
    "const database = new Database(process.argv[2], { readonly: true });",
    "database.prepare('SELECT count(*) FROM sqlite_master').get();",
    "process.stdout.write('held\\n');",
    "setInterval(() => undefined, 1 << 30);",
  ].join("\n");
  return await holdingProcess(process.execPath, ["-e", script, process.cwd(), defaultRerailDatabasePath()]);
}

/** {@link cuedWriter}'s process: it runs the first step at once and one more on each line it reads. */
const CUED_WRITER = [
  "const Database = require(require.resolve('better-sqlite3', { paths: [process.argv[1]] }));",
  "const database = new Database(process.argv[2]);",
  "database.pragma('wal_autocheckpoint = 0');",
  "const steps = JSON.parse(process.argv[3]);",
  "let next = 0;",
  "const run = () => { database.exec(steps[next]); next += 1; process.stdout.write(`step ${next} ran\\n`); };",
  "process.stdin.on('data', run);",
  "run();",
  "process.stdout.write('held\\n');",
].join("\n");

/** A {@link DatabaseHolder} that runs the next of its steps on `cue`, settling once it has. */
interface CuedWriter extends DatabaseHolder {
  cue(): Promise<void>;
}

/**
 * Another process holding the Rerail database as the daemon's writer does, its own checkpoints off, which runs the first
 * of `steps` (SQL) at once and each later one on a cue: a test changes a file of the database under it between two.
 */
async function cuedWriter(steps: readonly string[]): Promise<CuedWriter> {
  const child = spawn(process.execPath, ["-e", CUED_WRITER, process.cwd(), defaultRerailDatabasePath(), JSON.stringify(steps)], {
    stdio: ["pipe", "pipe", "pipe"], windowsHide: true,
  });
  child.stdin.on("error", () => undefined);
  const exited = new Promise<void>(resolve => child.once("exit", () => resolve()));
  const end = async (): Promise<void> => {
    if (child.exitCode === null && child.signalCode === null) child.kill();
    await exited;
  };
  let ran = 1;
  const cue = async (): Promise<void> => {
    ran += 1;
    const done = printed(child, `step ${ran} ran`);
    child.stdin.write("\n");
    await done;
  };
  try {
    await printed(child, "held");
  } catch (error) {
    await end();
    throw error;
  }
  return { pid: child.pid, end, cue };
}

/**
 * The WAL written into the database file and emptied by a writable connection of its own, as the daemon's checkpoints
 * do: a connection held open elsewhere keeps the WAL and its index from being removed when this one closes.
 */
function checkpointed(): void {
  inDatabase(database => database.$client.pragma("wal_checkpoint(TRUNCATE)"));
}

/**
 * The database moved aside to `<name>.live` under the connections that hold it, and the copy `backup` put at its name,
 * as a restore that skips `rerail daemon stop` leaves it: the WAL and its index stay where they are.
 */
function movedAsideAndReplaced(backup: string): void {
  const path = defaultRerailDatabasePath();
  renameSync(path, `${path}.live`);
  copyFileSync(backup, path);
}

/**
 * A start check in `workspace` beside a daemon that answers stopped in time, named by `options`: the hook takes its
 * answer and keeps none there.
 */
async function notKept(host: StartCheckHost, workspace: string, options: StandInOptions = {}): Promise<void> {
  const daemon = await countingDaemon(process.env.RERAIL_HOME!, "stopped", undefined, { scopeOf: host.daemonScope, ...options });
  try {
    const started = await host.hookRerailStarted("claude-code", workspace, undefined, GRACE_NO_ANSWER_MISSES);
    expect({ started, kept: existsSync(keptAnswerFile(workspace, options)) }).toEqual({ started: false, kept: false });
  } finally {
    await daemon.close();
  }
}

/** A Windows server address that never answers (TEST-NET-1), one per call: Windows remembers a server it gave up on. */
function silentServer(): string {
  return `192.0.2.${String(2 + Math.floor(Math.random() * 250))}`;
}

/**
 * A grace for the start check that no answer of the daemon's misses, nor the empty read where none is recorded: a test
 * of what the hook does when the daemon answers in time gives it, so a slow or busy machine cannot make that answer late.
 */
const GRACE_NO_ANSWER_MISSES = 60_000;

/** The Git variables the test setup sets (`setup-rerail-home.ts`); a shell's others reach no test of the start check. */
const TEST_SETUP_GIT_VARIABLES = new Set(["GIT_CONFIG_GLOBAL", "GIT_CONFIG_NOSYSTEM"]);

/**
 * Changes after the daemon answered stopped in `<app>/src` (`app` a checkout stopped in its own record) that the hook
 * sees, each of which leaves it no answer to repeat there: it waits for the late daemon and takes its answer. A daemon
 * named by another start time (`startedAt`) is another daemon, which places every folder again. With `held`, a
 * connection holds the database open from before the answer is kept until the hook has run, as the daemon does, so its
 * WAL and the WAL's index keep their file ids through the change.
 */
const changesSinceTheAnswer: Array<{ what: string; change: (app: string) => void; startedAt?: number; held?: true }> = [
  { what: "the answer kept does not parse", change: app => writeFileSync(keptAnswerFile(join(app, "src")), "{") },
  {
    // Whole and right in every field but its size: a file of many megabytes there is never read whole.
    what: "the answer kept grows past 16 KiB", change: app => {
      const file = keptAnswerFile(join(app, "src"));
      writeFileSync(file, JSON.stringify({ ...JSON.parse(readFileSync(file, "utf8")) as object, padding: "x".repeat(16_384) }));
    },
  },
  { what: "the answer kept is older than 30 minutes", change: app => keptAt(join(app, "src"), Date.now() - 30 * 60_000 - 1_000) },
  { what: "the answer kept is dated after now, as once the clock is set back", change: app => keptAt(join(app, "src"), Date.now() + 60_000) },
  {
    what: "the folder is made again", change: app => {
      retried(() => renameSync(join(app, "src"), join(app, "src-before")));
      mkdirSync(join(app, "src"));
    },
  },
  {
    what: "the checkout's .git is made again", change: app => {
      retried(() => renameSync(join(app, ".git"), join(app, ".git-before")));
      cpSync(join(app, ".git-before"), join(app, ".git"), { recursive: true });
    },
  },
  {
    // Only the top's own file id tells this apart: the folder and the `.git` keep theirs, and the path is the same.
    what: "the checkout's top is made again, its .git and the folder moved into it", change: app => {
      retried(() => renameSync(app, `${app}-before`));
      mkdirSync(app);
      for (const name of ["src", ".git"]) retried(() => renameSync(join(`${app}-before`, name), join(app, name)));
    },
  },
  { what: "a .git is made in the folder", change: app => git(join(app, "src"), "init", "-q") },
  { what: "the checkout is started", change: app => inDatabase(database => setCheckoutStarted(database, recordCheckout(database, app), true)) },
  {
    // The database says stopped again, as it did, and its files keep their ids (`held`): only the start mark tells the
    // change, which a daemon may have read.
    what: "the checkout is started and stopped again", held: true, change: app => inDatabase(database => {
      setCheckoutStarted(database, recordCheckout(database, app), true);
      setCheckoutStarted(database, recordCheckout(database, app), false);
    }),
  },
  { what: "a renewal of the start mark could not replace it", change: () => writeFileSync(join(process.env.RERAIL_HOME!, "stopped-answers", STUCK_MARK), "") },
  {
    what: "the checkout's repository takes the remote it gained",
    change: app => inDatabase(database => adoptRemote(database, recordCheckout(database, app).repositoryKey, "https://example.invalid/team/other.git")),
  },
  { what: "another daemon runs", change: () => undefined, startedAt: 2 },
  {
    what: "the folder of kept answers is a link to a copy of it", change: () => {
      const answers = join(process.env.RERAIL_HOME!, "stopped-answers");
      cpSync(answers, `${answers}-copy`, { recursive: true });
      retried(() => renameSync(answers, `${answers}-before`));
      folderLink(`${answers}-copy`, answers);
    },
  },
  {
    what: "this daemon's folder of kept answers is a link to a copy of it", change: app => {
      const folder = dirname(keptAnswerFile(join(app, "src")));
      cpSync(folder, `${folder}-copy`, { recursive: true });
      retried(() => renameSync(folder, `${folder}-before`));
      folderLink(`${folder}-copy`, folder);
    },
  },
];

/**
 * Writes `text` over the file at `path` in place, as an editor that keeps the file does, so it keeps its file id. Through
 * an open of the file as it is: Windows refuses to create one over a hidden file, as Git leaves a `.git` file there.
 */
function rewrittenInPlace(path: string, text: string): void {
  const descriptor = openSync(path, "r+");
  try {
    ftruncateSync(descriptor, 0);
    writeSync(descriptor, text, 0);
  } finally {
    closeSync(descriptor);
  }
}

/**
 * The file `source` written over the file at `path` in place, as `cp`, `copy /Y` or a file manager's replace writes a
 * copy over a file that is there, so `path` keeps its file id. Through an open of the file as it is, which the
 * connections that hold a database do not refuse.
 */
function copiedInPlace(source: string, path: string): void {
  const descriptor = openSync(path, "r+");
  try {
    ftruncateSync(descriptor, 0);
    writeSync(descriptor, readFileSync(source), 0);
  } finally {
    closeSync(descriptor);
  }
}

/** The clone id file of the Git checkout `checkout`, whose `.git` is a folder (`repositoryIdentityLocation`). */
function cloneIdFile(checkout: string): string {
  return join(checkout, ".git", "rerail-repository-id");
}

/**
 * Mirrors the folder `source` over `dest` in place, as `rsync -a --delete` or a sync tool does: folders stay, each file is
 * written beside its name and renamed over it, and what `source` lacks is removed. Git's read-only files are made
 * writable first on Windows, which refuses a rename over a read-only file.
 */
function mirrorOver(source: string, dest: string): void {
  for (const name of readdirSync(dest).filter(each => !existsSync(join(source, each)))) {
    const path = join(dest, name);
    if (lstatSync(path).isDirectory()) removeFolderSync(path);
    else retried(() => rmSync(path, { force: true }));
  }
  for (const entry of readdirSync(source, { withFileTypes: true })) {
    const [from, to] = [join(source, entry.name), join(dest, entry.name)];
    if (entry.isDirectory()) {
      mkdirSync(to, { recursive: true });
      mirrorOver(from, to);
      continue;
    }
    copyFileSync(from, `${to}.sync~`);
    if (process.platform === "win32" && existsSync(to)) chmodSync(to, 0o666);
    retried(() => renameSync(`${to}.sync~`, to));
  }
}

/**
 * Changes after the daemon answered stopped in a folder of `app`, a repository stopped as a whole, that keep the folder,
 * the checkout's top and its `.git` the same on disk but have the daemon read the checkout's clone id as `started`'s, a
 * started repository with a `src` folder: it then places the checkout in that repository by its clone id and answers
 * that Rerail runs there (`not-set-up`: started, not set up), since `lookupAt` reads the clone id file again on every
 * answer and the checkout has no on/off of its own. `arrange` names the folder the hooks run in and gives the change.
 */
const cloneIdsChanged: Array<{ what: string; arrange: (root: string, app: string, started: string) => { folder: string; change: () => void } }> = [
  {
    what: "its clone id file is written over with the started repository's",
    arrange: (_root, app, started) => ({ folder: join(app, "src"), change: () => copyFileSync(cloneIdFile(started), cloneIdFile(app)) }),
  },
  {
    what: "the started checkout is mirrored over it in place, as rsync -a --delete or a sync tool does",
    arrange: (_root, app, started) => ({ folder: join(app, "src"), change: () => mirrorOver(started, app) }),
  },
  {
    what: "the clone id file of its Git folder, which a linked worktree of it shares, is written over with the started repository's",
    arrange: (root, app, started) => {
      const worktree = join(root, "wt");
      git(app, "worktree", "add", "-q", worktree);
      inDatabase(database => recordCheckout(database, worktree));
      return { folder: worktree, change: () => copyFileSync(cloneIdFile(started), cloneIdFile(app)) };
    },
  },
  {
    what: "the database records it under another clone, and a checkout of the started repository under the clone id it holds",
    arrange: (root, app, started) => ({
      folder: join(app, "src"),
      change: () => inDatabase(database => {
        const cloneId = readFileSync(cloneIdFile(app), "utf8").trim();
        const startedKey = recordCheckout(database, started).repositoryKey;
        database.$client.prepare("UPDATE checkouts SET clone_id = 'git:another-clone' WHERE checkout_path = ?").run(realpathSync.native(app));
        database.$client.prepare("INSERT INTO checkouts (checkout_path, repository_key, clone_id, started, created_at, last_seen_at) VALUES (?, ?, ?, NULL, 0, 0)")
          .run(join(root, "elsewhere"), startedKey, cloneId);
      }),
    }),
  },
  {
    what: "Git no longer takes its .git, so the daemon places it outside Git, whose clone id file names the started repository's",
    arrange: (root, app, started) => ({
      folder: app,
      change: () => {
        // Without a HEAD Git passes over the .git folder, and the ceiling keeps it from the folders above the test's.
        rmSync(join(app, ".git", "HEAD"));
        vi.stubEnv("GIT_CEILING_DIRECTORIES", realpathSync.native(root));
        mkdirSync(join(app, ".rerail"));
        copyFileSync(cloneIdFile(started), join(app, ".rerail", "repository-id"));
      },
    }),
  },
];

/**
 * Ways a linked worktree's `.git` file names its Git folder through a link, which the hook never follows: a link to the
 * repository's own folder, and on Windows a link to a server that does not answer, where Windows holds each look about
 * 21 s.
 */
const gitFoldersThroughALink: Array<{ what: string; link: (app: string, path: string) => void; daemonSays: string; started: boolean }> = [
  { what: "a link to its repository's folder", link: (app, path) => folderLink(app, path), daemonSays: "stopped", started: false },
  ...(process.platform === "win32"
    ? [{ what: "a link to a server that does not answer", link: (_app: string, path: string) => symlinkSync(`\\\\${silentServer()}\\share`, path, "dir"), daemonSays: "started", started: true }]
    : []),
];

/**
 * A linked worktree at `<root>/wt` of `<root>/app`, a repository stopped as a whole, recorded with no on/off of its own,
 * where the daemon's answer of stopped is kept (`keptStopped`); and its repository's key.
 */
async function keptInAWorktree(host: StartCheckHost, root: string): Promise<{ app: string; worktree: string; repositoryKey: string }> {
  const app = gitRepository(join(root, "app"));
  startTestRepository(app);
  stopRerail(app, silently, { repository: true });
  const worktree = join(root, "wt");
  git(app, "worktree", "add", "-q", worktree);
  const repositoryKey = inDatabase(database => recordCheckout(database, worktree).repositoryKey);
  await keptStopped(host, worktree);
  return { app, worktree, repositoryKey };
}

/**
 * Names in a linked worktree's checkout, its Git folders and the Rerail home where a named pipe can be put after the
 * daemon's answer of stopped was kept there. A read that opened one as a file would wait for a writer, holding the hook
 * for good; the check of the kept answer reads none of them so, and waits for the daemon at once. A pipe at `.git` is
 * never opened at all: the check opens `.git` only where its look finds a plain file, and the pipe's file id is not the
 * one kept either, so this row shows the hook is not held there and pins neither guard; the case of a look that finds
 * the file before the pipe takes its place pins the open without waiting.
 */
const pipesTheCheckMeets: Array<{ what: string; at: (worktree: string, app: string) => string }> = [
  { what: "the kept answer's name", at: worktree => keptAnswerFile(worktree) },
  { what: "the clone id file of the Git folder the worktree shares", at: (_worktree, app) => cloneIdFile(app) },
  { what: "the commondir file of the worktree's own Git folder", at: (worktree, app) => join(app, ".git", "worktrees", basename(worktree), "commondir") },
  { what: "the worktree's .git file", at: worktree => join(worktree, ".git") },
];

/**
 * Shapes on Windows that would lead the check of a kept answer to a server that does not answer, where Windows holds
 * each look about 21 s, each made in a linked worktree once the daemon's answer of stopped is kept there, with the guard
 * that keeps the check from looking there.
 */
const serverShapesAfterTheAnswer: Array<{ what: string; shape: (root: string, worktree: string) => void }> = [
  {
    // The `.git` file keeps its file id, so the check reads it; `namedFolder` refuses a server path.
    what: "the worktree's .git file is written over in place to name a Git folder on a server",
    shape: (_root, worktree) => rewrittenInPlace(join(worktree, ".git"), `gitdir: \\\\${silentServer()}\\share\\app.git\n`),
  },
  {
    // The look at each folder from the top down stops at the link.
    what: "the worktree's .git file is written over in place to name a Git folder through a local link to a server",
    shape: (root, worktree) => {
      symlinkSync(`\\\\${silentServer()}\\share`, join(root, "srv"), "dir");
      rewrittenInPlace(join(worktree, ".git"), `gitdir: ${join(root, "srv", "app.git")}\n`);
    },
  },
  {
    // `.git` is looked at as it is, never through the link, whose own file id is not the one kept.
    what: "the worktree's .git is a link to a local path through a link to a server",
    shape: (root, worktree) => {
      symlinkSync(`\\\\${silentServer()}\\share`, join(root, "srv"), "dir");
      retried(() => renameSync(join(worktree, ".git"), join(root, "wt.git")));
      symlinkSync(join(root, "srv", "app.git"), join(worktree, ".git"), "dir");
    },
  },
  {
    // The folders of kept answers are looked at as they are, never through a link.
    what: "the folder of kept answers is a link to a server",
    shape: () => {
      const answers = join(process.env.RERAIL_HOME!, "stopped-answers");
      retried(() => renameSync(answers, `${answers}-before`));
      symlinkSync(`\\\\${silentServer()}\\share`, answers, "dir");
    },
  },
];

/**
 * `node:fs` for the hook host the test loads next (`startCheckHost`), where this process's list of open files cannot be
 * read, as `/proc` keeps it for a process of another user: `readdirSync` of `/proc/<pid>/fd` fails with `EACCES`.
 */
function openFilesUnlisted(): void {
  vi.doMock("node:fs", async () => {
    const actual = await vi.importActual<typeof import("node:fs")>("node:fs");
    const readdirSync = (...args: Parameters<typeof actual.readdirSync>): ReturnType<typeof actual.readdirSync> => {
      if (String(args[0]) !== `/proc/${process.pid}/fd`) return actual.readdirSync(...args);
      throw Object.assign(new Error(`EACCES: permission denied, scandir '${String(args[0])}'`), { code: "EACCES" });
    };
    return { ...actual, default: { ...actual, readdirSync }, readdirSync };
  });
}

/**
 * `node:fs` for the hook host the test loads next (`startCheckHost`), whose `lstatSync` of `path` gives what it gives
 * now, whatever is put there after: the check's look finds the file it was, and its open meets what is there then, as
 * when a pipe is swapped in between the two.
 */
function lookFindsTheFileThatWas(path: string): void {
  const was = { plain: lstatSync(path), big: lstatSync(path, { bigint: true }) };
  vi.doMock("node:fs", async () => {
    const actual = await vi.importActual<typeof import("node:fs")>("node:fs");
    const look = (...args: Parameters<typeof actual.lstatSync>): ReturnType<typeof actual.lstatSync> => {
      if (String(args[0]) !== path) return actual.lstatSync(...args);
      return (args[1] as { bigint?: boolean } | undefined)?.bigint === true ? was.big : was.plain;
    };
    // `lstatSync` is overloaded by its options; `look` answers each overload as the real one does.
    const lstatSync = look as typeof actual.lstatSync;
    return { ...actual, default: { ...actual, lstatSync }, lstatSync };
  });
}

/**
 * An answer of stopped kept for `share`, the stopped checkout `local` reached as a share of this machine, as a hook would
 * keep it but for its guard: the database records the checkout at the share's path too, stopped, under its clone id, and
 * the file holds the ids read through the share. A check of it would find each of them as kept.
 */
function plantedForShare(share: string, local: string): void {
  const real = realpathSync.native(share);
  const cloneId = readFileSync(cloneIdFile(local), "utf8").trim();
  const repository = inDatabase(database => {
    const { repositoryKey } = recordCheckout(database, local);
    database.$client.prepare("INSERT INTO checkouts (checkout_path, repository_key, clone_id, started, created_at, last_seen_at) VALUES (?, ?, ?, 0, 0, 0)")
      .run(real, repositoryKey, cloneId);
    return repositoryKey;
  });
  const id = (path: string, look: typeof statSync): string => {
    const { dev, ino } = look(path, { bigint: true });
    return `${dev}:${ino}`;
  };
  const database = defaultRerailDatabasePath();
  const file = keptAnswerFile(share);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, JSON.stringify({
    v: 4, repository, cloneId, depth: 0, folder: id(real, statSync), checkout: id(real, statSync), git: id(join(real, ".git"), lstatSync),
    database: id(database, lstatSync), wal: id(`${database}-wal`, lstatSync), walIndex: id(`${database}-shm`, lstatSync), mark: startMarkFound(database),
    at: Date.now(),
  }));
}

/**
 * Two start checks in each of `folders`, beside a daemon that answers late as a live one reads the folder, its Git
 * stopping right above each folder where `ceiling` says: where the hook repeated stopped though the daemon reads
 * otherwise or took another answer (`wrong`), and where its second check waited though the daemon reads stopped. With a
 * ceiling it is another daemon, named by another start time.
 */
async function checkTrees(
  host: StartCheckHost, folders: ReadonlyArray<[string, string]>, ceiling: boolean, found: { wrong: string[]; waitedThoughStopped: string[] },
): Promise<void> {
  const daemon = await daemonAnsweringAfterProof(host, daemonReads, { startedAt: ceiling ? 2 : 1, scopeOf: daemonScope });
  try {
    for (const [name, path] of folders) {
      const real = realpathSync.native(path);
      vi.stubEnv("GIT_CEILING_DIRECTORIES", ceiling ? dirname(real) : undefined);
      await checkTwice(host, { name: `${name}${ceiling ? " (ceiling)" : ""}`, path, reads: daemonReads(real), ceiling }, found);
    }
  } finally {
    vi.stubEnv("GIT_CEILING_DIRECTORIES", undefined);
    await daemon.close();
  }
}

/** {@link checkTrees} in one folder, where the daemon `reads` as it does. */
async function checkTwice(
  host: StartCheckHost, folder: { name: string; path: string; reads: string; ceiling: boolean }, found: { wrong: string[]; waitedThoughStopped: string[] },
): Promise<void> {
  for (const check of ["first", "second"]) {
    host.proofs.nextCheck();
    const started = await host.hookRerailStarted("claude-code", folder.path);
    const repeated = host.proofs.found.at(-1) === true;
    if (repeated ? folder.reads !== "stopped" : started !== (folder.reads !== "stopped")) found.wrong.push(`${folder.name}, ${check} check: ${folder.reads}`);
    if (check === "second" && !repeated && folder.reads === "stopped" && !folder.ceiling) found.waitedThoughStopped.push(folder.name);
  }
}

/**
 * The folders of `generatedTrees` the daemon reads stopped where a hook waits all the same, its second as its first, as
 * `main` does: the daemon placed the folder in a checkout with no record of its own (a worktree only its clone id
 * places, and on Windows, where Git takes a `.git` whose `objects` is a file, one only a clone id names), or in none it
 * knows (a repository never recorded, a folder outside Git).
 */
const TREE_FOLDERS_THAT_WAIT = [
  ...(process.platform === "win32" ? ["app/objects-file"] : []), "b/.wt/inner", "c-beside", "never", "never/src", "plain", "plain-started/sub",
];

/**
 * The folders of `miscasedFolders` where the hook repeats the daemon's last answer of stopped. Never the one outside
 * Git, whose placement the daemon makes again on every answer. With macOS's realpaths both checkouts, which the daemon
 * places by their stored spelling, as the hook asks. On a case-insensitive Linux folder neither: Git names each top in
 * the case the kernel cached, which the folder the hook asks about, as typed, is not in as spelled.
 */
const MISCASED_FOLDERS_THAT_REPEAT = { macos: ["Git checkout", "linked worktree"], linux: [] as string[] };

describe("the hook start check", () => {
  beforeEach(() => {
    // A shell that runs the tests with its own GIT_DIR, GIT_TRACE or other Git variable would send the tests' Git and
    // the hook's elsewhere.
    for (const name of Object.keys(process.env)) {
      if (name.toUpperCase().startsWith("GIT_") && !TEST_SETUP_GIT_VARIABLES.has(name.toUpperCase())) vi.stubEnv(name, undefined);
    }
  });
  afterEach(() => {
    vi.doUnmock("../../src/engine/git.js");
    vi.doUnmock("../../src/runtime/start-check/stopped-answers.js");
    vi.doUnmock("../../src/engine/storage/database.js");
    vi.doUnmock("../../src/engine/catalog/checkout-control.js");
    vi.doUnmock("node:fs/promises");
    vi.doUnmock("node:fs");
    vi.resetModules();
    vi.unstubAllEnvs();
  });

  it.each(foldersOfEachKind)("asks the daemon first, and takes its prompt answer in $folder without opening the database or checking a kept answer", async ({ make, daemonSays, started }) => {
    const root = await temporaryDirectory({ started: false });
    const workspace = make(root);
    const host = await startCheckHost();
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, daemonSays);
    try {
      expect({
        started: await host.hookRerailStarted("claude-code", workspace, undefined, GRACE_NO_ANSWER_MISSES),
        requests: daemon.requests, opened: host.opened, proofs: host.proofs.found,
      }).toEqual({ started, requests: ["/api/presentation"], opened: [], proofs: [] });
    } finally {
      await daemon.close();
    }
  });

  it("waits for a late daemon where it last answered nothing, and keeps its answer of stopped; the next hook there repeats it without waiting, running no Git and ending its request", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    mkdirSync(join(app, "src"));
    const host = await startCheckHost();
    await keptStopped(host, join(app, "src"));
    // A daemon that would say started: an answer of stopped is the repeat of the one kept. The Git the first daemon ran
    // to name its checkout is not the hook's.
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "started", busyTheWholeTest);
    try {
      host.proofs.nextCheck();
      const gitBefore = host.git.length;
      const started = await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", join(app, "src")));
      expect({ started, git: host.git.slice(gitBefore), proofs: host.proofs.found, ...await endedStartChecks(daemon) })
        .toEqual({ started: false, git: [], proofs: [false, true], asked: 1, ended: 1 });
    } finally {
      await daemon.close();
    }
  });

  it("repeats the daemon's answer kept 25 minutes ago, as a busy daemon that last answered then leaves it", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    keptAt(app, Date.now() - 25 * 60_000);
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "started", busyTheWholeTest);
    try {
      host.proofs.nextCheck();
      const started = await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", app));
      expect({ started, checked: host.proofs.found.at(-1), ...await endedStartChecks(daemon) }).toEqual({ started: false, checked: true, asked: 1, ended: 1 });
    } finally {
      await daemon.close();
    }
  });

  it("asks nothing where the start check an earlier hook there gave up on still waits at the daemon, repeating the answer kept; once the daemon served it, the next hook asks", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    const busy = await countingDaemon(process.env.RERAIL_HOME!, "started", busyTheWholeTest);
    try {
      const first = await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", app));
      const marks = askMarks(app);
      const later: boolean[] = [];
      for (let hook = 0; hook < 3; hook += 1) later.push(await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", app)));
      expect({ first, marks, later, asked: busy.asked.length }).toEqual({ first: false, marks: ["waiting"], later: [false, false, false], asked: 1 });
    } finally {
      // The held check goes on as the daemon closes, and is served then.
      await busy.close();
    }
    const prompt = await countingDaemon(process.env.RERAIL_HOME!, "stopped");
    try {
      expect({ started: await host.hookRerailStarted("claude-code", app), asked: prompt.asked.length, marks: askMarks(app) }).toEqual({ started: false, asked: 1, marks: [] });
    } finally {
      await prompt.close();
    }
  });

  it("removes the mark of its own start check where the daemon answers it in time, though the daemon removed none", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    const prompt = await countingDaemon(process.env.RERAIL_HOME!, "stopped", undefined, { marks: "kept" });
    try {
      expect({ started: await host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES), asked: prompt.asked.length, marks: askMarks(app) })
        .toEqual({ started: false, asked: 1, marks: [] });
    } finally {
      await prompt.close();
    }
  });

  it("marks no check as waiting where the daemon served a start check of the folder after this hook asked, so the next hook there asks", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    // The daemon serves an earlier check of the folder while this one waits in its queue, then holds this one.
    const daemonSide = new ServedAsks();
    daemonSide.open(process.env.RERAIL_HOME!, { pid: process.pid, startedAt: 1 });
    const servedEarlier = async (url: URL): Promise<never> => {
      daemonSide.served(url.searchParams.get("repository") ?? "");
      return await busyTheWholeTest();
    };
    const busy = await countingDaemon(process.env.RERAIL_HOME!, "started", servedEarlier, { marks: "kept" });
    try {
      const first = await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", app));
      const marks = askMarks(app);
      const next = await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", app));
      expect({ first, marks, next, asked: busy.asked.length }).toEqual({ first: false, marks: [], next: false, asked: 2 });
    } finally {
      await busy.close();
    }
  });

  it("makes no mark beside a daemon that keeps none, as one of an older build, and each hook there asks", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app, { marks: "none" });
    const busy = await countingDaemon(process.env.RERAIL_HOME!, "started", busyTheWholeTest, { marks: "none" });
    try {
      const answers: boolean[] = [];
      for (let hook = 0; hook < 2; hook += 1) answers.push(await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", app)));
      expect({ answers, asked: busy.asked.length, asks: existsSync(join(dirname(keptAnswerFile(app)), "asks")) }).toEqual({ answers: [false, false], asked: 2, asks: false });
    } finally {
      await busy.close();
    }
  });

  it.each([
    { what: "no answer is kept there", keep: false },
    { what: "the answer kept there no longer holds, the checkout started since", keep: true },
  ])("asks the daemon and takes its answer where a mark says a check of the folder waits but $what: a mark, forged or left, repeats only what the kept answer allows", async ({ keep }) => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    if (keep) {
      await keptStopped(host, app);
      startTestRepository(app);
    }
    const prompt = await countingDaemon(process.env.RERAIL_HOME!, "started", undefined, { marks: "kept" });
    plantMark(app, "waiting");
    try {
      expect({ started: await host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES), asked: prompt.asked.length }).toEqual({ started: true, asked: 1 });
    } finally {
      await prompt.close();
    }
  });

  it("reads no mark of a daemon before the one daemon.json names, whose queue is gone with it", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app, { startedAt: 2 });
    plantMark(app, "waiting", { startedAt: 1 });
    const busy = await countingDaemon(process.env.RERAIL_HOME!, "started", busyTheWholeTest, { startedAt: 2 });
    try {
      const first = await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", app));
      expect({ first, asked: busy.asked.length, marks: askMarks(app, { startedAt: 2 }) }).toEqual({ first: false, asked: 1, marks: ["waiting"] });
    } finally {
      await busy.close();
    }
  });

  it.each(changesSinceTheAnswer)("waits for a late daemon where it last answered stopped, once $what", async ({ change, startedAt, held }) => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    mkdirSync(join(app, "src"));
    const holder = held === true ? openRerailDatabase(defaultRerailDatabasePath()) : undefined;
    try {
      const host = await startCheckHost();
      await keptStopped(host, join(app, "src"));
      const files = databaseFileIds();
      change(app);
      const daemon = await daemonAnsweringAfterProof(host, "started", { startedAt });
      try {
        host.proofs.nextCheck();
        expect({ started: await host.hookRerailStarted("claude-code", join(app, "src")), checked: host.proofs.found.at(-1), requests: daemon.requests, ended: daemon.ended() })
          .toEqual({ started: true, checked: false, requests: ["/api/presentation"], ended: 0 });
        if (holder !== undefined) expect(databaseFileIds()).toBe(files);
      } finally {
        await daemon.close();
      }
    } finally {
      holder?.$client.close();
    }
  });

  it.each(cloneIdsChanged)("waits for a late daemon where it last answered stopped in a repository stopped as a whole, once $what, and takes its answer that Rerail runs there", async ({ arrange }) => {
    const root = await temporaryDirectory({ started: false });
    const started = startedApp(join(root, "started"));
    mkdirSync(join(started, "src"));
    const app = gitRepository(join(root, "app"));
    startTestRepository(app);
    stopRerail(app, silently, { repository: true });
    mkdirSync(join(app, "src"));
    const { folder, change } = arrange(root, app, started);
    const host = await startCheckHost();
    await keptStopped(host, folder);
    change();
    const daemon = await daemonAnsweringAfterProof(host, repository => host.daemonReads(repository));
    try {
      host.proofs.nextCheck();
      expect({
        daemonReads: host.daemonReads(realpathSync.native(folder)), started: await host.hookRerailStarted("claude-code", folder),
        checked: host.proofs.found.at(-1), requests: daemon.requests, ended: daemon.ended(),
      }).toEqual({ daemonReads: "not-set-up", started: true, checked: false, requests: ["/api/presentation"], ended: 0 });
    } finally {
      await daemon.close();
    }
  });

  it("keeps no answer where the checkout's clone id file is gone, which the daemon reads as no id, and waits for a late daemon there", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    rmSync(cloneIdFile(app));
    const host = await startCheckHost();
    const prompt = await countingDaemon(process.env.RERAIL_HOME!, "stopped");
    try {
      expect(await host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES)).toBe(false);
    } finally {
      await prompt.close();
    }
    const late = await daemonAnsweringAfterProof(host, "stopped");
    try {
      host.proofs.nextCheck();
      expect({ kept: existsSync(keptAnswerFile(app)), started: await host.hookRerailStarted("claude-code", app), checked: host.proofs.found.at(-1) })
        .toEqual({ kept: false, started: false, checked: false });
    } finally {
      await late.close();
    }
  });

  it.for(gitFoldersThroughALink)("waits for a late daemon in a linked worktree whose .git file names its Git folder through $what, which it never follows", async ({ link, daemonSays, started }, { skip }) => {
    const root = await temporaryDirectory({ started: false });
    const app = gitRepository(join(root, "app"));
    startTestRepository(app);
    stopRerail(app, silently, { repository: true });
    const worktree = join(root, "wt");
    git(app, "worktree", "add", "-q", worktree);
    inDatabase(database => recordCheckout(database, worktree));
    const host = await startCheckHost();
    await keptStopped(host, worktree);
    try {
      link(app, join(root, "linked"));
    } catch (error) {
      if (codeOf(error) === "EPERM") skip("this user may not make a symbolic link");
      throw error;
    }
    rewrittenInPlace(join(worktree, ".git"), `gitdir: ${join(root, "linked", ".git", "worktrees", "wt")}\n`);
    const daemon = await daemonAnsweringAfterProof(host, daemonSays);
    try {
      host.proofs.nextCheck();
      expect({ started: await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", worktree)), checked: host.proofs.found.at(-1) })
        .toEqual({ started, checked: false });
      expect(host.proofs.took.at(-1)).toBeLessThan(GIVEN_UP_WITHIN_MS);
    } finally {
      await daemon.close();
    }
  });

  it("removes the answer it kept where the daemon then answers otherwise, and the next hook there waits for a late daemon", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    const prompt = await countingDaemon(process.env.RERAIL_HOME!, "started");
    try {
      expect({ started: await host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES), kept: existsSync(keptAnswerFile(app)) })
        .toEqual({ started: true, kept: false });
    } finally {
      await prompt.close();
    }
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      host.proofs.nextCheck();
      expect({ started: await host.hookRerailStarted("claude-code", app), checked: host.proofs.found.at(-1), requests: daemon.requests })
        .toEqual({ started: true, checked: false, requests: ["/api/presentation"] });
    } finally {
      await daemon.close();
    }
  });

  it("leaves one whole answer where many hooks in one folder keep the daemon's answer at once", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "stopped");
    try {
      for (let round = 0; round < 5; round += 1) {
        await rm(keptAnswerFile(app), { force: true });
        const hooks = Array.from({ length: 8 }, () => host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES));
        expect(await Promise.all(hooks)).toEqual(Array.from({ length: 8 }, () => false));
        expect(JSON.parse(readFileSync(keptAnswerFile(app), "utf8"))).toMatchObject({ v: 4, depth: 0, cloneId: readFileSync(cloneIdFile(app), "utf8").trim() });
        expect({ kept: answersBeside(app), marks: readdirSync(join(dirname(keptAnswerFile(app)), "asks")) }).toEqual({ kept: [basename(keptAnswerFile(app))], marks: [] });
      }
    } finally {
      await daemon.close();
    }
  });

  it("keeps one whole answer where 100 hooks in one folder write it again and repeat it at once: none fails, none reads part of one, and a read Windows refuses only waits", { timeout: 120_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const traffic = watchKeptAnswers();
    const host = await startCheckHost();
    await keptStopped(host, app);
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "stopped", codexAnsweredAtOnce);
    try {
      for (let round = 0; round < 3; round += 1) {
        keptAt(app, Date.now() - 25 * 60_000);
        const [readsBefore, checksBefore] = [traffic.reads.length, host.proofs.found.length];
        const answers = settled(await Promise.allSettled(hooksAtOnce(host, app, 100)));
        const reads = traffic.reads.slice(readsBefore);
        const refusedReads = reads.filter(read => "code" in read).length;
        const waited = host.proofs.found.slice(checksBefore).filter(found => found !== true).length;
        expect({
          answers, partReads: reads.filter(read => "text" in read && !wholeKeptAnswer(read.text)),
          refusedOtherwise: [...reads.flatMap(read => ("code" in read ? [read.code] : [])), ...traffic.renames.filter(code => code !== "renamed")]
            .filter(code => process.platform !== "win32" || !WINDOWS_HOLDS.includes(code)),
          waitedThoughWhole: Math.max(0, waited - refusedReads), written: traffic.renames.includes("renamed"),
          folder: answersBeside(app), whole: wholeKeptAnswer(readFileSync(keptAnswerFile(app), "utf8")),
        }).toEqual({
          answers: Array.from({ length: 100 }, () => false), partReads: [], refusedOtherwise: [], waitedThoughWhole: 0, written: true,
          folder: [basename(keptAnswerFile(app))], whole: true,
        });
      }
    } finally {
      await daemon.close();
    }
  });

  it.runIf(process.platform === "win32")("answers every hook where another program holds the kept answer open sharing only reads, as a virus scan can: Windows refuses to write it again, and the answer kept stays whole and is repeated", { timeout: 60_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const traffic = watchKeptAnswers();
    const host = await startCheckHost();
    await keptStopped(host, app);
    keptAt(app, Date.now() - 25 * 60_000);
    const kept = readFileSync(keptAnswerFile(app), "utf8");
    const holder = await heldByAnotherProgram(keptAnswerFile(app), "Read", "Read");
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "stopped", codexAnsweredAtOnce);
    try {
      const [checksBefore, renamesBefore] = [host.proofs.found.length, traffic.renames.length];
      const answers = settled(await Promise.allSettled(hooksAtOnce(host, app, 20)));
      const renames = traffic.renames.slice(renamesBefore);
      expect({
        answers, refused: renames.length > 0 && renames.every(code => WINDOWS_HOLDS.includes(code)),
        repeated: host.proofs.found.slice(checksBefore), kept: readFileSync(keptAnswerFile(app), "utf8"), folder: answersBeside(app),
      }).toEqual({
        answers: Array.from({ length: 20 }, () => false), refused: true, repeated: Array.from({ length: 10 }, () => true), kept,
        folder: [basename(keptAnswerFile(app))],
      });
    } finally {
      await holder.end();
      await daemon.close();
    }
  });

  it("waits for a late daemon where the answer kept is a link to a copy of it, and writes nothing through it", async ({ skip }) => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    const [file, copy] = [keptAnswerFile(app), join(root, "copy.json")];
    copyFileSync(file, copy);
    rmSync(file);
    try {
      symlinkSync(copy, file, "file");
    } catch (error) {
      if (codeOf(error) === "EPERM") skip("this user may not make a symbolic link");
      throw error;
    }
    const before = readFileSync(copy, "utf8");
    const late = await daemonAnsweringAfterProof(host, "started");
    try {
      host.proofs.nextCheck();
      expect({ started: await host.hookRerailStarted("claude-code", app), checked: host.proofs.found.at(-1), requests: late.requests })
        .toEqual({ started: true, checked: false, requests: ["/api/presentation"] });
    } finally {
      await late.close();
    }
    const prompt = await countingDaemon(process.env.RERAIL_HOME!, "stopped");
    try {
      expect(await host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES)).toBe(false);
    } finally {
      await prompt.close();
    }
    expect({ copy: readFileSync(copy, "utf8"), link: lstatSync(file).isSymbolicLink(), whole: wholeKeptAnswer(readFileSync(file, "utf8")) })
      .toEqual({ copy: before, link: false, whole: true });
  });

  it.each([
    { what: "the folder of kept answers", at: (home: string): string => join(home, "stopped-answers") },
    { what: "this daemon's folder of kept answers", at: (home: string): string => join(home, "stopped-answers", `${process.pid}-1`) },
  ])("writes nothing through a link in place of $what where the daemon answers stopped in time, and leaves what it leads to", async ({ at }) => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const target = join(root, "target");
    mkdirSync(target);
    writeFileSync(join(target, "canary.txt"), "canary");
    const link = at(process.env.RERAIL_HOME!);
    mkdirSync(dirname(link), { recursive: true });
    if (existsSync(link)) removeFolderSync(link);
    folderLink(target, link);
    const host = await startCheckHost();
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "stopped");
    try {
      expect(await host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES)).toBe(false);
    } finally {
      await daemon.close();
    }
    expect({ target: readdirSync(target), canary: readFileSync(join(target, "canary.txt"), "utf8") }).toEqual({ target: ["canary.txt"], canary: "canary" });
  });

  it.runIf(process.platform === "win32")("keeps no answer for a stopped checkout reached as a share of this machine (\\\\localhost\\<drive>$), and waits for the daemon there", async ({ skip }) => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const local = realpathSync.native(app);
    const share = `\\\\localhost\\${local[0]!.toLowerCase()}$${local.slice(2)}`;
    if (!existsSync(share)) skip("this machine shares no drive to this user as <letter>$");
    const host = await startCheckHost();
    // A daemon whose Git reads the share names the checkout there, as the hook spells the folder it asks about.
    const repositoryKey = host.daemonScope(app)!.repositoryKey;
    const scopeOf = (folder: string): DaemonScope => ({ repositoryKey, checkoutPath: folder });
    const prompt = await countingDaemon(process.env.RERAIL_HOME!, "stopped", undefined, { scopeOf });
    try {
      expect(await host.hookRerailStarted("claude-code", share, undefined, GRACE_NO_ANSWER_MISSES)).toBe(false);
    } finally {
      await prompt.close();
    }
    const late = await daemonAnsweringAfterProof(host, "started", { scopeOf });
    try {
      host.proofs.nextCheck();
      expect({
        kept: readdirSync(join(process.env.RERAIL_HOME!, "stopped-answers", `${process.pid}-1`)).some(name => name.endsWith(".json")),
        started: await host.hookRerailStarted("claude-code", share),
        checked: host.proofs.found.at(-1), asked: [...prompt.asked, ...late.asked].map(folder => (folder.startsWith("\\\\") ? "the share" : folder)),
      }).toEqual({ kept: false, started: true, checked: false, asked: ["the share", "the share"] });
    } finally {
      await late.close();
    }
  });

  it("keeps a new daemon's first answer and removes nothing of the folders the daemons before it kept, which the daemon removes once it runs", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const answers = join(process.env.RERAIL_HOME!, "stopped-answers");
    for (const earlier of ["7-1", "8-1"]) {
      mkdirSync(join(answers, earlier), { recursive: true });
      writeFileSync(join(answers, earlier, `${"0".repeat(32)}.json`), "{}");
    }
    const removed = watchRemovals();
    const host = await startCheckHost();
    await keptStopped(host, app);
    expect({ removed: removed.filter(path => !path.includes(`${sep}${process.pid}-1`)), answers: readdirSync(answers).sort() })
      .toEqual({ removed: [], answers: [`${process.pid}-1`, "7-1", "8-1"].sort() });
  });

  it("removes, for a daemon once it runs, the folders of answers the daemons before it kept, leaving its own, the start mark and the stuck mark, and nothing a link leads to", async () => {
    const root = await temporaryDirectory({ started: false });
    const outside = join(root, "outside");
    mkdirSync(join(outside, "7-1"), { recursive: true });
    writeFileSync(join(outside, "keep.txt"), "kept");
    const home = process.env.RERAIL_HOME!;
    const answers = join(home, "stopped-answers");
    for (const daemon of ["7-1", `${process.pid}-1`]) {
      mkdirSync(join(answers, daemon), { recursive: true });
      writeFileSync(join(answers, daemon, `${"0".repeat(32)}.json`), "{}");
    }
    // A writer may be renewing the mark as the daemon starts; one removed then reads as none, as before any renewal.
    for (const mark of [START_MARK, STUCK_MARK]) writeFileSync(join(answers, mark), "");
    folderLink(outside, join(answers, "7-1", "linked"));
    folderLink(outside, join(answers, "8-1"));
    // A home whose folder of kept answers is itself a link: nothing in what it leads to is removed.
    const linkedHome = join(root, "linked-home");
    mkdirSync(linkedHome);
    folderLink(outside, join(linkedHome, "stopped-answers"));
    for (const each of [home, linkedHome]) await removeEarlierAnswers(each, { pid: process.pid, startedAt: 1 });
    expect({ answers: readdirSync(answers).sort(), own: readdirSync(join(answers, `${process.pid}-1`)), outside: readdirSync(outside).sort() })
      .toEqual({ answers: [`${process.pid}-1`, START_MARK, STUCK_MARK].sort(), own: [`${"0".repeat(32)}.json`], outside: ["7-1", "keep.txt"] });
  });

  it.each([
    { what: "no daemon is recorded", daemon: undefined, keeps: false },
    { what: "the daemon answers started", daemon: { says: "started", answersFolder: true }, keeps: false },
    { what: "the daemon answers stopped in a home where no daemon made the folder of kept answers", daemon: { says: "stopped", answersFolder: false }, keeps: false },
    { what: "the daemon answers stopped", daemon: { says: "stopped", answersFolder: true }, keeps: true },
  ])("leaves the Rerail home as it found it but for the answer it keeps where $what", async ({ daemon, keeps }) => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    const standIn = daemon === undefined ? undefined : await countingDaemon(process.env.RERAIL_HOME!, daemon.says, undefined, { answersFolder: daemon.answersFolder });
    const before = homeEntries();
    try {
      await host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES);
    } finally {
      await standIn?.close();
    }
    const kept = relative(process.env.RERAIL_HOME!, keptAnswerFile(app));
    expect(homeEntries()).toEqual([...new Set([...before, ...(keeps ? [dirname(kept), kept] : [])])].sort());
  });

  it("keeps no answer, and makes no folder of kept answers, where that folder goes while the hook waits for the daemon", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    const answers = join(process.env.RERAIL_HOME!, "stopped-answers");
    // The folder goes after the hook found the start mark in it and before the daemon answers: a change that commits
    // then renews no mark, so an answer kept with the mark found before would be read past it.
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "stopped", async () => removeFolderSync(answers));
    try {
      expect(await host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES)).toBe(false);
    } finally {
      await daemon.close();
    }
    expect({ requests: daemon.requests, answers: existsSync(answers) }).toEqual({ requests: ["/api/presentation"], answers: false });
  });

  it("makes the folder of kept answers for a daemon where nothing is at its name, and leaves a link or a file there as it is", async () => {
    const root = await temporaryDirectory({ started: false });
    const outside = join(root, "outside");
    mkdirSync(outside);
    const homes = ["empty", "linked", "file"].map(name => join(root, name));
    for (const home of homes) mkdirSync(home);
    folderLink(outside, join(homes[1]!, "stopped-answers"));
    writeFileSync(join(homes[2]!, "stopped-answers"), "");
    for (const home of homes) makeAnswersFolder(home);
    expect({ kinds: homes.map(home => entryOf(join(home, "stopped-answers"))), outside: readdirSync(outside) })
      .toEqual({ kinds: ["folder", "link", "file"], outside: [] });
  });

  it("keeps no answer that rests on a folder outside Git, which the daemon places again on every answer, and waits there", async () => {
    const root = await temporaryDirectory({ started: false });
    const plain = join(root, "plain");
    mkdirSync(plain);
    startTestRepository(plain);
    stopTestRepository(plain);
    const host = await startCheckHost();
    for (const check of ["first", "second"]) {
      const daemon = await daemonAnsweringAfterProof(host, "stopped");
      try {
        host.proofs.nextCheck();
        expect({ check, started: await host.hookRerailStarted("claude-code", plain), checked: host.proofs.found.at(-1), kept: existsSync(keptAnswerFile(plain)) })
          .toEqual({ check, started: false, checked: false, kept: false });
      } finally {
        await daemon.close();
      }
    }
  });

  it("opens no database in a repository Rerail never recorded, which only its remote can place, and waits for the daemon", async () => {
    const root = await temporaryDirectory({ started: false });
    startedApp(root);
    const never = gitRepository(join(root, "never"));
    const host = await startCheckHost();
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      expect({ started: await host.hookRerailStarted("claude-code", never), opened: host.opened, proofs: host.proofs.found, requests: daemon.requests })
        .toEqual({ started: true, opened: [], proofs: [false], requests: ["/api/presentation"] });
    } finally {
      await daemon.close();
    }
  });

  it.each(foldersTheDaemonDecides)("waits for the daemon in $folder when it answers late, and takes its answer", async ({ make, daemonSays, started }) => {
    const root = await temporaryDirectory({ started: false });
    const workspace = make(root);
    const host = await startCheckHost();
    const daemon = await daemonAnsweringAfterProof(host, daemonSays);
    try {
      expect({ started: await host.hookRerailStarted("claude-code", workspace), proofs: host.proofs.found, requests: daemon.requests, ended: daemon.ended() })
        .toEqual({ started, proofs: [false], requests: ["/api/presentation"], ended: 0 });
    } finally {
      await daemon.close();
    }
  });

  it("waits for a late daemon that last answered stopped where this process cannot open the database, and with no daemon recorded its failure is the hook's", async () => {
    const root = await temporaryDirectory({ started: false });
    const home = process.env.RERAIL_HOME!;
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      await writeFile(join(home, "rerail.sqlite"), "not a database");
      await startedFromLateDaemon(host, daemon, app);
    } finally {
      await daemon.close();
    }
    await rm(join(home, "daemon.json"));
    await expect(host.hookRerailStarted("claude-code", app)).rejects.toThrow("not a database");
  });

  it("waits for a late daemon that last answered stopped where the database lacks one of this version's schema steps, and leaves the file as it is", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    const step = newestStepNotRun();
    // A daemon that would say started: an answer of stopped would be the repeat.
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      await startedFromLateDaemon(host, daemon, app);
      expect(stepRun(step)).toBe(false);
    } finally {
      await daemon.close();
    }
  });

  it.each([
    { held: "for the whole check", releaseMs: undefined },
    { held: "and lets it go 1.5 s later", releaseMs: 1_500 },
  ])("waits for a late daemon that last answered stopped without waiting on the lock where another process holds the database locked $held", { timeout: 30_000 }, async ({ releaseMs }) => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    companionsLeftByAReader();
    const holder = await lockedByAnotherProcess(releaseMs);
    // The daemon reads nothing either while the lock is held.
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      await startedFromLateDaemon(host, daemon, app);
    } finally {
      await holder.end();
      await daemon.close();
    }
  });

  it.runIf(process.platform === "win32")("waits for a late daemon that last answered stopped at once where another program holds the database file open and shares nothing", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    companionsLeftByAReader();
    const holder = await heldWithoutSharing();
    // The daemon cannot open the file either while the program holds it.
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      await startedFromLateDaemon(host, daemon, app);
    } finally {
      await holder.end();
      await daemon.close();
    }
  });

  it.runIf(process.platform === "win32").for([
    { file: "database", path: () => defaultRerailDatabasePath() },
    { file: "database's WAL", path: () => `${defaultRerailDatabasePath()}-wal` },
  ])("waits for a late daemon that last answered stopped at once where the $file is denied to this user", { timeout: 30_000 }, async ({ path }, { skip }) => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    const daemon = await daemonAnsweringAfterProof(host, "started");
    const holder = await deniedToThisUser(path());
    try {
      if (nodeOpensDespiteDeny(path())) skip("this process's Backup privilege is enabled, so Node's open passes the deny entry");
      await startedFromLateDaemon(host, daemon, app);
    } finally {
      await holder.end();
      await daemon.close();
    }
  });

  it.runIf(process.platform === "win32").for(lockedRanges)("waits for a late daemon that last answered stopped at once where another program locks $what", { timeout: 30_000 }, async ({ suffix, offset, length }) => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    const daemon = await daemonAnsweringAfterProof(host, "started");
    // Written while the daemon's connection holds the database, so the checkout's row stays in the WAL.
    inDatabase(database => database.$client.exec("UPDATE checkouts SET last_seen_at = last_seen_at + 1"));
    const holder = await bytesLockedByAnotherProgram(`${defaultRerailDatabasePath()}${suffix}`, offset, length);
    try {
      await startedFromLateDaemon(host, daemon, app);
    } finally {
      await holder.end();
      await daemon.close();
    }
  });

  it.each(checkoutPathsGoneAWhile)("waits for a late daemon that last answered stopped where $what while the hook checks the answer it kept", async ({ away, folderOf }) => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(join(root, "parent"));
    mkdirSync(join(app, "src"));
    const moves: string[] = [];
    onCheck(2, moves, () => goAway(folderOf(app), away), () => backFromAway(folderOf(app), away));
    const host = await startCheckHost();
    await keptStopped(host, join(app, "src"));
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      host.proofs.nextCheck();
      expect({
        started: await host.hookRerailStarted("claude-code", join(app, "src")), checked: host.proofs.found.at(-1), requests: daemon.requests,
        ended: daemon.ended(), moves: moves.length,
      }).toEqual({ started: true, checked: false, requests: ["/api/presentation"], ended: 0, moves: 1 });
    } finally {
      await daemon.close();
    }
  });

  it("waits for a late daemon that last answered stopped where the checkout's .git folder turns into a gitdir: file while the hook checks the answer it kept", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    mkdirSync(join(app, "src"));
    const moves: string[] = [];
    const separate = join(root, "separate.git");
    onCheck(2, moves, () => retried(() => renameSync(join(app, ".git"), separate)), () => writeFileSync(join(app, ".git"), `gitdir: ${separate}\n`));
    const host = await startCheckHost();
    await keptStopped(host, join(app, "src"));
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      host.proofs.nextCheck();
      expect({
        started: await host.hookRerailStarted("claude-code", join(app, "src")), checked: host.proofs.found.at(-1), requests: daemon.requests,
        ended: daemon.ended(), moves: moves.length,
      }).toEqual({ started: true, checked: false, requests: ["/api/presentation"], ended: 0, moves: 1 });
    } finally {
      await daemon.close();
    }
  });

  it.each([
    { checkout: "a recorded started checkout", make: (app: string) => { const nested = gitRepository(join(app, "nested")); startTestRepository(nested); return nested; } },
    { checkout: "a never-recorded clone of a started repository", make: (app: string) => newClone(join(app, "nested")) },
  ])("waits for a late daemon where $checkout inside a stopped one has its .git turn into a gitdir: file while the hook checks", async ({ make }) => {
    const root = await temporaryDirectory({ started: false });
    startedApp(join(root, "started"));
    const app = stoppedApp(root);
    const nested = make(app);
    mkdirSync(join(nested, "src"));
    const moves: string[] = [];
    const separate = join(root, "separate.git");
    onCheck(1, moves, () => retried(() => renameSync(join(nested, ".git"), separate)), () => writeFileSync(join(nested, ".git"), `gitdir: ${separate}\n`));
    const host = await startCheckHost();
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      expect({
        started: await host.hookRerailStarted("claude-code", join(nested, "src")), proofs: host.proofs.found, requests: daemon.requests, ended: daemon.ended(), moves: moves.length,
      }).toEqual({ started: true, proofs: [false], requests: ["/api/presentation"], ended: 0, moves: 1 });
    } finally {
      await daemon.close();
    }
  });

  it.runIf(process.platform === "linux")("waits for a late daemon that last answered stopped where the WAL and its index are deleted under the daemon and made again by another reader, its start only in the WAL it holds", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = startedApp(root);
    const host = await startCheckHost();
    // The writer is the daemon's connection, and daemon.json names it. One in this process too would share its WAL
    // index with the hook's read, which a hook's own process does not.
    const writer = await startedOnlyInTheWal();
    try {
      const daemonProcess = { holdsDatabase: false, pid: writer.pid };
      await keptStopped(host, app, daemonProcess);
      for (const companion of ["-wal", "-shm"]) rmSync(`${defaultRerailDatabasePath()}${companion}`);
      companionsLeftByAReader();
      const daemon = await daemonAnsweringAfterProof(host, "started", daemonProcess);
      try {
        await startedFromLateDaemon(host, daemon, app);
      } finally {
        await daemon.close();
      }
    } finally {
      await writer.end();
    }
  });

  it.runIf(process.platform === "win32").for(serverShapesAfterTheAnswer)("waits for a late daemon at once where, after it last answered stopped there, $what and the repository is started", { timeout: 60_000 }, async ({ shape }, { skip }) => {
    const root = await temporaryDirectory({ started: false });
    const host = await startCheckHost();
    const { worktree, repositoryKey } = await keptInAWorktree(host, root);
    try {
      shape(root, worktree);
    } catch (error) {
      if (codeOf(error) === "EPERM") skip("this user may not make a symbolic link");
      throw error;
    }
    inDatabase(database => setRepositoryStarted(database, repositoryKey, true));
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      await startedFromLateDaemon(host, daemon, worktree);
    } finally {
      await daemon.close();
    }
  });

  it.skipIf(process.platform === "win32").for(pipesTheCheckMeets)("waits for a late daemon at once where, after it last answered stopped there, a named pipe is at $what and the repository is started", { timeout: 30_000 }, async ({ at }) => {
    const root = await temporaryDirectory({ started: false });
    const host = await startCheckHost();
    const { app, worktree, repositoryKey } = await keptInAWorktree(host, root);
    const pipe = pipeAt(at(worktree, app));
    inDatabase(database => setRepositoryStarted(database, repositoryKey, true));
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      await startedFromLateDaemon(host, daemon, worktree);
    } finally {
      await daemon.close();
      await pipe.end();
    }
  });

  it.skipIf(process.platform === "win32")("waits for a late daemon at once where, after it last answered stopped there, the worktree's .git file is a named pipe once the check's look has found the file, and the repository is started", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const { worktree, repositoryKey } = await keptInAWorktree(await startCheckHost(), root);
    const dotGit = join(realpathSync.native(worktree), ".git");
    lookFindsTheFileThatWas(dotGit);
    const pipe = pipeAt(dotGit);
    inDatabase(database => setRepositoryStarted(database, repositoryKey, true));
    const host = await startCheckHost();
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      await startedFromLateDaemon(host, daemon, worktree);
    } finally {
      await daemon.close();
      await pipe.end();
    }
  });

  it.skipIf(process.platform === "win32")("takes the daemon's answer of stopped in time where a named pipe is at the kept answer's name, and keeps it in a file of its own there", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    const file = keptAnswerFile(app);
    const pipe = pipeAt(file);
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "stopped");
    try {
      const started = await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES));
      expect({ started, kept: lstatSync(file).isFile() && wholeKeptAnswer(readFileSync(file, "utf8")) }).toEqual({ started: false, kept: true });
    } finally {
      await daemon.close();
      await pipe.end();
    }
  });

  it.skipIf(process.platform === "win32")("repeats the daemon's answer at once where a named pipe is at the checkout's .rerail/repository-id, which the daemon reads only for a folder its Git does not take", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    mkdirSync(join(app, ".rerail"), { recursive: true });
    const pipe = pipeAt(join(app, ".rerail", "repository-id"));
    // A daemon that would say started: an answer of stopped is the repeat of the one kept.
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "started", busyTheWholeTest);
    try {
      host.proofs.nextCheck();
      const started = await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", app));
      expect({ started, checked: host.proofs.found.at(-1), ...await endedStartChecks(daemon) }).toEqual({ started: false, checked: true, asked: 1, ended: 1 });
    } finally {
      await daemon.close();
      await pipe.end();
    }
  });

  it("writes the daemon's answer of stopped again only once the one kept is a minute old", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    const keptWhen = (): number => (JSON.parse(readFileSync(keptAnswerFile(app), "utf8")) as { at: number }).at;
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "stopped");
    try {
      const inTime = async (): Promise<boolean> => await host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES);
      expect(await inTime()).toBe(false);
      keptAt(app, Date.now() - 30_000);
      const halfAMinute = keptWhen();
      expect(await inTime()).toBe(false);
      const afterHalfAMinute = keptWhen();
      keptAt(app, Date.now() - 61_000);
      const overAMinute = keptWhen();
      expect(await inTime()).toBe(false);
      expect({ halfAMinuteKept: afterHalfAMinute === halfAMinute, overAMinuteWrittenAgain: keptWhen() > overAMinute }).toEqual({ halfAMinuteKept: true, overAMinuteWrittenAgain: true });
    } finally {
      await daemon.close();
    }
  });

  it.skipIf(process.platform === "win32")("keeps the daemon's answer in folders only this user may open and a file only this user may read and write", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    const file = keptAnswerFile(app);
    expect([dirname(dirname(file)), dirname(file), file].map(path => (statSync(path).mode & 0o777).toString(8))).toEqual(["700", "700", "600"]);
  });

  it.runIf(process.platform === "linux")("keeps no answer of stopped where the daemon's open files cannot be listed, and waits for a late daemon there", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    openFilesUnlisted();
    const host = await startCheckHost();
    await notKept(host, app);
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      await startedFromLateDaemon(host, daemon, app);
    } finally {
      await daemon.close();
    }
  });

  it.skipIf(process.platform === "win32")("waits for a late daemon that last answered stopped where the database is moved aside under it and an older copy put at its name, the daemon reading started from the file it holds", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    // The daemon's connection, held from before its answer to the end, as a running daemon holds its database.
    const daemonHolds = heldOpen(defaultRerailDatabasePath());
    try {
      await keptStopped(host, app);
      const backup = join(root, "backup.sqlite");
      checkpointed();
      copyFileSync(defaultRerailDatabasePath(), backup);
      inDatabase(database => setCheckoutStarted(database, recordCheckout(database, app), true));
      checkpointed();
      movedAsideAndReplaced(backup);
      const daemon = await daemonAnsweringAfterProof(host, "started");
      try {
        await startedFromLateDaemon(host, daemon, app);
      } finally {
        await daemon.close();
      }
    } finally {
      daemonHolds?.close();
    }
  });

  it("waits for a late daemon that last answered stopped where an older copy of the database is written over the live file in place, once the start since then committed, which the daemon read and holds", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    // The daemon's connection, held from before its answer to the end, as a running daemon holds its database.
    const daemonHolds = heldOpen(defaultRerailDatabasePath());
    try {
      await keptStopped(host, app);
      const backup = join(root, "backup.sqlite");
      checkpointed();
      copyFileSync(defaultRerailDatabasePath(), backup);
      inDatabase(database => setCheckoutStarted(database, recordCheckout(database, app), true));
      checkpointed();
      // The daemon reads the start, as its answer to any request in that folder would, and keeps those pages.
      daemonHolds?.prepare("SELECT count(*) FROM checkouts WHERE started = 1").get();
      const id = (): bigint => statSync(defaultRerailDatabasePath(), { bigint: true }).ino;
      const before = id();
      copiedInPlace(backup, defaultRerailDatabasePath());
      expect(id()).toBe(before);
      const daemon = await daemonAnsweringAfterProof(host, "started");
      try {
        await startedFromLateDaemon(host, daemon, app);
      } finally {
        await daemon.close();
      }
    } finally {
      daemonHolds?.close();
    }
  });

  it("keeps the start mark it found before it asked the daemon, so a start that commits while the daemon answers, then is undone in the file in place, leaves nothing to repeat", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    const daemonHolds = heldOpen(defaultRerailDatabasePath());
    try {
      const backup = join(root, "backup.sqlite");
      checkpointed();
      copyFileSync(defaultRerailDatabasePath(), backup);
      // The daemon has read its answer of stopped; the start commits before that answer reaches the hook, and an older
      // copy written over the file in place then makes the file say stopped again.
      const startedMeanwhile = (): Promise<void> => {
        inDatabase(database => setCheckoutStarted(database, recordCheckout(database, app), true));
        checkpointed();
        copiedInPlace(backup, defaultRerailDatabasePath());
        return Promise.resolve();
      };
      const answering = await countingDaemon(process.env.RERAIL_HOME!, "stopped", startedMeanwhile, { scopeOf: host.daemonScope });
      try {
        expect(await host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES)).toBe(false);
      } finally {
        await answering.close();
      }
      expect(existsSync(keptAnswerFile(app))).toBe(true);
      const daemon = await daemonAnsweringAfterProof(host, "started");
      try {
        await startedFromLateDaemon(host, daemon, app);
      } finally {
        await daemon.close();
      }
    } finally {
      daemonHolds?.close();
    }
  });

  it("writes the daemon's answer of stopped again at once where the start mark changed since it kept one, so the next hook there repeats it", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    await keptStopped(host, app);
    // Started and stopped again: the database says stopped as it did, and the start mark is new.
    inDatabase(database => setCheckoutStarted(database, recordCheckout(database, app), true));
    inDatabase(database => setCheckoutStarted(database, recordCheckout(database, app), false));
    const inTime = await countingDaemon(process.env.RERAIL_HOME!, "stopped", undefined, { scopeOf: host.daemonScope });
    try {
      expect(await host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES)).toBe(false);
    } finally {
      await inTime.close();
    }
    const busy = await countingDaemon(process.env.RERAIL_HOME!, "stopped", busyTheWholeTest);
    try {
      expect(await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", app))).toBe(false);
    } finally {
      await busy.close();
    }
  });

  it("keeps no answer where the daemon reads another database than the one beside which the hook found the start mark, whose writers never renew it, and waits there", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    const elsewhere = join(root, "elsewhere", "rerail.sqlite");
    mkdirSync(dirname(elsewhere));
    checkpointed();
    copyFileSync(defaultRerailDatabasePath(), elsewhere);
    // The other database held open as a daemon that reads it holds it, so its WAL and the WAL's index are there.
    const held = heldOpen(elsewhere);
    try {
      await notKept(host, app, { databasePath: elsewhere });
    } finally {
      held?.close();
    }
  });

  it.runIf(process.platform === "linux")("keeps no answer of stopped where the database at its name is not the one the daemon holds, moved aside under it and a copy put there before it answered, and waits for a late daemon there", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    const writer = await heldByAnotherProcess();
    try {
      const backup = join(root, "backup.sqlite");
      checkpointed();
      copyFileSync(defaultRerailDatabasePath(), backup);
      movedAsideAndReplaced(backup);
      const daemonProcess = { holdsDatabase: false, pid: writer.pid };
      await notKept(host, app, daemonProcess);
      const daemon = await daemonAnsweringAfterProof(host, "started", daemonProcess);
      try {
        await startedFromLateDaemon(host, daemon, app);
      } finally {
        await daemon.close();
      }
    } finally {
      await writer.end();
    }
  });

  it.skipIf(process.platform === "win32")("waits for a late daemon that last answered stopped where the WAL is moved aside under it and an older copy put at its name, the daemon reading started from the WAL it holds", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = startedApp(root);
    const host = await startCheckHost();
    // The daemon's writer: the stop alone in an emptied WAL; once that WAL is copied, the stop written into the database
    // file and the start written from the WAL's first frame again, where the copy holds the stop.
    const writer = await cuedWriter([
      "UPDATE repositories SET started = 0; PRAGMA wal_checkpoint(TRUNCATE); UPDATE repositories SET started = 0;",
      "PRAGMA wal_checkpoint(PASSIVE); UPDATE repositories SET started = 1;",
    ]);
    try {
      const wal = `${defaultRerailDatabasePath()}-wal`;
      const older = join(root, "older-wal");
      copyFileSync(wal, older);
      await writer.cue();
      const daemonProcess = { holdsDatabase: false, pid: writer.pid };
      await keptStopped(host, app, daemonProcess);
      renameSync(wal, `${wal}.live`);
      copyFileSync(older, wal);
      const daemon = await daemonAnsweringAfterProof(host, "started", daemonProcess);
      try {
        await startedFromLateDaemon(host, daemon, app);
      } finally {
        await daemon.close();
      }
    } finally {
      await writer.end();
    }
  });

  it.skipIf(process.platform === "win32")("waits for a late daemon that last answered stopped where the WAL's index is removed under it and made again by another reader that keeps it open, the daemon then writing the start through the index it holds", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = startedApp(root);
    const host = await startCheckHost();
    // The daemon's writer: the stop in the database file; once another reader has made a new index and holds it, the
    // start written through the index the writer holds, which the new one never learns of. With no reader holding the
    // new index, each new reader would build it again from the WAL and read the start.
    const writer = await cuedWriter(["UPDATE repositories SET started = 0; PRAGMA wal_checkpoint(TRUNCATE);", "UPDATE repositories SET started = 1;"]);
    try {
      const daemonProcess = { holdsDatabase: false, pid: writer.pid };
      await keptStopped(host, app, daemonProcess);
      rmSync(`${defaultRerailDatabasePath()}-shm`);
      const reader = await heldByAnotherProcess();
      try {
        expect(existsSync(`${defaultRerailDatabasePath()}-shm`)).toBe(true);
        await writer.cue();
        const daemon = await daemonAnsweringAfterProof(host, "started", daemonProcess);
        try {
          await startedFromLateDaemon(host, daemon, app);
        } finally {
          await daemon.close();
        }
      } finally {
        await reader.end();
      }
    } finally {
      await writer.end();
    }
  });

  it.runIf(process.platform === "win32")("checks no answer kept for a folder reached as a share of this machine (\\\\localhost\\<drive>$), one planted there included, and waits for the daemon", async ({ skip }) => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const local = realpathSync.native(app);
    const share = `\\\\localhost\\${local[0]!.toLowerCase()}$${local.slice(2)}`;
    if (!existsSync(share)) skip("this machine shares no drive to this user as <letter>$");
    const host = await startCheckHost();
    // Planted while the daemon holds the database, so the files it names are those a check finds.
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      plantedForShare(share, local);
      host.proofs.nextCheck();
      expect({ started: await host.hookRerailStarted("claude-code", share), checked: host.proofs.found.at(-1) }).toEqual({ started: true, checked: false });
    } finally {
      await daemon.close();
    }
  });

  it.each(readFailures)("$what: $outcome", async ({ error, asked }) => {
    const root = await temporaryDirectory({ started: false });
    const host = await startCheckHost((): never => {
      throw error;
    });
    // A daemon that answers late where the hook waits for it, and not at all where it must end its request.
    const daemon = asked ? await daemonAnsweringAfterProof(host, "started") : await countingDaemon(process.env.RERAIL_HOME!, "started", busyTheWholeTest);
    try {
      const started = host.hookRerailStarted("claude-code", root);
      await (asked ? expect(started).resolves.toBe(true) : expect(started).rejects.toBe(error));
      expect(await endedStartChecks(daemon)).toEqual({ asked: 1, ended: asked ? 0 : 1 });
    } finally {
      await daemon.close();
    }
  });

  it("reads as main does where no daemon is recorded: through its own Git, once", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const host = await startCheckHost();
    expect({ started: await host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES), git: host.git, proofs: host.proofs.found })
      .toEqual({ started: false, git: ["rev-parse"], proofs: [] });
  });

  it("reads the database through its own Git in a started checkout whose daemon.json names a process that is gone", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = startedApp(root);
    const home = process.env.RERAIL_HOME!;
    const gone = spawnSync(process.execPath, ["-e", ""]).pid;
    await writeFile(join(home, "daemon.json"), JSON.stringify({
      pid: gone, host: "127.0.0.1", port: 9, url: "http://127.0.0.1:9/", version: "test", databasePath: join(home, "rerail.sqlite"),
      kernelSocketPath: testEngineEndpoint(home), ui: "bundled", startedAt: 1, packageRoot: home,
    }));
    const host = await startCheckHost();
    expect({ started: await host.hookRerailStarted("claude-code", app), git: host.git }).toEqual({ started: true, git: ["rev-parse"] });
  });

  it.each(agentGitSettings)("waits for a late daemon where the coding agent's $variable would have this process's Git read a started folder as stopped", async ({ variable, arrange }) => {
    const root = await temporaryDirectory({ started: false });
    // A person's environment sets neither of the two the test setup sets; each case sets only its own.
    vi.stubEnv("GIT_CONFIG_GLOBAL", undefined);
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", undefined);
    const { workspace, value } = arrange(root);
    const host = await startCheckHost();
    // The daemon reads the folder from its own environment, as started.
    const daemon = await daemonAnsweringAfterProof(host, "started");
    vi.stubEnv(variable, value);
    try {
      expect(workspaceStartState(workspace)).toBe("stopped");
      expect({ started: await host.hookRerailStarted("claude-code", workspace), proofs: host.proofs.found, requests: daemon.requests })
        .toEqual({ started: true, proofs: [false], requests: ["/api/presentation"] });
    } finally {
      await daemon.close();
    }
  });

  it("repeats only the daemon's last answer of stopped, and wherever that holds, in every folder of these trees", { timeout: 240_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const folders = generatedTrees(root);
    const host = await startCheckHost();
    // Then again with a daemon whose Git stops right above each folder: a ceiling, as for a person whose
    // `safe.directory` refuses the checkout or whose folder is on another filesystem. It is another daemon, which
    // places every folder again, so nothing the first one answered is repeated beside it.
    const found = { wrong: [] as string[], waitedThoughStopped: [] as string[] };
    for (const ceiling of [false, true]) await checkTrees(host, folders, ceiling, found);
    // Where the daemon reads stopped and the second hook waits all the same, as `main` does: the daemon placed the
    // folder outside Git or in a checkout with no record of its own (a repository never recorded, a worktree only its
    // clone id places, a folder outside Git), or a `.git` lies between the folder and the checkout it was placed in.
    expect(found).toEqual({ wrong: [], waitedThoughStopped: TREE_FOLDERS_THAT_WAIT });
  });

  it("asks about a folder typed in another letter case by its stored spelling and repeats only where the daemon reads stopped, with macOS's realpaths", { timeout: 120_000 }, async () => {
    const root = realpathSync.native(await temporaryDirectory({ started: false }));
    const folders = miscasedFolders(root);
    // On macOS checkout keys go through the JS realpath, which keeps a name as typed; the native realpath, which the
    // hook asks the daemon with, gives it as stored.
    const host = await startCheckHost(undefined, { system: "macos", folders });
    const daemon = await daemonAnsweringAfterProof(host, repository => host.daemonReads(repository));
    try {
      const found = await startChecksTypedOtherwise(host, daemon, folders);
      expect({ wrong: found.wrong, answeredItselfIn: found.answeredItselfIn, asked: [...new Set(daemon.asked)] }).toEqual({
        wrong: [], answeredItselfIn: MISCASED_FOLDERS_THAT_REPEAT.macos, asked: folders.map(folder => folder.storedWorkspace),
      });
    } finally {
      await daemon.close();
    }
  });

  it.runIf(process.platform === "linux")("repeats only where the daemon reads stopped where Git names a checkout's top in another letter case than the hook's realpath, as on a case-insensitive Linux folder", { timeout: 120_000 }, async () => {
    const root = realpathSync.native(await temporaryDirectory({ started: false }));
    const folders = miscasedFolders(root);
    // There every realpath keeps a name as typed, and Git's `getcwd` gives the name the kernel cached.
    const host = await startCheckHost(undefined, { system: "linux-case-insensitive", folders });
    const daemon = await daemonAnsweringAfterProof(host, repository => host.daemonReads(repository));
    try {
      const found = await startChecksTypedOtherwise(host, daemon, folders);
      expect({ wrong: found.wrong, answeredItselfIn: found.answeredItselfIn, asked: [...new Set(daemon.asked)] })
        .toEqual({ wrong: [], answeredItselfIn: MISCASED_FOLDERS_THAT_REPEAT.linux, asked: folders.map(folder => folder.workspace) });
    } finally {
      await daemon.close();
    }
  });
});

describe("a session's start where RERAIL_AUTOSTART keeps the daemon off and none runs", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  const notRunning = (setting: string) =>
    `The Rerail daemon is not running, and RERAIL_AUTOSTART=${setting} keeps hooks from starting it, so nothing is recorded until it runs. Start it with rerail daemon start.`;
  const sessionStarts: Array<{
    host: "claude-code" | "codex" | "cursor";
    run: (input: HookInput, options: HookOptions) => Promise<HookOutput>;
    input: (root: string) => HookInput;
    told: (line: string) => HookOutput;
  }> = [
    { host: "claude-code", run: runClaudeHook, input: cwd => ({ cwd, session_id: "s", hook_event_name: "SessionStart", source: "startup" }), told: line => ({ systemMessage: line }) },
    { host: "codex", run: runCodexHook, input: cwd => ({ cwd, session_id: "s", hook_event_name: "SessionStart", source: "startup" }), told: line => ({ systemMessage: line }) },
    {
      host: "cursor", run: runCursorHook,
      input: cwd => ({ workspace_roots: [cwd], conversation_id: "s", hook_event_name: "sessionStart" }),
      told: line => ({
        additional_context: `Rerail checks nothing until its daemon runs, and Cursor shows the user nothing from Rerail. Begin your first reply with this line, exactly as written:\n${line}`,
      }),
    },
  ];

  // Nothing is recorded or checked while no daemon runs, and a person who set `false` may not know it keeps the daemon
  // off: the session's start says so where the person reads it, so they need not run `rerail status` to find out.
  it.each(sessionStarts.flatMap(start => ["0", "false"].map(setting => ({ ...start, setting }))))(
    "tells the person in $host that nothing is recorded until the daemon runs, with RERAIL_AUTOSTART=$setting",
    async ({ run, input, told, setting }) => {
      const root = await temporaryDirectory();
      vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
      vi.stubEnv("RERAIL_AUTOSTART", setting);
      const { embedder: _testRuntime, ...daemonPath } = hookOptions(root);
      expect(await run(input(root), daemonPath)).toEqual(told(notRunning(setting)));
      // What the person asked for is no failure, and nothing started a daemon.
      expect(existsSync(join(process.env.RERAIL_HOME!, HOOK_FAILURES_FILE))).toBe(false);
      expect(existsSync(join(process.env.RERAIL_HOME!, "daemon.json"))).toBe(false);
    },
  );

  const laterPrompts: Record<(typeof sessionStarts)[number]["host"], (cwd: string) => HookInput> = {
    "claude-code": cwd => ({ cwd, session_id: "s", hook_event_name: "UserPromptSubmit", prompt_id: "p1", prompt: "hi" }),
    codex: cwd => ({ cwd, session_id: "s", hook_event_name: "UserPromptSubmit", turn_id: "t1", prompt: "hi" }),
    cursor: cwd => ({ workspace_roots: [cwd], conversation_id: "s", generation_id: "g1", hook_event_name: "beforeSubmitPrompt", prompt: "hi" }),
  };

  // The start's one lookup found no daemon, and one can come up later in the same session: a `rerail daemon start` that
  // was in flight, the daemon's own restart at its heap ceiling, or the person following the line. The session's later
  // events are then recorded, so the start says only what its lookup found, and nothing of the rest of the session.
  it.each(sessionStarts)(
    "tells the person in $host only what its lookup found, since a daemon that comes up records the session's later events",
    { timeout: 20_000 },
    async ({ host, run, input }) => {
      const root = await temporaryDirectory();
      vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
      vi.stubEnv("RERAIL_AUTOSTART", "0");
      const { embedder: _testRuntime, ...daemonPath } = hookOptions(root);
      const said = JSON.stringify(await run(input(root), daemonPath));
      const engine = await heldEngine(root);
      engine.answerFirst();
      try {
        // The daemon is up by the session's next hook, which finds it and sends its event there.
        lookups.runningDaemonEngineEndpoint.mockClear().mockResolvedValueOnce({ status: "running", endpoint: { socketPath: engine.socketPath } });
        await run(laterPrompts[host](root), daemonPath);
        expect(engine.received.length).toBeGreaterThan(0);
        expect(said).toContain("nothing is recorded until it runs");
        expect(said).not.toContain("in this session");
      } finally {
        await engine.close();
      }
    },
  );

  const laterHooks: Array<Omit<(typeof sessionStarts)[number], "told">> = [
    { host: "claude-code", run: runClaudeHook, input: cwd => ({ cwd, session_id: "s", hook_event_name: "UserPromptSubmit", prompt_id: "p1", prompt: "hi" }) },
    { host: "codex", run: runCodexHook, input: cwd => ({ cwd, session_id: "s", hook_event_name: "UserPromptSubmit", prompt: "hi" }) },
    { host: "cursor", run: runCursorHook, input: cwd => ({ workspace_roots: [cwd], conversation_id: "s", hook_event_name: "beforeSubmitPrompt", prompt: "hi" }) },
  ];

  // A later hook meets the same error when a daemon of another package, which it went to replace, is gone before the
  // replacement looks again. That hook answers as any failed hook does (`serveHook` writes `{}`, the reason goes to
  // stderr), and only the session's start tells the person.
  it.each(laterHooks)("answers a later $host hook as any failed hook when the daemon it went to replace is gone", async ({ run, input }) => {
    const root = await temporaryDirectory();
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    vi.stubEnv("RERAIL_AUTOSTART", "0");
    const reason = "RERAIL_AUTOSTART=0 requires a running daemon; run rerail daemon start or set RERAIL_ENGINE_SOCKET";
    lookups.runningDaemonEngineEndpoint.mockClear().mockRejectedValueOnce(lookups.autostartOffError(reason));
    const { embedder: _testRuntime, ...daemonPath } = hookOptions(root);
    expect(await serveText(parsed => run(parsed, daemonPath), JSON.stringify(input(root)))).toEqual({ stdout: "{}\n", stderr: `rerail hook: ${reason}\n` });
    expect(lookups.runningDaemonEngineEndpoint).toHaveBeenCalledTimes(1);
    expect(existsSync(join(process.env.RERAIL_HOME!, HOOK_FAILURES_FILE))).toBe(false);
  });

  // Beside a daemon that answers stopped, the session's start says nothing, as Rerail does not run there, and its answer
  // is kept, so a later hook beside the same daemon, busy, repeats it. A kept answer never stands in for a daemon that is
  // gone: once that one has stopped and the checkout has been started, the session's start says its line.
  it.each(sessionStarts)(
    "lets $host repeat a daemon's answer of stopped while it runs, and says its line once it is gone and the checkout was started",
    { timeout: 20_000 },
    async ({ host, run, input, told }) => {
      const root = await temporaryDirectory({ started: false });
      const app = stoppedApp(root);
      vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
      vi.stubEnv("RERAIL_AUTOSTART", "0");
      const throughDaemon = { storageRoot: join(root, "sessions") };
      const answered = await countingDaemon(process.env.RERAIL_HOME!, "stopped");
      try {
        expect(await run(input(app), throughDaemon)).toEqual({});
        expect(existsSync(keptAnswerFile(app))).toBe(true);
      } finally {
        await answered.close();
      }
      // The same daemon, now busy: it would answer started, so an answer of `{}` within the second is the kept one. The
      // hook ends its request once it repeats, which can be before the request reaches the daemon (Windows 11 under load).
      const busy = await countingDaemon(process.env.RERAIL_HOME!, "started", busyTheWholeTest);
      try {
        expect(await answeredWellBeforeReadLimit(run(laterPrompts[host](app), throughDaemon))).toEqual({});
        const { asked, ended } = await endedStartChecks(busy);
        expect(ended).toBe(asked);
      } finally {
        await busy.close();
      }
      await rm(join(process.env.RERAIL_HOME!, "daemon.json"));
      startTestRepository(app);
      expect(await run(input(app), throughDaemon)).toEqual(told(notRunning("0")));
      expect(existsSync(join(process.env.RERAIL_HOME!, HOOK_FAILURES_FILE))).toBe(false);
    },
  );
});
