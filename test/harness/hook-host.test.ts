import { execFileSync, spawnSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { chmodSync, copyFileSync, existsSync, mkdirSync, realpathSync, renameSync, symlinkSync, writeFileSync } from "node:fs";
import { appendFile, mkdir, readdir, readFile, rename, rm, symlink, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { basename, dirname, join, resolve, sep } from "node:path";
import { Readable, Writable } from "node:stream";
import { setTimeout as delay } from "node:timers/promises";
import Database from "better-sqlite3";
import { DrizzleQueryError } from "drizzle-orm";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { type AdapterDelivery, defaultRerailDatabasePath, openRerailDatabase, recordCheckout, sessionStartNote } from "../../src/engine/index.js";
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
 * holds each start check until the promise `answerWhen` gives for it settles, as a daemon busy with other checkouts
 * does, and `close` lets every held one go. On Windows a recorded daemon counts as alive only while its
 * engine pipe exists (`recordedDaemonAlive`), so it holds one.
 */
async function countingDaemon(
  home: string,
  repositoryState: string | ((repository: string) => string) = "stopped",
  answerWhen?: () => Promise<unknown>,
): Promise<CountingDaemon> {
  const requests: string[] = [];
  const asked: string[] = [];
  let ended = 0;
  const closing = Promise.withResolvers<void>();
  const answers: Record<string, (url: URL) => unknown> = {
    "/healthz": () => ({ status: "ok", pid: process.pid, database: { path: join(home, "rerail.sqlite"), exists: true }, native: { ok: true } }),
    "/api/presentation": url => ({
      presence: "compact",
      repositoryState: typeof repositoryState === "string" ? repositoryState : repositoryState(url.searchParams.get("repository") ?? ""),
    }),
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
      await Promise.race([answerWhen(), closing.promise]);
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
    pid: process.pid, host: "127.0.0.1", port, url: `http://127.0.0.1:${port}/`, version: "test",
    databasePath: join(home, "rerail.sqlite"), kernelSocketPath, ui: "bundled", startedAt: 1, packageRoot: home,
  }));
  return {
    requests,
    asked,
    ended: () => ended,
    close: async () => {
      closing.resolve();
      await releaseEndpoint();
      await new Promise<void>(resolve => server.close(() => resolve()));
    },
  };
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

  it.each(hooksWhereRerailIsOff)("answer $host's $event in a stopped checkout without waiting for a busy daemon, ending their request", async ({ run }) => {
    const root = await temporaryDirectory({ started: false });
    startTestRepository(root);
    stopTestRepository(root);
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "started", busyTheWholeTest);
    try {
      expect(await answeredWellBeforeReadLimit(run(root))).toEqual({});
      const { asked, ended } = await endedStartChecks(daemon);
      expect(ended).toBe(asked);
    } finally {
      await daemon.close();
    }
  });

  it.each(hooksWhereRerailIsOff)("answer $host's $event in a folder outside Git that Rerail was never started in without waiting for a busy daemon", async ({ run }) => {
    const root = await temporaryDirectory({ started: false });
    // Rerail runs in another folder on the machine, so the database is there.
    const elsewhere = join(root, "elsewhere");
    const never = join(root, "never");
    await mkdir(elsewhere);
    await mkdir(never);
    startTestRepository(elsewhere);
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "started", busyTheWholeTest);
    try {
      expect(await answeredWellBeforeReadLimit(run(never))).toEqual({});
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

/** What each proof of the hook start check (`stoppedWithoutGit`) found, in order. */
interface ProofLog {
  readonly found: Array<boolean | "threw">;
  run(proof: () => boolean): boolean;
  /** Marks the start of another start check: `ran` then waits for its proof. */
  nextCheck(): void;
  /** Settles once the proof of the start check under way has run. */
  ran(): Promise<void>;
}

function proofLog(): ProofLog {
  const found: Array<boolean | "threw"> = [];
  const waiting: Array<{ count: number; resolve: () => void }> = [];
  let before = 0;
  const record = (result: boolean | "threw"): void => {
    found.push(result);
    for (const each of waiting.filter(entry => found.length >= entry.count)) each.resolve();
  };
  return {
    found,
    run: proof => {
      try {
        const result = proof();
        record(result);
        return result;
      } catch (error) {
        record("threw");
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
  /** Each folder whose Git search it walks on disk (`gitSearchEnds`). */
  walked: string[];
  proofs: ProofLog;
  /** What the daemon answers for `folder` (`startStateReader`), with the folders' names spelled as this host spells them. */
  daemonReads: (folder: string) => string;
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
 * Git commands, database opens, walks of Git's search and proofs, which `proof` replaces when given. With `spelling`,
 * its realpaths answer as that system's do.
 */
async function startCheckHost(proof?: (workspace: string) => boolean, spelling?: Spelling): Promise<StartCheckHost> {
  const seen = { git: [] as string[], opened: [] as string[], walked: [] as string[], proofs: proofLog() };
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
    const openRerailDatabaseAsIs: typeof actual.openRerailDatabaseAsIs = path => {
      seen.opened.push(path);
      return actual.openRerailDatabaseAsIs(path);
    };
    return { ...actual, openRerailDatabase, openRerailDatabaseAsIs };
  });
  vi.doMock("../../src/engine/catalog/repository-identity.js", async () => {
    const actual = await vi.importActual<typeof import("../../src/engine/catalog/repository-identity.js")>("../../src/engine/catalog/repository-identity.js");
    const gitSearchEnds: typeof actual.gitSearchEnds = workspace => {
      seen.walked.push(workspace);
      return actual.gitSearchEnds(workspace);
    };
    return { ...actual, gitSearchEnds };
  });
  vi.doMock("../../src/runtime/repository-start.js", async () => {
    const actual = await vi.importActual<typeof import("../../src/runtime/repository-start.js")>("../../src/runtime/repository-start.js");
    const stoppedWithoutGit: typeof actual.stoppedWithoutGit = (workspace, databasePath) =>
      seen.proofs.run(() => proof === undefined ? actual.stoppedWithoutGit(workspace, databasePath) : proof(workspace));
    return { ...actual, stoppedWithoutGit };
  });
  const host = await import("../../src/harness/hook-host.js");
  const { startStateReader: reader } = await import("../../src/runtime/index.js");
  const daemonReads = (folder: string): string => {
    const database = openRerailDatabase(defaultRerailDatabasePath(), { readonly: true });
    try {
      return reader(database)(folder);
    } finally {
      database.$client.close();
    }
  };
  return { ...seen, hookRerailStarted: host.hookRerailStarted, daemonReads };
}

/**
 * A daemon recorded in the Rerail home that answers each start check only once the hook has run its own proof for it,
 * as a daemon busy with other checkouts answers after the hook's grace: the hook's own answer, where it gives one,
 * comes first, and anywhere else it takes the daemon's.
 */
async function daemonAnsweringAfterProof(host: StartCheckHost, repositoryState: string | ((repository: string) => string)): Promise<CountingDaemon> {
  return await countingDaemon(process.env.RERAIL_HOME!, repositoryState, () => host.proofs.ran());
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
  const found = { wrong: [] as string[], askedThoughStopped: [] as string[], answeredItselfIn: new Set<string>() };
  for (const folder of folders) {
    for (const rows of SPELLING_ROWS) {
      plantRows(folder, rows);
      const proofsBefore = host.proofs.found.length;
      host.proofs.nextCheck();
      const started = await host.hookRerailStarted("claude-code", folder.workspace);
      const reads = host.daemonReads(daemon.asked.at(-1)!);
      const answeredItself = host.proofs.found.length > proofsBefore && host.proofs.found.at(-1) === true;
      const name = `${folder.name}: stored ${String(rows.stored)}, typed ${String(rows.typed)}, repository ${String(rows.repository)}: ${reads}`;
      if (answeredItself ? reads !== "stopped" : started !== (reads !== "stopped")) found.wrong.push(name);
      if (!answeredItself && reads === "stopped") found.askedThoughStopped.push(name);
      if (answeredItself) found.answeredItselfIn.add(folder.name);
    }
  }
  return { ...found, answeredItselfIn: [...found.answeredItselfIn] };
}

/**
 * A grace for the start check that no answer of the daemon's misses, nor the empty read where none is recorded: a test
 * of what the hook does when the daemon answers in time gives it, so a slow or busy machine cannot make that answer late.
 */
const GRACE_NO_ANSWER_MISSES = 60_000;

/** The Git variables the test setup sets (`setup-rerail-home.ts`); a shell's others reach no test of the start check. */
const TEST_SETUP_GIT_VARIABLES = new Set(["GIT_CONFIG_GLOBAL", "GIT_CONFIG_NOSYSTEM"]);

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
    vi.doUnmock("../../src/runtime/repository-start.js");
    vi.doUnmock("../../src/engine/storage/database.js");
    vi.doUnmock("../../src/engine/catalog/repository-identity.js");
    vi.doUnmock("../../src/engine/catalog/checkout-control.js");
    vi.doUnmock("node:fs/promises");
    vi.resetModules();
    vi.unstubAllEnvs();
  });

  it.each(foldersOfEachKind)("asks the daemon first, and takes its prompt answer in $folder without opening the database or reading its folders", async ({ make, daemonSays, started }) => {
    const root = await temporaryDirectory({ started: false });
    const workspace = make(root);
    const host = await startCheckHost();
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, daemonSays);
    try {
      expect({
        started: await host.hookRerailStarted("claude-code", workspace, undefined, GRACE_NO_ANSWER_MISSES),
        requests: daemon.requests, opened: host.opened, walked: host.walked, proofs: host.proofs.found,
      }).toEqual({ started, requests: ["/api/presentation"], opened: [], walked: [], proofs: [] });
    } finally {
      await daemon.close();
    }
  });

  it("answers stopped itself beside a daemon that has not answered in time, where the database shows it would say stopped, running no Git and ending its request", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    mkdirSync(join(app, "src"));
    const host = await startCheckHost();
    // A daemon that would say started: an answer of stopped is the hook's own.
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "started", busyTheWholeTest);
    try {
      const started = await answeredWellBeforeReadLimit(host.hookRerailStarted("claude-code", join(app, "src")));
      expect({ started, git: host.git, proofs: host.proofs.found, ...await endedStartChecks(daemon) })
        .toEqual({ started: false, git: [], proofs: [true], asked: 1, ended: 1 });
    } finally {
      await daemon.close();
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

  it("waits for a late daemon where this process cannot open the database, and with no daemon recorded its failure is the hook's", async () => {
    const root = await temporaryDirectory({ started: false });
    const home = process.env.RERAIL_HOME!;
    const app = stoppedApp(root);
    const host = await startCheckHost();
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      await writeFile(join(home, "rerail.sqlite"), "not a database");
      expect({ started: await host.hookRerailStarted("claude-code", app), proofs: host.proofs.found, requests: daemon.requests })
        .toEqual({ started: true, proofs: [false], requests: ["/api/presentation"] });
    } finally {
      await daemon.close();
    }
    await rm(join(home, "daemon.json"));
    await expect(host.hookRerailStarted("claude-code", app)).rejects.toThrow("not a database");
  });

  it("waits for a late daemon where the database lacks one of this version's schema steps, and leaves the file as it is", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = stoppedApp(root);
    const step = newestStepNotRun();
    const host = await startCheckHost();
    // A daemon that would say started: an answer of stopped would be the hook's own.
    const daemon = await daemonAnsweringAfterProof(host, "started");
    try {
      expect({ started: await host.hookRerailStarted("claude-code", app), proofs: host.proofs.found, requests: daemon.requests, stepRun: stepRun(step) })
        .toEqual({ started: true, proofs: [false], requests: ["/api/presentation"], stepRun: false });
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

  it("never answers stopped itself where the daemon reads otherwise, in any folder of these trees", { timeout: 120_000 }, async () => {
    const root = await temporaryDirectory({ started: false });
    const folders = generatedTrees(root);
    const host = await startCheckHost();
    // A daemon asked about a folder's real path, as the hook sends it, answers as a live one reads it, late: only after
    // the hook's own proof, so the hook answers itself wherever that proof shows stopped.
    const daemon = await daemonAnsweringAfterProof(host, daemonReads);
    const wrong: string[] = [];
    const askedThoughStopped: string[] = [];
    try {
      // Then again with a daemon whose Git stops right above each folder: a ceiling, as for a person whose
      // `safe.directory` refuses the checkout or whose folder is on another filesystem.
      for (const ceiling of [false, true]) {
        for (const [name, path] of folders) {
          const real = realpathSync.native(path);
          vi.stubEnv("GIT_CEILING_DIRECTORIES", ceiling ? dirname(real) : undefined);
          const reads = daemonReads(real);
          const proofsBefore = host.proofs.found.length;
          host.proofs.nextCheck();
          const started = await host.hookRerailStarted("claude-code", path);
          const answeredItself = host.proofs.found.length > proofsBefore && host.proofs.found.at(-1) === true;
          if (answeredItself ? reads !== "stopped" : started !== (reads !== "stopped")) wrong.push(`${name}${ceiling ? " (ceiling)" : ""}: ${reads}`);
          if (!answeredItself && reads === "stopped" && !ceiling) askedThoughStopped.push(name);
        }
      }
    } finally {
      await daemon.close();
    }
    expect({ wrong, askedThoughStopped }).toEqual({ wrong: [], askedThoughStopped: ["b/own-folder", "never", "never/src"] });
  });

  it("asks about a folder typed in another letter case by its stored spelling and answers itself only where the daemon reads stopped, with macOS's realpaths", { timeout: 120_000 }, async () => {
    const root = realpathSync.native(await temporaryDirectory({ started: false }));
    const folders = miscasedFolders(root);
    // On macOS checkout keys go through the JS realpath, which keeps a name as typed; the native realpath, which the
    // hook asks the daemon with, gives it as stored.
    const host = await startCheckHost(undefined, { system: "macos", folders });
    const daemon = await daemonAnsweringAfterProof(host, repository => host.daemonReads(repository));
    try {
      const found = await startChecksTypedOtherwise(host, daemon, folders);
      expect({ ...found, asked: [...new Set(daemon.asked)] }).toEqual({
        wrong: [], askedThoughStopped: [], answeredItselfIn: folders.map(folder => folder.name), asked: folders.map(folder => folder.storedWorkspace),
      });
    } finally {
      await daemon.close();
    }
  });

  it.runIf(process.platform === "linux")("waits for the daemon where Git names a checkout's top in another letter case than the hook's realpath, as on a case-insensitive Linux folder", { timeout: 120_000 }, async () => {
    const root = realpathSync.native(await temporaryDirectory({ started: false }));
    const folders = miscasedFolders(root);
    // There every realpath keeps a name as typed, and Git's `getcwd` gives the name the kernel cached.
    const host = await startCheckHost(undefined, { system: "linux-case-insensitive", folders });
    const daemon = await daemonAnsweringAfterProof(host, repository => host.daemonReads(repository));
    try {
      const found = await startChecksTypedOtherwise(host, daemon, folders);
      expect({ wrong: found.wrong, answeredItselfIn: found.answeredItselfIn, asked: [...new Set(daemon.asked)] })
        .toEqual({ wrong: [], answeredItselfIn: ["outside Git"], asked: folders.map(folder => folder.workspace) });
    } finally {
      await daemon.close();
    }
  });
});
