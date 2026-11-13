import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { appendFile, mkdir, readdir, readFile, rename, rm, symlink, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { basename, join } from "node:path";
import { Readable, Writable } from "node:stream";
import { setTimeout as delay } from "node:timers/promises";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { type AdapterDelivery, sessionStartNote } from "../../src/engine/index.js";
import { encodeRerailEventResult, type EngineEndpoint, workspaceStartState } from "../../src/runtime/index.js";
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
    await persistPending(pendingPath(options.storageRoot!, "claude-code", "session-1"), [
      { ...delivery("guidance"), requested: "steer" },
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

/**
 * A daemon recorded in the Rerail home `home` that answers as a live one does and counts every request it gets. On
 * Windows a recorded daemon counts as alive only while its engine pipe exists (`recordedDaemonAlive`), so it holds one.
 */
async function countingDaemon(home: string, repositoryState = "stopped"): Promise<{ requests: string[]; close: () => Promise<void> }> {
  const requests: string[] = [];
  const answers: Record<string, unknown> = {
    "/healthz": { status: "ok", pid: process.pid, database: { path: join(home, "rerail.sqlite"), exists: true }, native: { ok: true } },
    "/api/presentation": { presence: "compact", repositoryState },
  };
  const server = createServer((request, response) => {
    const path = new URL(request.url ?? "/", "http://127.0.0.1").pathname;
    requests.push(path);
    response.writeHead(Object.hasOwn(answers, path) ? 200 : 404, { "content-type": "application/json" });
    response.end(JSON.stringify(answers[path] ?? { error: "not-found" }));
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
    close: async () => {
      await releaseEndpoint();
      await new Promise<void>(resolve => server.close(() => resolve()));
    },
  };
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

/** What coding agents and editors set for the commands they run, Git's included; none of it moves where Git looks. */
const AGENT_GIT_ENVIRONMENT = { GIT_ASKPASS: "askpass", GIT_EDITOR: "true", GIT_PAGER: "cat", GIT_TERMINAL_PROMPT: "0" };

/**
 * A coding agent's environment with no `GIT_*` variable but `AGENT_GIT_ENVIRONMENT`'s. Any other sends the hook's start
 * check to the daemon: the test setup's `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_NOSYSTEM`, and whatever the shell running
 * the tests sets (a Git hook sets `GIT_DIR`). Git then reads the test home's `.gitconfig`, the file `GIT_CONFIG_GLOBAL`
 * named, and the machine's system file, which none of these cases depends on.
 */
function stubAgentGitEnvironment(): void {
  for (const name of Object.keys(process.env)) {
    if (name.toUpperCase().startsWith("GIT_")) vi.stubEnv(name, undefined);
  }
  for (const [name, value] of Object.entries(AGENT_GIT_ENVIRONMENT)) vi.stubEnv(name, value);
}

describe("hooks where Rerail does not run", () => {
  beforeEach(() => {
    // The hook takes CLAUDE_PROJECT_DIR over the event's folder; a test run from inside a Claude Code session has one.
    vi.stubEnv("CLAUDE_PROJECT_DIR", undefined);
    stubAgentGitEnvironment();
  });
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it.each(hooksWhereRerailIsOff)("answer $host's $event in a stopped checkout without a request to the daemon", async ({ run }) => {
    const root = await temporaryDirectory({ started: false });
    startTestRepository(root);
    stopTestRepository(root);
    const daemon = await countingDaemon(process.env.RERAIL_HOME!);
    try {
      expect(await run(root)).toEqual({});
      expect(daemon.requests).toEqual([]);
    } finally {
      await daemon.close();
    }
  });

  it.each(hooksWhereRerailIsOff)("answer $host's $event in a folder Rerail was never started in without a request to the daemon", async ({ run }) => {
    const root = await temporaryDirectory({ started: false });
    // Rerail runs in another repository on the machine, so the database is there.
    const elsewhere = join(root, "elsewhere");
    const never = join(root, "never");
    await mkdir(elsewhere);
    await mkdir(never);
    startTestRepository(elsewhere);
    const daemon = await countingDaemon(process.env.RERAIL_HOME!);
    try {
      expect(await run(never)).toEqual({});
      expect(daemon.requests).toEqual([]);
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

/** A new clone at `<root>/clone` of `startedApp`'s repository, which Rerail finds by its remote alone. */
function newClone(root: string): string {
  const clone = gitRepository(join(root, "clone"));
  git(clone, "remote", "add", "origin", REMOTE);
  return clone;
}

/**
 * Folders of a started repository at `<root>/app` that Rerail finds without a row of their own, each through a Git read:
 * the top level `rev-parse` gives, the clone id in the common Git folder it names, or the remote `ls-remote` gives.
 */
const foldersFoundThroughGit = [
  { shape: "a folder inside the checkout", silent: ["rev-parse"], make: (app: string) => { mkdirSync(join(app, "src")); return join(app, "src"); } },
  {
    shape: "a linked worktree Rerail never recorded", silent: ["rev-parse"],
    make: (app: string, root: string) => { git(app, "worktree", "add", "-q", join(root, "worktree")); return join(root, "worktree"); },
  },
  { shape: "a new clone of the repository", silent: ["ls-remote", "remote"], make: (_app: string, root: string) => newClone(root) },
];

/** A Git config file at `path` that gives every address under https://example.invalid/ another, as a mirror setting does. */
function remoteRewrite(path: string): string {
  writeFileSync(path, "[url \"https://mirror.invalid/\"]\n\tinsteadOf = https://example.invalid/\n");
  return path;
}

/** A started repository and a new clone of it under `root`, with a config file named `file` that rewrites the remote. */
function cloneWithRewrite(file: string): (root: string) => { workspace: string; value: string } {
  return root => {
    startedApp(root);
    return { workspace: newClone(root), value: remoteRewrite(join(root, file)) };
  };
}

/**
 * A person's own Git settings, each set in the coding agent's environment and not in the daemon's, with a folder of a
 * started repository that this process's Git then reads otherwise: a ceiling at the checkout's top hides the checkout
 * from a folder inside it; a global or system file that rewrites the remote gives a new clone another address; and
 * leaving out the system file the daemon's Git reads leaves the clone the address the repository was not recorded under.
 * A test cannot write the machine's system file, so `GIT_CONFIG_SYSTEM` stands in for it while the repository is
 * recorded.
 */
const personalGitSettings: Array<{ variable: string; arrange: (root: string) => { workspace: string; value: string } }> = [
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
      return { workspace: newClone(root), value: "1" };
    },
  },
];

/** How spawnSync reports a Git that gave no answer: one it ended past runGit's 10 s, and one another process ended on Windows. */
const UNANSWERED_GIT = {
  "timed out": { status: null, stdout: "", stderr: "", error: Object.assign(new Error("spawnSync git ETIMEDOUT"), { code: "ETIMEDOUT" }) },
  "ended by another process on Windows": { status: 1, stdout: "", stderr: "" },
};

/** Each folder found through Git, with each way Git gives no answer. */
const unansweredFolders = foldersFoundThroughGit.flatMap(folder =>
  (Object.keys(UNANSWERED_GIT) as Array<keyof typeof UNANSWERED_GIT>).map(how => ({ ...folder, how })));

/**
 * The hook host over a Git that starts and gives no answer to each command named in `silent`, reported as `unanswered`
 * (by default a Git spawnSync ended past runGit's 10 s: no status and an error), where `gitText` gives nothing.
 */
async function hookHostWithSilentGit(
  silent: readonly string[],
  unanswered: (typeof UNANSWERED_GIT)[keyof typeof UNANSWERED_GIT] = UNANSWERED_GIT["timed out"],
): Promise<typeof import("../../src/harness/hook-host.js")> {
  vi.resetModules();
  vi.doMock("../../src/engine/git.js", async () => {
    const actual = await vi.importActual<typeof import("../../src/engine/git.js")>("../../src/engine/git.js");
    const answers = (args: readonly string[]): boolean => !silent.includes(args[0] ?? "");
    return {
      ...actual,
      runGit: (cwd: string, args: readonly string[], options?: Parameters<typeof actual.runGit>[2]) => answers(args) ? actual.runGit(cwd, args, options) : unanswered,
      gitText: (cwd: string, args: readonly string[]) => answers(args) ? actual.gitText(cwd, args) : undefined,
    };
  });
  return await import("../../src/harness/hook-host.js");
}

describe("the hook start check", () => {
  beforeEach(() => {
    stubAgentGitEnvironment();
  });
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("reads the database, and asks a live daemon only where this process cannot read it", async () => {
    const root = await temporaryDirectory({ started: false });
    const home = process.env.RERAIL_HOME!;
    startTestRepository(root);
    // The database says started; the daemon, which would say stopped, is not asked.
    const daemon = await countingDaemon(home, "stopped");
    try {
      expect(await hookRerailStarted("claude-code", root)).toBe(true);
      expect(daemon.requests).toEqual([]);
      // A database this process cannot read: the live daemon answers.
      await writeFile(join(home, "rerail.sqlite"), "not a database");
      expect(await hookRerailStarted("claude-code", root)).toBe(false);
      expect(daemon.requests).toEqual(["/api/presentation"]);
    } finally {
      await daemon.close();
    }
    // Neither answers: the database is read again, and its failure is the hook's.
    await rm(join(home, "daemon.json"));
    await expect(hookRerailStarted("claude-code", root)).rejects.toThrow("not a database");
  });

  it("asks a live daemon where this process finds no Git to resolve the checkout with", async () => {
    const root = await temporaryDirectory({ started: false });
    const elsewhere = join(root, "elsewhere");
    await mkdir(elsewhere);
    startTestRepository(elsewhere);
    // The daemon, with a Git of its own, reads the checkout as started; this process would read it as unknown.
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "started");
    vi.resetModules();
    vi.doMock("../../src/engine/git.js", async () => {
      const git = await vi.importActual<typeof import("../../src/engine/git.js")>("../../src/engine/git.js");
      // As where no Git is installed: every way of reaching it fails.
      const missing = (): never => { throw new git.GitUnavailableError([]); };
      return { ...git, gitExecutable: missing, runGit: missing, gitText: missing };
    });
    try {
      const { hookRerailStarted } = await import("../../src/harness/hook-host.js");
      expect(await hookRerailStarted("claude-code", root)).toBe(true);
      expect(daemon.requests).toEqual(["/api/presentation"]);
    } finally {
      vi.doUnmock("../../src/engine/git.js");
      vi.resetModules();
      await daemon.close();
    }
  });

  it.each(unansweredFolders)("asks a live daemon where this process's Git gives no answer ($how), in $shape", async ({ silent, make, how }) => {
    const root = await temporaryDirectory({ started: false });
    const workspace = make(startedApp(root), root);
    // The daemon keeps each folder's Git location for its life, and reads the folder as started.
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "started");
    try {
      // Git answering, the hook finds the folder started itself.
      expect(await hookRerailStarted("claude-code", workspace)).toBe(true);
      expect(daemon.requests).toEqual([]);
      const silenced = await hookHostWithSilentGit(silent, UNANSWERED_GIT[how]);
      expect({ started: await silenced.hookRerailStarted("claude-code", workspace), requests: daemon.requests })
        .toEqual({ started: true, requests: ["/api/presentation"] });
    } finally {
      vi.doUnmock("../../src/engine/git.js");
      vi.resetModules();
      await daemon.close();
    }
  });

  it("asks a live daemon where the coding agent's environment sends Git to another repository", async () => {
    const root = await temporaryDirectory({ started: false });
    const app = gitRepository(join(root, "app"));
    const other = gitRepository(join(root, "other"));
    startTestRepository(app);
    await mkdir(join(app, "src"));
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "started");
    // The agent's Git, and this process's, would read `other` for every folder.
    vi.stubEnv("GIT_DIR", join(other, ".git"));
    try {
      expect({ started: await hookRerailStarted("claude-code", join(app, "src")), requests: daemon.requests })
        .toEqual({ started: true, requests: ["/api/presentation"] });
    } finally {
      vi.unstubAllEnvs();
      await daemon.close();
    }
  });

  it.each(personalGitSettings)("asks a live daemon where the coding agent's environment sets $variable", async ({ variable, arrange }) => {
    const root = await temporaryDirectory({ started: false });
    const { workspace, value } = arrange(root);
    // The daemon's Git, started from another environment, reads the folder as started.
    const daemon = await countingDaemon(process.env.RERAIL_HOME!, "started");
    vi.stubEnv(variable, value);
    try {
      // This process's own read of the database, with the variable set, finds the folder stopped.
      expect(workspaceStartState(workspace)).toBe("stopped");
      expect({ started: await hookRerailStarted("claude-code", workspace), requests: daemon.requests })
        .toEqual({ started: true, requests: ["/api/presentation"] });
    } finally {
      await daemon.close();
    }
  });
});
