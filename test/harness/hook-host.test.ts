import { type ChildProcess, execFileSync, spawn, spawnSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import {
  chmodSync, closeSync, copyFileSync, cpSync, existsSync, lstatSync, mkdirSync, openSync, readdirSync, readFileSync, realpathSync, renameSync, rmSync,
  symlinkSync, writeFileSync,
} from "node:fs";
import { appendFile, mkdir, readdir, readFile, rename, rm, symlink, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { basename, dirname, join, resolve, sep } from "node:path";
import { Readable, Writable } from "node:stream";
import { setTimeout as delay } from "node:timers/promises";
import Database from "better-sqlite3";
import { DrizzleQueryError } from "drizzle-orm";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { adoptRemote, type AdapterDelivery, defaultRerailDatabasePath, lookupCheckout, openRerailDatabase, recordCheckout, sessionStartNote } from "../../src/engine/index.js";
import {
  encodeRerailEventResult,
  type EngineEndpoint,
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
import { runClaudeHook } from "../../src/hosts/claude/index.js";
import { runCodexHook } from "../../src/hosts/codex/index.js";
import { runCursorHook } from "../../src/hosts/cursor/index.js";
import { hookOptions, serveText, temporaryDirectories, sessionDatabaseQuery } from "../support/hook-events.js";
import { holdEngineEndpoint, testEngineEndpoint } from "../support/platform/engine-endpoint.js";
import { startTestRepository, stopTestRepository } from "../support/started-repository.js";

// The lookup of a named engine socket works as it does; a test stands in for one call of it at a time.
const lookups = vi.hoisted(() => ({ engineEndpointAt: vi.fn<(socketPath: string) => Promise<EngineEndpoint>>() }));
vi.mock("../../src/runtime/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
  lookups.engineEndpointAt.mockImplementation(socketPath => actual.engineEndpointAt(socketPath));
  return { ...actual, engineEndpointAt: lookups.engineEndpointAt };
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
 * and `startedAt`, this process and 1 unless given.
 */
async function countingDaemon(
  home: string,
  repositoryState: string | ((repository: string) => string) = "stopped",
  answerWhen?: (url: URL) => Promise<unknown>,
  { holdsDatabase = true, pid = process.pid, startedAt = 1, scopeOf = daemonScope }: StandInOptions = {},
): Promise<CountingDaemon> {
  const requests: string[] = [];
  const asked: string[] = [];
  let ended = 0;
  const closing = Promise.withResolvers<void>();
  const answers: Record<string, (url: URL) => unknown> = {
    "/healthz": () => ({ status: "ok", pid, database: { path: join(home, "rerail.sqlite"), exists: true }, native: { ok: true } }),
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
      if (gone) return;
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
    databasePath: join(home, "rerail.sqlite"), kernelSocketPath, ui: "bundled", startedAt, packageRoot: home,
  }));
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
  vi.doMock("../../src/runtime/stopped-answers.js", async () => {
    const actual = await vi.importActual<typeof import("../../src/runtime/stopped-answers.js")>("../../src/runtime/stopped-answers.js");
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

/** The answer kept for `folder`, as it was kept but at `at`. */
function keptAt(folder: string, at: number): void {
  const file = keptAnswerFile(folder);
  writeFileSync(file, JSON.stringify({ ...JSON.parse(readFileSync(file, "utf8")) as object, at }));
}

/** Whether `text` is a whole kept answer: it parses, and holds every field one is written with. */
function wholeKeptAnswer(text: string): boolean {
  try {
    const kept = JSON.parse(text) as Record<string, unknown>;
    return kept.v === 1 && ["repository", "folder", "checkout", "git"].every(field => typeof kept[field] === "string")
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
 * descriptor, so an open that fails is a read that fails.
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
    const readFileSync = (...args: Parameters<typeof actual.readFileSync>): ReturnType<typeof actual.readFileSync> => {
      const [from] = args;
      if (!kept(from) && !(typeof from === "number" && descriptors.has(from))) return actual.readFileSync(...args);
      try {
        const text = actual.readFileSync(...args);
        traffic.reads.push({ text: String(text) });
        return text;
      } catch (error) {
        traffic.reads.push({ code: codeOf(error) });
        throw error;
      }
    };
    const replaced = { openSync, closeSync, readFileSync };
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
 * `folder` made so that removing it fails until `end`: on Windows another program holds its first file open sharing only
 * reads, as a virus scan can; elsewhere the folder loses its write permission, so its files cannot be deleted.
 */
async function heldFromRemoval(folder: string): Promise<DatabaseHolder> {
  if (process.platform === "win32") return await heldByAnotherProgram(join(folder, readdirSync(folder)[0]!), "Read", "Read");
  chmodSync(folder, 0o500);
  return { end: () => Promise.resolve(chmodSync(folder, 0o700)) };
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
 * named by another start time (`startedAt`) is another daemon, which places every folder again.
 */
const changesSinceTheAnswer: Array<{ what: string; change: (app: string) => void; startedAt?: number }> = [
  { what: "the answer kept does not parse", change: app => writeFileSync(keptAnswerFile(join(app, "src")), "{") },
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
  { what: "a .git is made in the folder", change: app => git(join(app, "src"), "init", "-q") },
  { what: "the checkout is started", change: app => inDatabase(database => setCheckoutStarted(database, recordCheckout(database, app), true)) },
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
    vi.doUnmock("../../src/runtime/stopped-answers.js");
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

  it.each(changesSinceTheAnswer)("waits for a late daemon where it last answered stopped, once $what", async ({ change, startedAt }) => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    mkdirSync(join(app, "src"));
    const host = await startCheckHost();
    await keptStopped(host, join(app, "src"));
    change(app);
    const daemon = await daemonAnsweringAfterProof(host, "started", { startedAt });
    try {
      host.proofs.nextCheck();
      expect({ started: await host.hookRerailStarted("claude-code", join(app, "src")), checked: host.proofs.found.at(-1), requests: daemon.requests, ended: daemon.ended() })
        .toEqual({ started: true, checked: false, requests: ["/api/presentation"], ended: 0 });
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
        expect(JSON.parse(readFileSync(keptAnswerFile(app), "utf8"))).toMatchObject({ v: 1, depth: 0 });
        expect(readdirSync(dirname(keptAnswerFile(app)))).toEqual([basename(keptAnswerFile(app))]);
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
          folder: readdirSync(dirname(keptAnswerFile(app))), whole: wholeKeptAnswer(readFileSync(keptAnswerFile(app), "utf8")),
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
      const checksBefore = host.proofs.found.length;
      const answers = settled(await Promise.allSettled(hooksAtOnce(host, app, 20)));
      expect({
        answers, refused: traffic.renames.length > 0 && traffic.renames.every(code => WINDOWS_HOLDS.includes(code)),
        repeated: host.proofs.found.slice(checksBefore), kept: readFileSync(keptAnswerFile(app), "utf8"), folder: readdirSync(dirname(keptAnswerFile(app))),
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

  it("removes the folders of the daemons before it at a new daemon's first kept answer, following no link in them", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const outside = join(root, "outside");
    mkdirSync(outside);
    writeFileSync(join(outside, "keep.txt"), "kept");
    const answers = join(process.env.RERAIL_HOME!, "stopped-answers");
    mkdirSync(join(answers, "7-1"), { recursive: true });
    writeFileSync(join(answers, "7-1", `${"0".repeat(32)}.json`), "{}");
    folderLink(outside, join(answers, "7-1", "linked"));
    folderLink(outside, join(answers, "8-1"));
    const host = await startCheckHost();
    await keptStopped(host, app);
    expect({ answers: readdirSync(answers), outside: readdirSync(outside) }).toEqual({ answers: [`${process.pid}-1`], outside: ["keep.txt"] });
  });

  it("keeps a new daemon's first answer where an earlier daemon's folder cannot be removed yet, and removes that folder at a later kept answer", { timeout: 30_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const answers = join(process.env.RERAIL_HOME!, "stopped-answers");
    const earlier = join(answers, "7-1");
    mkdirSync(earlier, { recursive: true });
    writeFileSync(join(earlier, `${"0".repeat(32)}.json`), "{}");
    const host = await startCheckHost();
    const held = await heldFromRemoval(earlier);
    try {
      await keptStopped(host, app);
      expect(readdirSync(answers).sort()).toEqual([`${process.pid}-1`, "7-1"].sort());
    } finally {
      await held.end();
    }
    keptAt(app, Date.now() - 2 * 60_000);
    const prompt = await countingDaemon(process.env.RERAIL_HOME!, "stopped");
    try {
      expect(await host.hookRerailStarted("claude-code", app, undefined, GRACE_NO_ANSWER_MISSES)).toBe(false);
    } finally {
      await prompt.close();
    }
    expect(readdirSync(answers)).toEqual([`${process.pid}-1`]);
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

  it.runIf(process.platform === "win32").for([
    { what: "a .git file names a Git folder on a server", shape: (root: string, app: string) => {
      renameSync(join(app, ".git"), join(root, "app.git"));
      writeFileSync(join(app, ".git"), `gitdir: \\\\${silentServer()}\\share\\app.git\n`);
    } },
    { what: "a .git file names a Git folder through a local link to a server", shape: (root: string, app: string) => {
      symlinkSync(`\\\\${silentServer()}\\share`, join(root, "srv"), "dir");
      renameSync(join(app, ".git"), join(root, "app.git"));
      writeFileSync(join(app, ".git"), `gitdir: ${join(root, "srv", "app.git")}\n`);
    } },
    { what: ".git is a link to a local path through a link to a server", shape: (root: string, app: string) => {
      symlinkSync(`\\\\${silentServer()}\\share`, join(root, "srv"), "dir");
      renameSync(join(app, ".git"), join(root, "app.git"));
      symlinkSync(join(root, "srv", "app.git"), join(app, ".git"), "dir");
    } },
    { what: "the folder of kept answers is a link to a server", shape: () => {
      mkdirSync(process.env.RERAIL_HOME!, { recursive: true });
      symlinkSync(`\\\\${silentServer()}\\share`, join(process.env.RERAIL_HOME!, "stopped-answers"), "dir");
    } },
  ])("waits for a late daemon at once where $what", { timeout: 60_000 }, async ({ shape }, { skip }) => {
    const root = await temporaryDirectory({ started: false });
    const app = startedApp(root);
    try {
      shape(root, app);
    } catch (error) {
      if (codeOf(error) === "EPERM") skip("this user may not make a symbolic link");
      throw error;
    }
    const host = await startCheckHost();
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      await startedFromLateDaemon(host, daemon, app);
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
