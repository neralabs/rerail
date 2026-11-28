import { createHash, randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { appendFile, mkdir, readdir, readFile, rename, rm, symlink, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { basename, dirname, join, resolve } from "node:path";
import { Readable, Writable } from "node:stream";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";
import { type AdapterDelivery, sessionStartNote } from "../../src/engine/index.js";
import { ENGINE_CHALLENGE_HEADER, ENGINE_PROOF_PATH, encodeRerailEventResult, engineProof, type DaemonMachine, type EngineEndpoint } from "../../src/runtime/index.js";
import { defaultEngineSocketPath } from "../../src/daemon/index.js";
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
import type { HookOptions } from "../../src/harness/hook-host.js";
import { HOOK_FAILURES_FILE } from "../../src/harness/hook-failures.js";
import { runClaudeHook } from "../../src/hosts/claude/index.js";
import { runCodexHook } from "../../src/hosts/codex/index.js";
import { runCursorHook } from "../../src/hosts/cursor/index.js";
import { hookOptions, serveText, temporaryDirectories, sessionDatabaseQuery } from "../support/hook-events.js";
import { fakeMachine } from "../support/platform/daemon-machine.js";
import { DEAD_PID } from "../support/processes/daemon-home.js";

// The lookup of a named engine socket, a session start's lookup of the daemon, and a later hook's lookup of the running
// daemon work as they do; a test stands in for one call of any of them at a time.
const lookups = vi.hoisted(() => ({
  engineEndpointAt: vi.fn<(socketPath: string, deadline?: number) => Promise<EngineEndpoint>>(),
  daemonEngineEndpoint: vi.fn<(replaceOtherPackage: boolean, deadline?: number) => Promise<EngineEndpoint>>(),
  runningDaemonEngineEndpoint: vi.fn<(deadline?: number) => Promise<unknown>>(),
}));
vi.mock("../../src/runtime/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
  lookups.engineEndpointAt.mockImplementation((socketPath, deadline) => actual.engineEndpointAt(socketPath, deadline));
  lookups.daemonEngineEndpoint.mockImplementation((replace, deadline) => actual.daemonEngineEndpoint(replace, deadline));
  lookups.runningDaemonEngineEndpoint.mockImplementation(deadline => actual.runningDaemonEngineEndpoint(deadline));
  return {
    ...actual,
    engineEndpointAt: lookups.engineEndpointAt,
    daemonEngineEndpoint: lookups.daemonEngineEndpoint,
    runningDaemonEngineEndpoint: lookups.runningDaemonEngineEndpoint,
  };
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

/** The capability the held engine holds, which daemon.json records for its socket. */
const HELD_CAPABILITY = "held-engine-capability";

/**
 * A daemon's engine socket that answers the first event only once `answerFirst` is called, as a busy daemon checking an
 * edit does, and every later event at once. daemon.json records it with its capability until it closes, as the daemon
 * records itself: this process's pid and a start stamped after the process began, so Windows names the recorded daemon
 * as the pipe's holder. It proves it holds that capability, as a hook's start check asks.
 */
async function heldEngine(root: string): Promise<HeldEngine> {
  const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-held-engine-${randomUUID()}` : join(root, "engine.sock");
  const received: string[] = [];
  const firstArrived = Promise.withResolvers<void>();
  const first = Promise.withResolvers<void>();
  const server = createServer(async (request, response) => {
    if (request.url === ENGINE_PROOF_PATH) {
      request.resume();
      response.end(JSON.stringify({ proof: engineProof(HELD_CAPABILITY, String(request.headers[ENGINE_CHALLENGE_HEADER])) }));
      return;
    }
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
  const stateFile = join(process.env.RERAIL_HOME!, "daemon.json");
  await mkdir(process.env.RERAIL_HOME!, { recursive: true });
  await writeFile(stateFile, JSON.stringify({ pid: process.pid, host: "127.0.0.1", port: 1, url: "http://127.0.0.1:1/", version: "test",
    databasePath: join(root, "rerail.sqlite"), kernelSocketPath: socketPath, ui: "bundled", startedAt: Date.now(), packageRoot: root, accessToken: HELD_CAPABILITY }));
  return {
    socketPath,
    received,
    firstArrived: firstArrived.promise,
    answerFirst: () => first.resolve(),
    close: async () => {
      await rm(stateFile, { force: true });
      await new Promise<void>(resolve => server.close(() => resolve()));
    },
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
    const conflict = { id: "conflict:overlap:abc:12345678", text: "your edit overlaps jdoe-1: retry.ts" };
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
    const conflict = { id: "conflict:overlap:abc:12345678", text: "your edit overlaps jdoe-1: retry.ts" };
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

  it("looks up the engine socket the hook is given with the hook's deadline, so a proof its start check had since the event arrived answers for it", async () => {
    const root = await temporaryDirectory();
    const deadlines: number[] = [];
    // The hook ends at its lookup, which is all this case reads: the deadline it hands the lookup.
    lookups.engineEndpointAt.mockImplementationOnce(async (_socketPath, deadline) => {
      deadlines.push(deadline ?? Number.NaN);
      throw new Error("the lookup this case stands in for");
    });
    const { embedder: _embedder, ...options } = hookOptions(root);
    const before = Date.now();
    await expect(runClaudeHook({ session_id: "s", cwd: root, hook_event_name: "UserPromptSubmit", prompt_id: "p", prompt: "go" }, { ...options, kernelSocketPath: join(root, "named.sock") }))
      .rejects.toThrow("the lookup this case stands in for");
    expect(deadlines).toHaveLength(1);
    // The hook's deadline: 12 s from the event's arrival, the start check's read of the daemon included.
    expect(deadlines[0]).toBeGreaterThanOrEqual(before + 12_000);
    expect(deadlines[0]).toBeLessThanOrEqual(Date.now() + 12_000);
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

describe("a session's start where RERAIL_AUTOSTART keeps hooks from starting or replacing a daemon", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
    vi.unstubAllGlobals();
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

  const keptLine = (setting: string) =>
    `The running Rerail daemon is from another Rerail package or an earlier release, and RERAIL_AUTOSTART=${setting} keeps hooks from replacing it, so nothing is recorded until a daemon from the active package runs. Run rerail daemon stop, then rerail daemon start.`;
  /** This checkout: the package the hooks under test run from. */
  const OWN_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  const PIPE = defaultEngineSocketPath(String.raw`C:\Users\me\.rerail\rerail.sqlite`, "win32");
  /** The engine pipe as `\\.\pipe\` lists it. */
  const PIPE_NAME = PIPE.slice(PIPE.lastIndexOf("\\") + 1);
  /** When the kept daemon stamped `startedAt`; its process started 0.6 s before. */
  const STARTED_AT = 1_700_000_000_000;
  // A daemon of Rerail 0.1.4 or earlier, whose engine has no proof, launched from the hooks' own package as an upgrade
  // in place leaves its record; and a daemon of another install, whose engine proves itself.
  const keptDaemons = [
    { daemon: "of Rerail 0.1.4 or earlier from the hooks' own package", version: "0.1.4", packageRoot: OWN_ROOT, proof: "no-proof" },
    { daemon: "of another install", version: "0.1.6", packageRoot: String.raw`C:\Users\me\AppData\Roaming\npm\node_modules\@neralabs\rerail`, proof: "proved" },
  ] as const;

  /**
   * A Windows machine with RERAIL_AUTOSTART set to `setting` whose daemon.json, in this test's Rerail home, names `kept`
   * running; each request to its URL goes to `asked`.
   */
  async function besideKeptDaemon(kept: (typeof keptDaemons)[number], setting: string, asked: string[]): Promise<DaemonMachine> {
    const home = process.env.RERAIL_HOME!;
    await mkdir(home, { recursive: true });
    const state = {
      pid: DEAD_PID, host: "127.0.0.1", port: 1, url: "http://127.0.0.1:1/", version: kept.version, databasePath: join(home, "rerail.sqlite"),
      kernelSocketPath: PIPE, ui: "dev", startedAt: STARTED_AT, packageRoot: kept.packageRoot, accessToken: "capability", nativeToken: "native",
    };
    await writeFile(join(home, "daemon.json"), JSON.stringify(state));
    vi.stubGlobal("fetch", vi.fn(async (url: URL) => {
      const path = new URL(url).pathname;
      asked.push(path);
      return path === "/healthz" ? Response.json({ status: "ok", pid: DEAD_PID }) : new Response(null, { status: 404 });
    }));
    return fakeMachine("win32", {
      env: { RERAIL_HOME: home, RERAIL_AUTOSTART: setting },
      kill: pid => {
        if (pid !== DEAD_PID) throw Object.assign(new Error("kill ESRCH"), { code: "ESRCH" });
        return true;
      },
      pipes: () => [PIPE_NAME],
      capabilityProof: async () => kept.proof,
      processStartedAt: pid => (pid === DEAD_PID ? STARTED_AT - 600 : undefined),
    });
  }

  // A daemon runs that the hooks may not send events to: one of Rerail 0.1.4 or earlier predates the fields they send
  // (told apart on Windows), and one of another install is not the active package's. RERAIL_AUTOSTART=0 keeps the hooks
  // from replacing it, so nothing is recorded while it runs. The session's start says so where the person reads it, with
  // the stop before the start, since a start beside a running daemon answers that one already runs; each lost event is
  // a failure `rerail status` names. The line names the setting as the person set it.
  it.each(sessionStarts.flatMap(start => keptDaemons.flatMap(kept => ["0", "false"].map(setting => ({ ...start, kept, setting })))))(
    "tells the person in $host that nothing is recorded beside a daemon $kept.daemon that the hooks may not replace, with RERAIL_AUTOSTART=$setting",
    async ({ run, input, told, kept, setting }) => {
      const root = await temporaryDirectory();
      vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
      vi.stubEnv("RERAIL_AUTOSTART", setting);
      const asked: string[] = [];
      const machine = await besideKeptDaemon(kept, setting, asked);
      const { daemonEngineEndpoint } = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
      lookups.daemonEngineEndpoint.mockClear().mockImplementationOnce((replace, deadline) => daemonEngineEndpoint(replace, deadline, machine));
      const { embedder: _testRuntime, ...daemonPath } = hookOptions(root);
      const served = await serveText(parsed => run(parsed, daemonPath), JSON.stringify(input(root)));
      expect(served).toEqual({ stdout: `${JSON.stringify(told(keptLine(setting)))}\n`, stderr: "" });
      expect(lookups.daemonEngineEndpoint).toHaveBeenCalledTimes(1);
      // The kept daemon is sent nothing, its record stays for `rerail daemon stop`, and the lost event is logged.
      expect(asked.filter(path => path !== "/healthz")).toEqual([]);
      expect(existsSync(join(process.env.RERAIL_HOME!, "daemon.json"))).toBe(true);
      expect(existsSync(join(process.env.RERAIL_HOME!, HOOK_FAILURES_FILE))).toBe(true);
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
    const { AutostartOffError } = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
    const reason = "RERAIL_AUTOSTART=0 requires a running daemon; run rerail daemon start or set RERAIL_ENGINE_SOCKET";
    lookups.runningDaemonEngineEndpoint.mockClear().mockRejectedValueOnce(new AutostartOffError(reason));
    const { embedder: _testRuntime, ...daemonPath } = hookOptions(root);
    expect(await serveText(parsed => run(parsed, daemonPath), JSON.stringify(input(root)))).toEqual({ stdout: "{}\n", stderr: `rerail hook: ${reason}\n` });
    expect(lookups.runningDaemonEngineEndpoint).toHaveBeenCalledTimes(1);
    expect(existsSync(join(process.env.RERAIL_HOME!, HOOK_FAILURES_FILE))).toBe(false);
  });

  // Beside a daemon it may neither use nor replace, a later hook answers as any failed hook does and logs the event it
  // lost, which `rerail status` names: only the session's start tells the person, once.
  it.each(laterHooks)("answers a later $host hook beside a daemon it may not replace as any failed hook, and logs it", async ({ run, input }) => {
    const root = await temporaryDirectory();
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    vi.stubEnv("RERAIL_AUTOSTART", "0");
    const { OtherDaemonKeptError } = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
    const reason = "RERAIL_AUTOSTART=0 requires a daemon from the active Rerail package; run rerail daemon stop, then rerail daemon start, or set RERAIL_ENGINE_SOCKET";
    lookups.runningDaemonEngineEndpoint.mockClear().mockRejectedValueOnce(new OtherDaemonKeptError(reason));
    const { embedder: _testRuntime, ...daemonPath } = hookOptions(root);
    expect(await serveText(parsed => run(parsed, daemonPath), JSON.stringify(input(root)))).toEqual({ stdout: "{}\n", stderr: `rerail hook: ${reason}\n` });
    expect(existsSync(join(process.env.RERAIL_HOME!, HOOK_FAILURES_FILE))).toBe(true);
  });
});
