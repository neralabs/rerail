import { mkdir, readFile, writeFile } from "node:fs/promises";
import { createServer as createHttpServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { z } from "zod";
import { type AdapterDelivery, defaultRerailDatabasePath, type RerailEventLoop, sessionStartNote } from "../../../src/engine/index.js";
import { claudeHookOutput, handleClaudeEvent } from "../../../src/hosts/claude/claude-hook-adapter.js";
import { runClaudeHook } from "../../../src/hosts/claude/claude-hook.js";
import { bindSteerReceipt, type HookInput, type HookOptions } from "../../../src/harness/index.js";
import { dashboardOrigin, presentedSessionNoteRules } from "../../../src/presentation/index.js";
import { hookOptions, scriptedLoop, serveText, temporaryDirectories, sessionDatabaseQuery, eventRows } from "../../support/hook-events.js";
import { startTestRepository, stopTestRepository } from "../../support/started-repository.js";
import { editSteer, steeredRules } from "../../support/steers.js";
import { closeNativeTestDaemons, startNativeTestDaemon } from "../../support/native-daemon.js";
import { testEngineEndpoint } from "../../support/platform/engine-endpoint.js";

const temporaryDirectory = temporaryDirectories("rerail-claude-hook-");

/**
 * `hookOptions` without the test's embedder, so a hook takes the daemon path a coding agent's hook takes, and, unless
 * `keepDatabase`, without the test's database, so the hook reads the one the daemon serves.
 */
function daemonHookOptions(root: string, { keepDatabase }: { keepDatabase: boolean }): HookOptions {
  const options = hookOptions(root);
  delete options.embedder;
  if (!keepDatabase) delete options.databasePath;
  return options;
}

function delivery(requested: AdapterDelivery["requested"], content: string, ruleIds: readonly string[] = [], rules = steeredRules(ruleIds)): AdapterDelivery {
  return { sessionId: "claude-session", requested, mechanism: "additional-context", content, ruleIds, rules };
}

/** Serves hook events through the harness entry as the hook process does, collecting each steer receipt's rule ids. */
function receiptServer(storageRoot: string) {
  const receipts: Array<readonly string[]> = [];
  const serve = async (input: HookInput, loop: RerailEventLoop) => JSON.parse((await serveText(
    parsed => handleClaudeEvent(parsed, loop, storageRoot, { NO_COLOR: "1" }).then(output => bindSteerReceipt(output, (_receipt, ruleIds) => { receipts.push(ruleIds); })),
    JSON.stringify(input),
  )).stdout) as unknown;
  return { receipts, serve };
}

describe("Claude Code hook without a daemon", () => {
  it("answers {} at once after the session's start, without starting a daemon, so no tool call shows a hook error", async () => {
    const root = await temporaryDirectory();
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    try {
      // The test runtime's embedder is left out, so the hook takes the daemon path a coding agent's hook takes.
      const daemonPath = daemonHookOptions(root, { keepDatabase: true });
      const output = await runClaudeHook({
        session_id: "s", cwd: root, hook_event_name: "PostToolUse", tool_name: "Write", tool_use_id: "t", tool_input: {}, tool_response: "ok",
      }, daemonPath);
      expect(output).toEqual({});
      await expect(readFile(join(process.env.RERAIL_HOME!, "daemon.json"))).rejects.toMatchObject({ code: "ENOENT" });
    } finally {
      vi.unstubAllEnvs();
    }
  });
});

describe("Claude Code hook with a daemon that misses its health probe", () => {
  const servers: Server[] = [];
  afterEach(async () => {
    vi.unstubAllEnvs();
    await Promise.all(servers.splice(0).map(server => {
      server.closeAllConnections();
      return new Promise(resolve => server.close(resolve));
    }));
    await closeNativeTestDaemons();
  });

  /** A listener at a new loopback URL that answers each request as `upstream` does, `delayMs` late: a busy daemon. */
  async function slowFront(upstream: string, delayMs: number): Promise<string> {
    const server = createHttpServer((request, response) => {
      setTimeout(() => {
        const headers = Object.fromEntries(Object.entries(request.headers).filter((entry): entry is [string, string] => typeof entry[1] === "string"));
        fetch(new URL(request.url ?? "/", upstream), { headers }).then(async answer => {
          response.writeHead(answer.status, { "content-type": answer.headers.get("content-type") ?? "application/json" });
          response.end(await answer.text());
        }, () => response.destroy());
      }, delayMs);
    });
    servers.push(server);
    await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
    return `http://127.0.0.1:${(server.address() as AddressInfo).port}/`;
  }

  /** Rewrites the home's daemon.json with `change` applied; returns the state as the daemon wrote it. */
  async function rewriteState(change: (state: Record<string, unknown>) => Record<string, unknown>): Promise<Record<string, unknown>> {
    const stateFile = join(process.env.RERAIL_HOME!, "daemon.json");
    const state = JSON.parse(await readFile(stateFile, "utf8")) as Record<string, unknown>;
    await writeFile(stateFile, JSON.stringify(change(state)));
    return state;
  }

  function eventIds(): string[] {
    const database = new Database(defaultRerailDatabasePath(), { readonly: true });
    try {
      return database.prepare<[], { eventId: string }>("SELECT event_id AS eventId FROM events ORDER BY sequence").all().map(row => row.eventId);
    } finally {
      database.close();
    }
  }

  // RERAIL_AUTOSTART=1 is a person's session, where the session-start hook may start a daemon; 0 starts none.
  it.each(["1", "0"])("waits for a busy daemon to answer as itself, then records the session (RERAIL_AUTOSTART=%s)", { timeout: 60_000 }, async autostart => {
    const root = await temporaryDirectory();
    startTestRepository(root);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    vi.stubEnv("RERAIL_AUTOSTART", autostart);
    const daemon = await startNativeTestDaemon(root);
    // The daemon answers 3 s late, past the first probe's 2 s, as one does while an audit holds its event loop.
    const front = await slowFront(daemon.url, 3_000);
    await rewriteState(state => ({ ...state, url: front, port: Number(new URL(front).port) }));
    const daemonPath = daemonHookOptions(root, { keepDatabase: false });
    const common = { session_id: "busy-session", cwd: root };
    await runClaudeHook({ ...common, hook_event_name: "SessionStart", source: "startup" }, daemonPath);
    await runClaudeHook({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p1", prompt: "hi" }, daemonPath);
    expect(eventIds()).toEqual(expect.arrayContaining(["busy-session:SessionStart", "busy-session:prompt:p1"]));
  });

  it("sends no capability to an engine socket whose daemon never answers as itself, so a squatter gets none", { timeout: 60_000 }, async () => {
    const root = await temporaryDirectory();
    startTestRepository(root);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    vi.stubEnv("RERAIL_AUTOSTART", "0");
    await startNativeTestDaemon(root);
    // Something else listens where the state file says the engine is, and nothing answers at the recorded URL.
    const received: string[] = [];
    const squatter = createHttpServer((request, response) => {
      received.push(`${request.method} ${request.url} ${String(request.headers["x-rerail-access"] ?? "no capability")}`);
      response.writeHead(500).end();
    });
    servers.push(squatter);
    const squatted = process.platform === "win32" ? `\\\\.\\pipe\\rerail-squatted-${process.pid}-${Date.now()}` : join(root, "squatted.sock");
    await new Promise<void>(resolve => squatter.listen(squatted, resolve));
    await rewriteState(state => ({ ...state, url: "http://127.0.0.1:1/", port: 1, kernelSocketPath: squatted }));
    const daemonPath = daemonHookOptions(root, { keepDatabase: false });
    const common = { session_id: "squatted-session", cwd: root };
    const start = await runClaudeHook({ ...common, hook_event_name: "SessionStart", source: "startup" }, daemonPath)
      .catch((error: unknown) => error);
    // Neither start records the session. Elsewhere the recorded pid is alive and its URL never answers, so the start
    // fails; on Windows no engine on the recorded pipe proves itself, so no daemon runs, and the start says that
    // nothing is recorded, as it does for any start RERAIL_AUTOSTART keeps without a daemon.
    if (process.platform === "win32") expect(start).toEqual({ systemMessage: expect.stringContaining("so nothing is recorded") });
    else expect(start).toBeInstanceOf(Error);
    expect(await runClaudeHook({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p1", prompt: "hi" }, daemonPath)).toEqual({});
    // On Windows the hook asks the engine there to prove it holds the capability, which sends only a challenge.
    expect(received.filter(line => process.platform !== "win32" || line !== "POST /proof no capability")).toEqual([]);
  });

  /** A listener at a new loopback URL that takes every request and answers none: a daemon whose event loop is held. */
  async function silentFront(): Promise<string> {
    const server = createHttpServer(() => undefined);
    servers.push(server);
    await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
    return `http://127.0.0.1:${(server.address() as AddressInfo).port}/`;
  }

  it("waits out a silent daemon once for a session's concurrent hooks, not once per hook in turn", { timeout: 90_000 }, async () => {
    const root = await temporaryDirectory();
    startTestRepository(root);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    await startNativeTestDaemon(root);
    const daemonPath = daemonHookOptions(root, { keepDatabase: false });
    const common = { session_id: "parallel-session", cwd: root };
    await runClaudeHook({ ...common, hook_event_name: "SessionStart", source: "startup" }, daemonPath);
    // Parallel tool calls and subagents send one session's PreToolUse hooks at once, while a walk holds the daemon.
    const front = await silentFront();
    await rewriteState(state => ({ ...state, url: front, port: Number(new URL(front).port) }));
    const started = performance.now();
    const toolCall = async (id: string): Promise<number> => {
      await runClaudeHook({ ...common, hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: id, tool_input: { file_path: "a.ts" } }, daemonPath);
      return performance.now() - started;
    };
    const finished = await Promise.all([toolCall("t1"), toolCall("t2"), toolCall("t3")]);
    // Each hook gives up on the silent daemon about 12 s after its event arrives. Under the session's lock each would
    // begin its wait when the one before it gave up, and the three would finish 2 s or more apart.
    expect(Math.max(...finished) - Math.min(...finished)).toBeLessThan(1_000);
  });

  it("gives up on a silent daemon 12 s after the event arrives, the start check's read of it included", { timeout: 60_000 }, async () => {
    const root = await temporaryDirectory();
    startTestRepository(root);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    await startNativeTestDaemon(root);
    const daemonPath = daemonHookOptions(root, { keepDatabase: false });
    const common = { session_id: "silent-session", cwd: root };
    await runClaudeHook({ ...common, hook_event_name: "SessionStart", source: "startup" }, daemonPath);
    const front = await silentFront();
    await rewriteState(state => ({ ...state, url: front, port: Number(new URL(front).port) }));
    const started = performance.now();
    expect(await runClaudeHook({ ...common, hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: "t1", tool_input: { file_path: "a.ts" } }, daemonPath)).toEqual({});
    // The start check waited 2 s for the daemon, then the endpoint lookup waited its own 2 s and 10 s: about 14.3 s.
    expect(performance.now() - started).toBeLessThan(13_000);
  });

  // RERAIL_AUTOSTART=1 is a person's session, where the start finds the daemon alive and waits for it; 0 starts none.
  it.each(["1", "0"])("gives up on a silent daemon at a session's start 12 s after the event arrives (RERAIL_AUTOSTART=%s)", { timeout: 60_000 }, async autostart => {
    const root = await temporaryDirectory();
    startTestRepository(root);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    vi.stubEnv("RERAIL_AUTOSTART", autostart);
    await startNativeTestDaemon(root);
    const front = await silentFront();
    await rewriteState(state => ({ ...state, url: front, port: Number(new URL(front).port) }));
    const daemonPath = daemonHookOptions(root, { keepDatabase: false });
    const started = performance.now();
    await expect(runClaudeHook({ session_id: "silent-start", cwd: root, hook_event_name: "SessionStart", source: "startup" }, daemonPath))
      .rejects.toThrow(/is alive but not answering/u);
    // The start check waited 2 s for the daemon, then the start's lookup waited its own 2 s and 10 s: about 14 s, and
    // 24 s with RERAIL_AUTOSTART=0, whose lookup waited the 10 s twice.
    expect(performance.now() - started).toBeLessThan(13_000);
  });
});

describe("Claude Code one-shot hook", () => {
  it("persists the documented hook lifecycle into the SQLite session store across separate hook processes", async () => {
    const root = await temporaryDirectory();
    const options = hookOptions(root);
    const common = { session_id: "claude-session", cwd: root, transcript_path: "/tmp/transcript.jsonl", permission_mode: "default" };

    const note = sessionStartNote(await dashboardOrigin(root), "claude-code", "claude-session", await presentedSessionNoteRules({ host: "claude-code", sessionId: "claude-session", cwd: root }));
    expect(await runClaudeHook({ ...common, hook_event_name: "SessionStart", source: "startup" }, options)).toEqual({
      hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: note },
    });
    expect(await runClaudeHook({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p1", prompt: "Refactor auth" }, options)).toEqual({});
    expect(await runClaudeHook({
      ...common,
      hook_event_name: "PreToolUse",
      tool_name: "Read",
      tool_use_id: "tool-1",
      tool_input: { file_path: "src/auth.ts" },
    }, options)).toEqual({});
    expect(await runClaudeHook({
      ...common,
      hook_event_name: "Stop",
      prompt_id: "p1",
      stop_hook_active: false,
      last_assistant_message: "Done.",
    }, options)).toEqual({});

    expect(eventRows(root, ["session-start", "user-prompt", "call", "turn-end"])).toEqual([
      ["claude-session:SessionStart", "session-start"],
      ["claude-session:prompt:p1", "user-prompt"],
      ["claude-session:tool-call:tool-1", "call"],
      ["claude-session:turn:p1", "turn-end"],
    ]);
  });

  it("records each Stop of a turn other Stop hooks keep going as a turn end of its own, and one Stop delivered twice once", async () => {
    const root = await temporaryDirectory();
    const options = hookOptions(root);
    const common = { session_id: "claude-session", cwd: root, hook_event_name: "Stop", prompt_id: "p1" };
    await runClaudeHook({ session_id: "claude-session", cwd: root, hook_event_name: "SessionStart", source: "startup" }, options);
    await runClaudeHook({ ...common, stop_hook_active: false, last_assistant_message: "Done." }, options);
    await runClaudeHook({ ...common, stop_hook_active: true, last_assistant_message: "Fixed the test." }, options);
    await runClaudeHook({ ...common, stop_hook_active: true, last_assistant_message: "Goal met." }, options);
    await runClaudeHook({ ...common, stop_hook_active: true, last_assistant_message: "Goal met." }, options);

    const turnEnds = eventRows(root, ["turn-end"]).map(([eventId]) => eventId);
    expect(turnEnds).toEqual([
      "claude-session:turn:p1",
      expect.stringMatching(/^claude-session:turn:p1:continued:/),
      expect.stringMatching(/^claude-session:turn:p1:continued:/),
    ]);
    expect(new Set(turnEnds).size).toBe(3);
  });

  it("ignores hook events the adapter does not model without touching storage", async () => {
    const root = await temporaryDirectory();
    const output = await runClaudeHook(
      { session_id: "claude-session", cwd: root, hook_event_name: "Notification", message: "hi" },
      hookOptions(root),
    );
    expect(output).toEqual({});
    expect(sessionDatabaseQuery(root, "SELECT session_id FROM sessions")).toEqual([]);
  });

  it("records the session at CLAUDE_PROJECT_DIR when the hook cwd has moved below it", async () => {
    const root = await temporaryDirectory();
    const nested = join(root, "nested");
    await mkdir(nested);
    process.env.CLAUDE_PROJECT_DIR = root;
    try {
      await runClaudeHook({ session_id: "claude-session", cwd: nested, hook_event_name: "SessionStart", source: "startup" }, hookOptions(root));
    } finally {
      delete process.env.CLAUDE_PROJECT_DIR;
    }
    expect(sessionDatabaseQuery(root, "SELECT cwd FROM sessions")).toEqual([{ cwd: root }]);
  });

  it("does not start or record hook activity where Rerail is not started", async () => {
    const root = await temporaryDirectory({ started: false });
    const start = { session_id: "claude-session", cwd: root, hook_event_name: "SessionStart" as const, source: "startup" };

    expect(await runClaudeHook(start, hookOptions(root))).toEqual({});
    await expect(readFile(join(root, "rerail.sqlite"))).rejects.toMatchObject({ code: "ENOENT" });

    startTestRepository(root, hookOptions(root).databasePath);
    stopTestRepository(root, hookOptions(root).databasePath);
    expect(await runClaudeHook(start, hookOptions(root))).toEqual({});
    expect(sessionDatabaseQuery(root, "SELECT session_id FROM sessions")).toEqual([]);
  });

  it("answers {} on stdout and exits cleanly when stdin is malformed or the runtime fails", async () => {
    const root = await temporaryDirectory();
    // The runtime failure below reads the default database, so the repository is started there too.
    startTestRepository(root);
    const serve = (stdin: string, options: HookOptions) => serveText(input => runClaudeHook(input, options), stdin);

    const malformed = await serve("{not json", hookOptions(root));
    expect(JSON.parse(malformed.stdout)).toEqual({});
    expect(malformed.stderr).toMatch(/^rerail hook: /);

    const runtimeFailure = await serve(
      JSON.stringify({ hook_event_name: "SessionStart", session_id: "s", cwd: root, source: "startup" }),
      { storageRoot: join(root, "sessions"), kernelSocketPath: join(root, "missing-engine.sock") },
    );
    expect(JSON.parse(runtimeFailure.stdout)).toEqual({});
    expect(runtimeFailure.stderr).toMatch(/^rerail hook: /);
  });

  it("links the session note and a steer's why? to the checkout's dashboard when the daemon is on loopback", async () => {
    const root = await temporaryDirectory();
    const home = join(root, "home");
    await mkdir(home);
    await writeFile(join(home, "daemon.json"), JSON.stringify({ pid: process.pid, host: "127.0.0.1", port: 4791, url: "http://127.0.0.1:4791/",
      version: "test", databasePath: join(home, "rerail.sqlite"), kernelSocketPath: testEngineEndpoint(home), ui: "bundled", startedAt: 1, packageRoot: home }));
    vi.stubEnv("RERAIL_HOME", home);
    try {
      const common = { session_id: "claude-session", cwd: root };
      const start = await handleClaudeEvent({ ...common, hook_event_name: "SessionStart", source: "startup" }, scriptedLoop({}), join(root, "sessions"));
      expect(JSON.stringify(start)).toContain("http://127.0.0.1:4791/?sessionId=claude-session&host=claude-code#sessions");

      const steer = scriptedLoop({ result: [delivery("steer", editSteer("acme:no-print", "Remove print."), ["acme:no-print"])] });
      const output = await handleClaudeEvent({
        ...common, hook_event_name: "PostToolUse", tool_name: "Write", tool_use_id: "tool-1", tool_input: {}, tool_response: "ok",
      }, steer, join(root, "sessions"), { TERM_PROGRAM: "iTerm.app", COLORTERM: "truecolor" });
      expect(output).toMatchObject({
        // Claude Code draws ANSI colour, so Rerail is its coloured mark, each letter its own colour.
        systemMessage: expect.stringMatching(/^◆ \u001b\[38;2;165;173;251mR\u001b\[39m .* · acme:no-print {2}steered {2}· {2}\u001b\]8;;http:\/\/127\.0\.0\.1:4791\/#rule\/acme%3Ano-print\u0007why\?\u001b\]8;;\u0007$/u),
        hookSpecificOutput: {
          hookEventName: "PostToolUse",
          additionalContext: editSteer("acme:no-print", "Remove print."),
        },
      });
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("gives a turn-end steer to the coding agent at Stop, with the person's line and its count, and nothing at the next prompt", async () => {
    const root = await temporaryDirectory();
    const common = { session_id: "claude-session", cwd: root };
    const { receipts, serve } = receiptServer(join(root, "sessions"));
    const turnEnd = [delivery("steer", editSteer("a", "Fix a."), ["a"]), delivery("steer", editSteer("b", "Fix b."), ["b"])];

    // Without a daemon the rules stay unlinked, so each line is the header alone; without colour the mark is plain.
    expect(await serve({ ...common, hook_event_name: "Stop", prompt_id: "p1", stop_hook_active: false }, scriptedLoop({ "turn-end": turnEnd }))).toEqual({
      systemMessage: "\n◆ R E R A I L · a  steered\n◆ R E R A I L · b  steered",
      hookSpecificOutput: { hookEventName: "Stop", additionalContext: [editSteer("a", "Fix a."), editSteer("b", "Fix b.")].join("\n\n") },
    });
    expect(receipts).toEqual([["a", "b"]]);

    // Delivered at Stop, nothing of them is left for the person's next prompt or its first tool call.
    expect(await serve({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p2", prompt: "next" }, scriptedLoop({}))).toEqual({});
    expect(await serve({ ...common, hook_event_name: "PreToolUse", tool_name: "Bash", tool_use_id: "tool-2", tool_input: { command: "git log" } }, scriptedLoop({}))).toEqual({});
    expect(receipts).toEqual([["a", "b"]]);
  });

  it("keeps a turn going once: a continued Stop holds its steer, which goes out late with its line and count at the next tool call", async () => {
    const root = await temporaryDirectory();
    const common = { session_id: "claude-session", cwd: root };
    const { receipts, serve } = receiptServer(join(root, "sessions"));

    expect(await serve({ ...common, hook_event_name: "Stop", prompt_id: "p1", stop_hook_active: true }, scriptedLoop({ "turn-end": [delivery("steer", editSteer("a", "Fix a."), ["a"])] })))
      .toEqual({});
    expect(receipts).toEqual([]);

    expect(await serve({ ...common, hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: "tool-2", tool_input: {} }, scriptedLoop({}))).toEqual({
      systemMessage: "◆ R E R A I L · a  steered",
      hookSpecificOutput: {
        hookEventName: "PreToolUse",
        additionalContext: editSteer("a", "Fix a.").replace("Rerail found 1 problem in your last edit:", "Rerail finished checking your earlier edits after you moved on:"),
      },
    });
    expect(receipts).toEqual([["a"]]);
  });

  it("leaves held steers held and counts nothing at a Stop without prompt_id, as Claude Code before 2.1.196 sends it", async () => {
    const root = await temporaryDirectory();
    const common = { session_id: "claude-session", cwd: root };
    const { receipts, serve } = receiptServer(join(root, "sessions"));
    await serve(
      { ...common, hook_event_name: "MessageDisplay", message_id: "m1", index: "0", delta: "text", final: true },
      scriptedLoop({ "assistant-delta": [delivery("steer", editSteer("a", "Fix a."), ["a"])] }),
    );

    // Claude Code drops Stop context before 2.1.163 and sends no prompt_id before 2.1.196: such a Stop must not
    // spend the held steer on an agent that cannot be told.
    expect(await serve({ ...common, hook_event_name: "Stop", stop_hook_active: false }, scriptedLoop({}))).toEqual({});
    expect(receipts).toEqual([]);

    expect(await serve({ ...common, hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: "tool-2", tool_input: {} }, scriptedLoop({})))
      .toEqual({ systemMessage: "◆ R E R A I L · a  steered", hookSpecificOutput: { hookEventName: "PreToolUse", additionalContext: editSteer("a", "Fix a.") } });
    expect(receipts).toEqual([["a"]]);
  });

  it("holds a steer found at the person's prompt, line and all, for the next tool call", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "claude-session", cwd: root };
    const prompted = scriptedLoop({ "user-prompt": [delivery("steer", editSteer("c", "Fix c."), ["c"])] });

    // UserPromptSubmit's context lands inside the person's turn.
    expect(await handleClaudeEvent({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p1", prompt: "next" }, prompted, storageRoot, { NO_COLOR: "1" })).toEqual({});
    expect(await handleClaudeEvent({ ...common, hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: "tool-1", tool_input: {} }, scriptedLoop({}), storageRoot, { NO_COLOR: "1" }))
      .toEqual({ systemMessage: "◆ R E R A I L · c  steered", hookSpecificOutput: { hookEventName: "PreToolUse", additionalContext: editSteer("c", "Fix c.") } });
  });

  it("prints the why link after the word when the terminal cannot draw links", () => {
    const linked = delivery("steer", editSteer("a", "Fix a."), ["a"], steeredRules(["a"], "http://127.0.0.1:4791/"));
    expect(claudeHookOutput("PostToolUse", [linked], { links: "plain" })).toMatchObject({
      systemMessage: "◆ Rerail · a  steered  ·  why? → http://127.0.0.1:4791/#rule/a",
    });
  });

  it("routes steer and block deliveries through additionalContext or the pending file, never the person's prompt", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "claude-session", cwd: root };
    const ContextOutput = z.object({
      hookSpecificOutput: z.object({ hookEventName: z.string(), additionalContext: z.string() }),
    });

    const startLoop = scriptedLoop({ "session-start": [delivery("steer", "Read the conventions."), delivery("log", "hidden")] });
    expect(await handleClaudeEvent({ ...common, hook_event_name: "SessionStart", source: "startup" }, startLoop, storageRoot)).toEqual({
      hookSpecificOutput: {
        hookEventName: "SessionStart",
        additionalContext: `${sessionStartNote(await dashboardOrigin(root), "claude-code", "claude-session", await presentedSessionNoteRules({ host: "claude-code", sessionId: "claude-session", cwd: root }))}\n\nRead the conventions.`,
      },
    });

    const toolLoop = scriptedLoop({
      call: [delivery("steer", "Careful with auth."), delivery("block", "Do not edit generated files."), delivery("log", "Recorded only.")],
    });
    const toolOutput = ContextOutput.parse(await handleClaudeEvent({
      ...common,
      hook_event_name: "PreToolUse",
      tool_name: "Edit",
      tool_use_id: "tool-1",
      tool_input: {},
    }, toolLoop, storageRoot));
    expect(toolOutput.hookSpecificOutput).toEqual({
      hookEventName: "PreToolUse",
      additionalContext: "Careful with auth.\n\nDo not edit generated files.",
    });

    const displayLoop = scriptedLoop({ "assistant-delta": [delivery("steer", "Deferred steer."), delivery("block", "Deferred block.")] });
    expect(await handleClaudeEvent({
      ...common,
      hook_event_name: "MessageDisplay",
      message_id: "m1",
      index: "0",
      delta: "text",
      final: true,
    }, displayLoop, storageRoot)).toEqual({});

    // A turn that ends with steers still held hands them to the agent at Stop, which continues the turn.
    expect(await handleClaudeEvent({ ...common, hook_event_name: "Stop", prompt_id: "p1" }, scriptedLoop({}), storageRoot)).toEqual({
      hookSpecificOutput: { hookEventName: "Stop", additionalContext: "Deferred steer.\n\nDeferred block." },
    });

    expect(await handleClaudeEvent({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p2", prompt: "next" }, scriptedLoop({}), storageRoot)).toEqual({});
    expect(await handleClaudeEvent({
      ...common, hook_event_name: "PostToolUse", tool_name: "Read", tool_use_id: "tool-2", tool_input: {}, tool_response: "ok",
    }, scriptedLoop({}), storageRoot)).toEqual({});

    expect(await handleClaudeEvent({ ...common, hook_event_name: "Stop", prompt_id: "p2" }, scriptedLoop({}), storageRoot)).toEqual({});
    expect(claudeHookOutput("SessionEnd", [delivery("steer", "ignored")])).toEqual({});
  });

  it("discards pending context when Claude reports that the session has ended", async () => {
    const root = await temporaryDirectory();
    const storageRoot = join(root, "sessions");
    const common = { session_id: "claude-session", cwd: root };

    expect(await handleClaudeEvent(
      { ...common, hook_event_name: "UserPromptSubmit", prompt_id: "p1", prompt: "next" },
      scriptedLoop({ "user-prompt": [delivery("steer", "Do not reuse this.")] }),
      storageRoot,
    )).toEqual({});
    expect(await handleClaudeEvent(
      { ...common, hook_event_name: "SessionEnd", reason: "clear" },
      scriptedLoop({}),
      storageRoot,
    )).toEqual({});
    expect(await handleClaudeEvent(
      { ...common, hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: "tool-1", tool_input: {} },
      scriptedLoop({}),
      storageRoot,
    )).toEqual({});
  });
});
