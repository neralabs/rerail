import { execFileSync } from "node:child_process";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { decodeRerailEventFrame, RerailRuntime } from "../src/runtime/index.js";
import { codeRuleSchema, type AdapterDelivery } from "../src/engine/index.js";
import {
  CONTRACT_CURSOR_EXECUTE_CODE_RULE, CONTRACT_CURSOR_EXECUTE_MESSAGE, CONTRACT_CURSOR_EXECUTE_RULE_ID, seedContractCodeRuleForRoot,
} from "./support/contract-code-rule.js";
import { setUpTestRepository } from "../test/support/repository-rules.js";
import { steerText } from "./support/steer-text.js";


const originalRerailHome = process.env.RERAIL_HOME;
afterEach(() => {
  if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = originalRerailHome;
});
async function codeRuleSteers(runtime: RerailRuntime, sessionId: string, initial: readonly AdapterDelivery[], paths: string[]): Promise<AdapterDelivery[]> {
  const deliveries = [...initial];
  let poll = 0;
  const steers = () => deliveries.filter(delivery => delivery.requested === "steer" && delivery.ruleIds.includes(CONTRACT_CURSOR_EXECUTE_RULE_ID));
  await expect.poll(async () => {
    if (paths.every(path => steers().some(delivery => delivery.content.includes(path)))) return [];
    deliveries.push(...await runtime.ingest({
      kind: "user-prompt", eventId: `code-rule-poll:${++poll}`, sessionId, text: "continue",
    }));
    return paths.filter(path => !steers().some(delivery => delivery.content.includes(path)));
  }, { timeout: 10_000, interval: 10 }).toEqual([]);
  return steers();
}

// This suite crosses the production event seam with the actual native model,
// compiled code-rule catalog, filesystem, and session persistence. No test doubles.
describe("shared host engine", () => {
  it("runs conversation rule matching and committed-edit code rule enforcement in an OMP session", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-kernel-harness-"));
    process.env.RERAIL_HOME = join(root, "home");
    // The repository runs the contract code rule and one conversation rule, both from the library.
    setUpTestRepository(root, {
      library: [
        { kind: "code", rule: codeRuleSchema.parse(CONTRACT_CURSOR_EXECUTE_CODE_RULE) },
        {
          kind: "conversation",
          rule: {
            id: "require-transaction-guidance",
            mode: "steer",
            reference: "Preserve database transaction atomicity",
            match: { operator: "gte", threshold: 0.95 },
            sample: { source: "user-input", boundary: "message" },
            action: { content: "CONVERSATION: preserve transaction atomicity" },
            repeat: { mode: "once" },
          },
        },
      ],
    }, join(root, "home", "rerail.sqlite"));
    let runtime: RerailRuntime | undefined;
    try {
      await mkdir(join(root, "api"));
      await writeFile(join(root, "api", "route.py"), "def route():\n    return 1\n");
      runtime = new RerailRuntime({ storageRoot: join(root, ".rerail", "sessions"), workspaceRoot: root, modelPath: resolve("model") });
      const session = { host: "omp" as const, sessionId: "shared-runtime", cwd: root };
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      const conversationRule = await runtime.ingest({
        kind: "user-prompt",
        eventId: "prompt",
        sessionId: session.sessionId,
        text: "Preserve database transaction atomicity",
      });
      expect(conversationRule.filter(delivery => delivery.requested === "steer")).toEqual([
        {
          sessionId: session.sessionId,
          requested: "steer",
          mechanism: "native-queue",
          content: await steerText("require-transaction-guidance", "CONVERSATION: preserve transaction atomicity"),
          // Conversation rule matches are not code-rule steers: steer attribution only credits code rules.
          ruleIds: [],
        },
      ]);
      await runtime.ingest({
        kind: "call",
        toolCallId: "write-route",
        eventId: "write-start",
        sessionId: session.sessionId,
        toolName: "write",
        input: { path: "api/route.py" },
      });
      await writeFile(join(root, "api", "route.py"), 'def route(cursor):\n    cursor.execute("SELECT 1")\n');
      const codeRule = await runtime.ingest({
        kind: "result",
        toolCallId: "write-route",
        eventId: "write-end",
        sessionId: session.sessionId,
        toolName: "write",
        output: { ok: true },
      });
      expect(await codeRuleSteers(runtime, session.sessionId, codeRule, ["api/route.py"])).toEqual([
        expect.objectContaining({ requested: "steer", mechanism: "native-queue", content: expect.stringContaining(CONTRACT_CURSOR_EXECUTE_MESSAGE) }),
      ]);
      const repeated = await runtime.ingest({
        kind: "user-prompt",
        eventId: "prompt-again",
        sessionId: session.sessionId,
        text: "Preserve database transaction atomicity",
      });
      expect(repeated.filter(delivery => delivery.requested === "steer")).toEqual([]);
      await runtime.ingest({ kind: "session-end", eventId: "end", sessionId: session.sessionId });
    } finally {
      await runtime?.dispose();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("correlates out-of-order results by tool call identity, not tool name", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-kernel-correlation-"));
    process.env.RERAIL_HOME = join(root, "home");
    seedContractCodeRuleForRoot(root);
    let runtime: RerailRuntime | undefined;
    try {
      await mkdir(join(root, "api"));
      await writeFile(join(root, "api", "first.py"), "value = 1\n");
      await writeFile(join(root, "api", "second.py"), "value = 2\n");
      runtime = new RerailRuntime({ storageRoot: join(root, ".rerail", "sessions"), workspaceRoot: root, modelPath: resolve("model") });
      const session = { host: "claude-code" as const, sessionId: "correlation", cwd: root };
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await runtime.codeRulesOpened(session.sessionId);
      for (const [toolCallId, path] of [
        ["first", "api/first.py"],
        ["second", "api/second.py"],
      ]) {
        const { event } = decodeRerailEventFrame(
          JSON.stringify({
            event: {
              kind: "call",
              eventId: `call:${toolCallId}`,
              toolCallId,
              sessionId: session.sessionId,
              toolName: "write",
              input: { path },
            },
          }),
        );
        await runtime.ingest(event);
      }
      await writeFile(join(root, "api", "second.py"), 'cursor.execute("SELECT 1")\n');
      const { event } = decodeRerailEventFrame(
        JSON.stringify({
          event: {
            kind: "result",
            eventId: "result:second",
            toolCallId: "second",
            sessionId: session.sessionId,
            toolName: "write",
            output: { ok: true },
          },
        }),
      );
      const deliveries = await runtime.ingest(event);
      const steers = await codeRuleSteers(runtime, session.sessionId, deliveries, ["api/second.py"]);
      expect(steers.every(delivery => !delivery.content.includes("api/first.py"))).toBe(true);
    } finally {
      await runtime?.dispose();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("reconciles shell-written files at the command boundary and finds nothing new at the turn checkpoint", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-kernel-shell-"));
    process.env.RERAIL_HOME = join(root, "home");
    seedContractCodeRuleForRoot(root);
    let runtime: RerailRuntime | undefined;
    try {
      await mkdir(join(root, "api"));
      await writeFile(join(root, "api/route.py"), "def route():\n    return 1\n");
      runtime = new RerailRuntime({ storageRoot: join(root, ".rerail/sessions"), workspaceRoot: root, modelPath: resolve("model") });
      const session = { host: "codex" as const, sessionId: "shell", cwd: root };
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await runtime.codeRulesOpened(session.sessionId);
      await runtime.ingest({
        kind: "call",
        eventId: "call:shell",
        toolCallId: "shell",
        sessionId: session.sessionId,
        toolName: "Bash",
        input: { command: "write route" },
      });
      execFileSync(
        process.execPath,
        [
          "-e",
          "require('node:fs').writeFileSync('api/route.py', 'def route(cursor):\\n    cursor.execute(\"SELECT 1\")\\n')",
        ],
        { cwd: root },
      );
      // The command result reconciles the shell write and carries its violation.
      const boundary = await runtime.ingest({
        kind: "result",
        eventId: "result:shell",
        toolCallId: "shell",
        sessionId: session.sessionId,
        toolName: "Bash",
        output: "",
      });
      const steers = await codeRuleSteers(runtime, session.sessionId, boundary, ["api/route.py"]);
      expect(steers).toHaveLength(1);
      // The turn checkpoint sees the file already observed and reports nothing again.
      const turn = await runtime.ingest({
        kind: "turn-end",
        eventId: "turn",
        sessionId: session.sessionId,
        text: "Changed route.",
      });
      expect(turn.filter(delivery => delivery.ruleIds.includes(CONTRACT_CURSOR_EXECUTE_RULE_ID))).toEqual([]);
    } finally {
      await runtime?.dispose();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("discards unfinished assistant text when the host aborts its stream", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-kernel-abort-"));
    process.env.RERAIL_HOME = join(root, "home");
    setUpTestRepository(root, {
      library: [
        { kind: "code", rule: codeRuleSchema.parse(CONTRACT_CURSOR_EXECUTE_CODE_RULE) },
        {
          kind: "conversation",
          rule: {
            id: "unfinished",
            mode: "steer",
            reference: "Preserve database transaction atomicity",
            match: { operator: "gte", threshold: 0.95 },
            sample: { source: "assistant-text", boundary: "message", minimumTokens: 1 },
            action: { content: "UNFINISHED" },
            repeat: { mode: "always" },
          },
        },
      ],
    }, join(root, "home", "rerail.sqlite"));
    let runtime: RerailRuntime | undefined;
    try {
      runtime = new RerailRuntime({ storageRoot: join(root, ".rerail/sessions"), workspaceRoot: root, modelPath: resolve("model") });
      const session = { host: "pi" as const, sessionId: "abort", cwd: root };
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await runtime.codeRulesOpened(session.sessionId);
      await runtime.ingest({
        kind: "assistant-delta",
        eventId: "delta",
        sessionId: session.sessionId,
        messageId: "message",
        delta: "Preserve database transaction atomicity",
        final: false,
      });
      const { event } = decodeRerailEventFrame(
        JSON.stringify({
          event: {
            kind: "assistant-abort",
            eventId: "abort",
            sessionId: session.sessionId,
            messageId: "message",
          },
        }),
      );
      await runtime.ingest(event);
      expect(
        await runtime.ingest({
          kind: "assistant-delta",
          eventId: "final",
          sessionId: session.sessionId,
          messageId: "message",
          delta: "",
          final: true,
        }),
      ).toEqual([]);
    } finally {
      await runtime?.dispose();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("delivers current violations for concurrent tool results without losing either edit", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-kernel-concurrent-"));
    process.env.RERAIL_HOME = join(root, "home");
    seedContractCodeRuleForRoot(root);
    let runtime: RerailRuntime | undefined;
    try {
      await mkdir(join(root, "api"));
      for (const name of ["first", "second"])
        await writeFile(join(root, `api/${name}.py`), "def route():\n    return 1\n");
      runtime = new RerailRuntime({ storageRoot: join(root, ".rerail/sessions"), workspaceRoot: root, modelPath: resolve("model") });
      const session = { host: "omp" as const, sessionId: "concurrent", cwd: root };
      await runtime.ingest({ kind: "session-start", eventId: "start", session });
      await runtime.codeRulesOpened(session.sessionId);
      for (const name of ["first", "second"]) {
        await runtime.ingest({
          kind: "call",
          eventId: `call:${name}`,
          toolCallId: name,
          sessionId: session.sessionId,
          toolName: "write",
          input: { path: `api/${name}.py` },
        });
        await writeFile(join(root, `api/${name}.py`), "def route(cursor):\n    cursor.execute('SELECT 1')\n");
      }
      const deliveries = await Promise.all(
        ["first", "second"].map((name) =>
          runtime!.ingest({
            kind: "result",
            eventId: `result:${name}`,
            toolCallId: name,
            sessionId: session.sessionId,
            toolName: "write",
            output: "written",
          }),
        ),
      );
      await codeRuleSteers(runtime, session.sessionId, deliveries.flat(), ["api/first.py", "api/second.py"]);
    } finally {
      await runtime?.dispose();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
});
