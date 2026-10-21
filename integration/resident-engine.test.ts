import { appendFile, mkdtemp, mkdir, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { ingestEngineEvent, startEngineService, stopDaemon, SqliteSessionReader } from "../src/runtime/index.js";
import { runCodexHook } from "../src/hosts/codex/index.js";
import { RerailDaemon } from "../src/daemon/index.js";
import { runCursorHook } from "../src/hosts/cursor/index.js";
import { type AdapterDelivery, parseConversationRule } from "../src/engine/index.js";
import { awaitCodeRulesOpen } from "./support/code-rules-open.js";
import { CONTRACT_CURSOR_EXECUTE_MESSAGE, CONTRACT_CURSOR_EXECUTE_RULE_ID, seedContractCodeRule } from "./support/contract-code-rule.js";
import { cursorRelayed, steerLine, steerText } from "./support/steer-text.js";
import { startTestRepository } from "../test/support/started-repository.js";
import { setUpTestRepository } from "../test/support/repository-rules.js";
const originalRerailHome = process.env.RERAIL_HOME;
let isolatedRerailHome: string;

beforeAll(async () => {
  isolatedRerailHome = await mkdtemp(join(tmpdir(), "rerail-resident-home-"));
  process.env.RERAIL_HOME = isolatedRerailHome;
});

afterAll(async () => {
  if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = originalRerailHome;
  await rm(isolatedRerailHome, { recursive: true, force: true });
});

/** Add conversation rules from the library to `root`'s repository, set up and started: only rules a repository adds run. */
function addConversationRules(root: string, rules: readonly unknown[], databasePath?: string): void {
  setUpTestRepository(root, { library: rules.map(rule => ({ kind: "conversation" as const, rule: parseConversationRule(rule) })) }, databasePath);
}

async function awaitCodeRuleSteer(
  initial: readonly AdapterDelivery[],
  poll: (eventId: string) => Promise<readonly AdapterDelivery[]>,
  content: string,
): Promise<readonly AdapterDelivery[]> {
  const deliveries = [...initial];
  let pollCount = 0;
  await expect.poll(async () => {
    deliveries.push(...await poll(`code-rule-poll:${++pollCount}`));
    return deliveries.filter(delivery => delivery.requested === "steer" && delivery.content.includes(content));
  }, { timeout: 10_000, interval: 10 }).toEqual([
    expect.objectContaining({ requested: "steer", content: expect.stringContaining(content) }),
  ]);
  return deliveries.filter(delivery => delivery.requested === "steer" && delivery.content.includes(content));
}

describe("engine service", () => {
  it("preserves code rule correlation across one-shot hook connections", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-resident-kernel-"));
    seedContractCodeRule(isolatedRerailHome, root);
    const socketPath = join(root, "engine.sock");
    const storageRoot = join(root, ".rerail", "sessions");
    let service: { close(): Promise<void> } | undefined;
    const session = { host: "omp" as const, sessionId: "resident-engine", cwd: root };

    try {
      await mkdir(join(root, "api"));
      await writeFile(join(root, "api", "route.py"), "def route():\n    return 1\n");
      service = await startEngineService({ socketPath, storageRoot, modelPath: resolve("model") });

      await ingestEngineEvent(socketPath, { kind: "session-start", eventId: "start", session });
      await awaitCodeRulesOpen(socketPath, session);
      await ingestEngineEvent(socketPath, {
        kind: "call",
        eventId: "call:write-route",
        toolCallId: "write-route",
        sessionId: session.sessionId,
        toolName: "write",
        input: { path: "api/route.py" },
      });

      await writeFile(join(root, "api", "route.py"), 'def route(cursor):\n    cursor.execute("SELECT 1")\n');
      const deliveries = await ingestEngineEvent(socketPath, {
        kind: "result",
        eventId: "result:write-route",
        toolCallId: "write-route",
        sessionId: session.sessionId,
        toolName: "write",
        output: { ok: true },
      });

      expect(await awaitCodeRuleSteer(
        deliveries,
        eventId => ingestEngineEvent(socketPath, {
          kind: "user-prompt",
          eventId,
          sessionId: session.sessionId,
          text: "continue",
        }),
        CONTRACT_CURSOR_EXECUTE_RULE_ID,
      )).toEqual([
        expect.objectContaining({
          sessionId: session.sessionId,
          mechanism: "native-queue",
          content: expect.stringContaining(CONTRACT_CURSOR_EXECUTE_MESSAGE),
          ruleIds: expect.arrayContaining([CONTRACT_CURSOR_EXECUTE_RULE_ID]),
        }),
      ]);
      await ingestEngineEvent(socketPath, { kind: "session-end", eventId: "end", sessionId: session.sessionId });
    } finally {
      await service?.close();
      await rm(socketPath, { force: true });
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("keeps code rule state when each Codex hook invocation closes its client", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-resident-hooks-"));
    seedContractCodeRule(isolatedRerailHome, root);
    const socketPath = join(root, "engine.sock");
    const storageRoot = join(root, ".rerail/sessions");
    let service: { close(): Promise<void> } | undefined;
    try {
      await mkdir(join(root, "api"));
      await writeFile(join(root, "api/route.py"), "def route():\n    return 1\n");
      service = await startEngineService({ socketPath, storageRoot, modelPath: resolve("model") });
      const options = { storageRoot, modelPath: resolve("model"), kernelSocketPath: socketPath };
      const common = { session_id: "codex-resident", cwd: root };
      await runCodexHook({ ...common, hook_event_name: "SessionStart", source: "startup" }, options);
      await runCodexHook(
        {
          ...common,
          hook_event_name: "PreToolUse",
          tool_use_id: "write",
          tool_name: "apply_patch",
          tool_input: {
            command:
              "*** Begin Patch\n*** Update File: api/route.py\n@@\n-def route():\n-    return 1\n+def route(cursor):\n+    cursor.execute('SELECT 1')\n*** End Patch",
          },
        },
        options,
      );
      await writeFile(join(root, "api/route.py"), "def route(cursor):\n    cursor.execute('SELECT 1')\n");
      const initial = await runCodexHook(
        {
          ...common,
          hook_event_name: "PostToolUse",
          tool_use_id: "write",
          tool_name: "apply_patch",
          tool_response: "written",
        },
        options,
      );
      const stop = await runCodexHook(
        {
          ...common,
          hook_event_name: "Stop",
          turn_id: "turn-1",
          stop_hook_active: false,
          last_assistant_message: "Done.",
        },
        options,
      );
      const next = await runCodexHook({ ...common, hook_event_name: "UserPromptSubmit", turn_id: "turn-2", prompt: "next" }, options);
      const steer = steerText(CONTRACT_CURSOR_EXECUTE_RULE_ID, CONTRACT_CURSOR_EXECUTE_MESSAGE, "api/route.py:2:5");
      const late = steerText(CONTRACT_CURSOR_EXECUTE_RULE_ID, CONTRACT_CURSOR_EXECUTE_MESSAGE, "api/route.py:2:5", { kind: "late" });
      const shown = expect.stringContaining(await steerLine(CONTRACT_CURSOR_EXECUTE_RULE_ID, common.cwd, "plain"));
      // Policy evaluation settles in the background: the steer returns as PostToolUse context when it is ready by
      // then; otherwise its line shows at Stop and its block, led as late, goes out with the next message. Never as a
      // Stop block, whose reason Codex records as the user's turn.
      expect([initial, stop, next]).toEqual(Object.keys(initial).length === 0
        ? [{}, { systemMessage: shown }, { hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext: expect.stringContaining(late) } }]
        : [{ hookSpecificOutput: { hookEventName: "PostToolUse", additionalContext: expect.stringContaining(steer) }, systemMessage: shown }, {}, {}]);
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("serves the native engine for hooks throughout the daemon lifetime", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-daemon-kernel-"));
    let daemon: RerailDaemon | undefined;
    try {
      addConversationRules(root, [
        {
          id: "resident",
          mode: "steer",
          reference: "Preserve database transaction atomicity",
          match: { operator: "gte", threshold: 0.95 },
          sample: { source: "user-input", boundary: "message", minimumTokens: 1 },
          action: { content: "DAEMON_ENGINE_MATCH" },
          repeat: { mode: "once" },
        },
      ], join(root, ".rerail/rerail.sqlite"));
      daemon = await RerailDaemon.start({
        version: "test",
        port: 0,
        databasePath: join(root, ".rerail/rerail.sqlite"),
        workspaceRoot: root,
      });
      const session = { host: "claude-code" as const, sessionId: "daemon-session", cwd: root };
      await ingestEngineEvent(daemon.state().kernelSocketPath, { kind: "session-start", eventId: "start", session });
      const deliveries = await ingestEngineEvent(daemon.state().kernelSocketPath, {
        kind: "user-prompt",
        eventId: "prompt",
        sessionId: session.sessionId,
        text: "Preserve database transaction atomicity",
      });
      const socketPath = daemon.state().kernelSocketPath;
      expect(await awaitCodeRuleSteer(
        deliveries,
        eventId => ingestEngineEvent(socketPath, {
          kind: "user-prompt",
          eventId,
          sessionId: session.sessionId,
          text: "continue",
        }),
        "DAEMON_ENGINE_MATCH",
      )).toEqual([
        expect.objectContaining({ content: steerText("resident", "DAEMON_ENGINE_MATCH"), mechanism: "additional-context" }),
      ]);
    } finally {
      await daemon?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("retains captured pre-edit source when the daemon restarts during a tool call", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-kernel-restart-"));
    seedContractCodeRule(isolatedRerailHome, root);
    const socketPath = join(root, "engine.sock");
    const options = { socketPath, storageRoot: join(root, ".rerail/sessions"), modelPath: resolve("model") };
    let service: { close(): Promise<void> } | undefined;
    const session = { host: "codex" as const, sessionId: "restart", cwd: root };
    try {
      await mkdir(join(root, "api"));
      await writeFile(join(root, "api/route.py"), "def route():\n    return 1\n");
      service = await startEngineService(options);
      await ingestEngineEvent(socketPath, { kind: "session-start", eventId: "start", session });
      await awaitCodeRulesOpen(socketPath, session);
      await ingestEngineEvent(socketPath, {
        kind: "call",
        eventId: "call",
        toolCallId: "write",
        sessionId: session.sessionId,
        toolName: "write",
        input: { path: "api/route.py" },
      });
      await service.close();
      service = undefined;
      await writeFile(join(root, "api/route.py"), "def route(cursor):\n    cursor.execute('SELECT 1')\n");
      service = await startEngineService(options);
      // The restarted runtime takes the session back at its next event and reopens its evaluator; a result that
      // arrived during that open would go on without code rules.
      await ingestEngineEvent(socketPath, { kind: "user-prompt", eventId: "reattach", sessionId: session.sessionId, session, text: "go on" });
      await awaitCodeRulesOpen(socketPath, session);
      const deliveries = await ingestEngineEvent(socketPath, {
        kind: "result",
        eventId: "result",
        toolCallId: "write",
        sessionId: session.sessionId,
        session,
        toolName: "write",
        output: "written",
      });
      expect(await awaitCodeRuleSteer(
        deliveries,
        eventId => ingestEngineEvent(socketPath, {
          kind: "user-prompt",
          eventId,
          sessionId: session.sessionId,
          session,
          text: "continue",
        }),
        CONTRACT_CURSOR_EXECUTE_RULE_ID,
      )).toEqual([expect.objectContaining({
        content: expect.stringContaining(CONTRACT_CURSOR_EXECUTE_MESSAGE),
        ruleIds: expect.arrayContaining([CONTRACT_CURSOR_EXECUTE_RULE_ID]),
      })]);
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("isolates identical session IDs in different projects served by one daemon", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-kernel-projects-"));
    const socketPath = join(root, "engine.sock");
    let service: { close(): Promise<void> } | undefined;
    try {
      service = await startEngineService({
        socketPath,
        storageRoot: join(root, "default"),
        modelPath: resolve("model"),
      });
      for (const project of ["first", "second"]) {
        const cwd = join(root, project);
        addConversationRules(cwd, [
          {
            id: "project-rule",
            mode: "steer",
            reference: "Preserve database transaction atomicity",
            match: { operator: "gte", threshold: 0.95 },
            sample: { source: "user-input", boundary: "message", minimumTokens: 1 },
            action: { content: project },
            repeat: { mode: "once" },
          },
        ], join(cwd, ".rerail/rerail.sqlite"));
        const configuration = {
          storageRoot: join(cwd, ".rerail/sessions"),
          modelPath: resolve("model"),
          databasePath: join(cwd, ".rerail/rerail.sqlite"),
        };
        const session = { host: "claude-code" as const, sessionId: "same-id", cwd };
        await ingestEngineEvent(socketPath, { kind: "session-start", eventId: "start", session }, configuration);
        const deliveries = await ingestEngineEvent(
          socketPath,
          {
            kind: "user-prompt",
            eventId: "prompt",
            sessionId: session.sessionId,
            text: "Preserve database transaction atomicity",
          },
          configuration,
        );
        expect(await awaitCodeRuleSteer(
          deliveries,
          eventId => ingestEngineEvent(socketPath, {
            kind: "user-prompt",
            eventId: `${project}:${eventId}`,
            sessionId: session.sessionId,
            session,
            text: "continue",
          }, configuration),
          project,
        )).toEqual([expect.objectContaining({ content: steerText("project-rule", project), sessionId: session.sessionId })]);
      }
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it("defers Cursor failure-event steers to the next supported tool context", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-cursor-delivery-"));
    const socketPath = join(root, "engine.sock");
    const storageRoot = join(root, ".rerail/sessions");
    let service: { close(): Promise<void> } | undefined;
    try {
      addConversationRules(root, [
        {
          id: "recovery",
          mode: "steer",
          reference: "The requested operation failed",
          match: { operator: "gte", threshold: 0.95 },
          sample: { source: "tool:Write", boundary: "tool-call", minimumTokens: 1 },
          action: { content: "CURSOR_RECOVERY" },
          repeat: { mode: "once" },
        },
      ]);
      service = await startEngineService({ socketPath, storageRoot, modelPath: resolve("model") });
      const options = { storageRoot, modelPath: resolve("model"), kernelSocketPath: socketPath };
      const common = { workspace_roots: [root], conversation_id: "cursor", generation_id: "one" };
      await runCursorHook({ ...common, hook_event_name: "sessionStart" }, options);
      await runCursorHook(
        {
          ...common,
          hook_event_name: "preToolUse",
          tool_use_id: "failed",
          tool_name: "Write",
          tool_input: { file_path: "file.txt" },
        },
        options,
      );
      await runCursorHook(
        {
          ...common,
          hook_event_name: "postToolUseFailure",
          tool_use_id: "failed",
          tool_name: "Write",
          error_message: "The requested operation failed",
        },
        options,
      );
      const recovery = await cursorRelayed(steerText("recovery", "CURSOR_RECOVERY", undefined, { kind: "tool", tool: "Write" }), "recovery", common.workspace_roots[0]!);
      let pollCount = 0;
      await expect.poll(
        async () => {
          const tool = {
            ...common,
            tool_use_id: `retry:${++pollCount}`,
            tool_name: "Write",
            tool_input: { file_path: "file.txt" },
          };
          expect(await runCursorHook({ ...tool, hook_event_name: "preToolUse" }, options))
            .toEqual({ permission: "allow" });
          return runCursorHook({ ...tool, hook_event_name: "postToolUse", tool_output: "written" }, options);
        },
        { timeout: 10_000, interval: 10 },
      ).toEqual({ additional_context: recovery });
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("observes Cursor Read and Write phases sharing one native tool call ID", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-cursor-phases-")));
    seedContractCodeRule(isolatedRerailHome, root);
    const socketPath = join(root, "engine.sock");
    const storageRoot = join(root, ".rerail/sessions");
    let service: { close(): Promise<void> } | undefined;
    try {
      await mkdir(join(root, "api"));
      const file = join(root, "api/route.py");
      await writeFile(file, "def route():\n    return 1\n");
      const input = { content: "def route(cursor):\n    cursor.execute('SELECT 1')\n", file_path: file };
      addConversationRules(root, [
        {
          id: "write-call",
          mode: "steer",
          reference: JSON.stringify(input),
          match: { operator: "gte", threshold: 0.95 },
          sample: { source: "tool:Write", boundary: "tool-call", minimumTokens: 1 },
          action: { content: "CURSOR_WRITE_CALL" },
          repeat: { mode: "once" },
        },
        {
          id: "write-result",
          mode: "steer",
          reference: "The route was written",
          match: { operator: "gte", threshold: 0.95 },
          sample: { source: "tool:Write", boundary: "tool-call", minimumTokens: 1 },
          action: { content: "CURSOR_WRITE_RESULT" },
          repeat: { mode: "once" },
        },
      ]);
      service = await startEngineService({ socketPath, storageRoot, modelPath: resolve("model") });
      const options = { storageRoot, modelPath: resolve("model"), kernelSocketPath: socketPath };
      const common = {
        workspace_roots: [root],
        conversation_id: "cursor-phases",
        generation_id: "one",
        tool_use_id: "native-edit",
      };
      await runCursorHook({ ...common, hook_event_name: "sessionStart" }, options);
      await runCursorHook(
        { ...common, hook_event_name: "preToolUse", tool_name: "Read", tool_input: { file_path: file } },
        options,
      );
      await runCursorHook(
        { ...common, hook_event_name: "postToolUse", tool_name: "Read", tool_output: "def route():\n    return 1\n" },
        options,
      );
      const outputs = [
        await runCursorHook(
          { ...common, hook_event_name: "preToolUse", tool_name: "Write", tool_input: input },
          options,
        ),
      ];
      await writeFile(file, input.content);
      outputs.push(await runCursorHook(
        { ...common, hook_event_name: "postToolUse", tool_name: "Write", tool_output: "The route was written" },
        options,
      ));
      let pollCount = 0;
      await expect.poll(async () => {
        const read = { ...common, tool_use_id: `code-rule-poll:${++pollCount}`, tool_name: "Read" };
        outputs.push(await runCursorHook({ ...read, hook_event_name: "preToolUse", tool_input: { file_path: file } }, options));
        outputs.push(await runCursorHook({ ...read, hook_event_name: "postToolUse", tool_output: input.content }, options));
        const content = outputs.flatMap(Object.values).filter((value): value is string => typeof value === "string").join("\n");
        return ["CURSOR_WRITE_CALL", "CURSOR_WRITE_RESULT", CONTRACT_CURSOR_EXECUTE_RULE_ID].filter(marker => !content.includes(marker));
      }, { timeout: 10_000, interval: 10 }).toEqual([]);
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it("shows a turn-boundary conversation rule's line at Codex's Stop and gives its steer with the next message", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-codex-stop-rule-"));
    const socketPath = join(root, "engine.sock");
    const storageRoot = join(root, ".rerail/sessions");
    let service: { close(): Promise<void> } | undefined;
    try {
      addConversationRules(root, [
        {
          id: "turn-steer",
          mode: "steer",
          reference: "Preserve database transaction atomicity",
          match: { operator: "gte", threshold: 0.95 },
          sample: { source: "assistant-text", boundary: "turn", minimumTokens: 1 },
          action: { content: "CONTINUE_AFTER_STOP" },
          repeat: { mode: "once" },
        },
      ]);
      service = await startEngineService({ socketPath, storageRoot, modelPath: resolve("model") });
      const options = { storageRoot, modelPath: resolve("model"), kernelSocketPath: socketPath };
      const common = { cwd: root, session_id: "stop-rule", turn_id: "turn" };
      await runCodexHook({ ...common, hook_event_name: "SessionStart", source: "startup" }, options);
      const content = steerText("turn-steer", "CONTINUE_AFTER_STOP");
      expect(
        await runCodexHook(
          { ...common, hook_event_name: "Stop", last_assistant_message: "Preserve database transaction atomicity" },
          options,
        ),
      ).toEqual({ systemMessage: await steerLine("turn-steer", root, "plain") });
      expect(await runCodexHook({ ...common, turn_id: "turn-2", hook_event_name: "UserPromptSubmit", prompt: "next" }, options))
        .toEqual({ hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext: content } });
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("does not launch a daemon when hook autostart is explicitly disabled", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-no-autostart-"));
    const home = join(root, "daemon-home");
    const previous = {
      RERAIL_HOME: process.env.RERAIL_HOME,
      RERAIL_AUTOSTART: process.env.RERAIL_AUTOSTART,
      RERAIL_ENGINE_SOCKET: process.env.RERAIL_ENGINE_SOCKET,
    };
    process.env.RERAIL_HOME = home;
    process.env.RERAIL_AUTOSTART = "0";
    delete process.env.RERAIL_ENGINE_SOCKET;
    startTestRepository(root);
    try {
      // Codex has no footer, so a session that starts without a daemon says so in its one line.
      expect(await runCodexHook(
        { session_id: "no-autostart", cwd: root, hook_event_name: "SessionStart", source: "startup" },
        { storageRoot: join(root, "sessions"), modelPath: resolve("model") },
      )).toEqual({ systemMessage: "Rerail ✕ not running · start it with: rerail daemon start" });
      // The home holds only the database recording the repository start: no daemon state, log, or start lock.
      expect((await readdir(home)).filter(entry => !entry.startsWith("rerail.sqlite"))).toEqual([]);
    } finally {
      await stopDaemon(home);
      for (const [key, value] of Object.entries(previous)) {
        if (value === undefined) delete process.env[key];
        else process.env[key] = value;
      }
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("recovers real Cursor transcript records after the host omits prompt and response hooks", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-cursor-transcript-")));
    const socketPath = join(root, "engine.sock");
    const storageRoot = join(root, ".rerail/sessions");
    const databasePath = join(root, ".rerail/rerail.sqlite");
    const transcriptPath = join(root, "cursor-transcript.jsonl");
    const marker =
      "PRIVATE_CURSOR_TRANSCRIPT_CONVERSATION\n<timestamp>literal</timestamp>\n<user_query>\nPreserve these user-supplied tags.\n</user_query>";
    let service: { close(): Promise<void> } | undefined;
    try {
      addConversationRules(root, [
        {
          id: "transcript-prompt",
          mode: "steer",
          reference: marker,
          match: { operator: "gte", threshold: 0.95 },
          sample: { source: "user-input", boundary: "message", minimumTokens: 1 },
          action: { content: marker },
          repeat: { mode: "once" },
        },
      ], databasePath);
      service = await startEngineService({ socketPath, storageRoot, modelPath: resolve("model") });
      const options = {
        storageRoot,
        databasePath,
        modelPath: resolve("model"),
        kernelSocketPath: socketPath,
      };
      const common = { workspace_roots: [root], conversation_id: "cursor-transcript", generation_id: "one" };
      const userRecord = JSON.stringify({
        role: "user",
        message: {
          content: [
            {
              type: "text",
              text: `<timestamp>Monday, September 7, 2026</timestamp>\n<user_query>\n${marker}\n</user_query>`,
            },
          ],
        },
      });
      const assistantText = `${marker} in the real assistant response from Cursor`;
      const assistantRecord = JSON.stringify({
        role: "assistant",
        message: {
          content: [
            { type: "text", text: assistantText },
            { type: "tool_use", id: "real-native-tool-id", name: "Read", input: { path: "api/route.py" } },
          ],
        },
      });

      await runCursorHook({ ...common, hook_event_name: "sessionStart", transcript_path: null }, options);
      const split = Math.floor(userRecord.length / 2);
      await writeFile(transcriptPath, userRecord.slice(0, split));
      expect(
        await runCursorHook(
          {
            ...common,
            hook_event_name: "postToolUse",
            transcript_path: transcriptPath,
            tool_use_id: "real-native-tool-id",
            tool_name: "Read",
            tool_output: "partial transcript snapshot",
          },
          options,
        ),
      ).toEqual({});

      await appendFile(transcriptPath, `${userRecord.slice(split)}\n`);
      expect(
        await runCursorHook(
          {
            ...common,
            hook_event_name: "postToolUse",
            transcript_path: transcriptPath,
            tool_use_id: "real-native-tool-id",
            tool_name: "Read",
            tool_output: "complete transcript snapshot",
          },
          options,
        ),
      ).toEqual({ additional_context: await cursorRelayed(steerText("transcript-prompt", marker), "transcript-prompt", common.workspace_roots[0]!) });
      expect(
        await runCursorHook(
          {
            ...common,
            hook_event_name: "postToolUse",
            transcript_path: transcriptPath,
            tool_use_id: "real-native-tool-id",
            tool_name: "Read",
            tool_output: "replayed transcript snapshot",
          },
          options,
        ),
      ).toEqual({});

      await appendFile(
        transcriptPath,
        `${assistantRecord}\n${JSON.stringify({ type: "turn_ended", status: "success" })}\n`,
      );
      expect(
        await runCursorHook({ ...common, hook_event_name: "sessionEnd", transcript_path: transcriptPath }, options),
      ).toEqual({});

      const reader = new SqliteSessionReader(databasePath);
      const events = reader.events("cursor-transcript");
      expect(events.filter((event) => event.kind === "user-prompt")).toHaveLength(1);
      expect(events.filter((event) => event.kind === "assistant-message")).toEqual([
        expect.objectContaining({ payload: expect.objectContaining({ text: assistantText }) }),
      ]);
      expect(events.filter((event) => event.kind === "turn-end")).toEqual([
        expect.objectContaining({ payload: expect.objectContaining({ text: assistantText }) }),
      ]);
      reader.close();
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it("keeps native Cursor prompt mode authoritative when its transcript repeats the prompt", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-cursor-native-")));
    const socketPath = join(root, "engine.sock");
    const storageRoot = join(root, ".rerail/sessions");
    const databasePath = join(root, ".rerail/rerail.sqlite");
    const transcriptPath = join(root, "cursor-transcript.jsonl");
    const marker = "PRIVATE_CURSOR_NATIVE_MODE_CONVERSATION";
    let service: { close(): Promise<void> } | undefined;
    try {
      addConversationRules(root, [
        {
          id: "native-prompt",
          mode: "steer",
          reference: marker,
          match: { operator: "gte", threshold: 0.95 },
          sample: { source: "user-input", boundary: "message", minimumTokens: 1 },
          action: { content: marker },
          repeat: { mode: "once" },
        },
      ], databasePath);
      service = await startEngineService({ socketPath, storageRoot, modelPath: resolve("model") });
      const options = {
        storageRoot,
        databasePath,
        modelPath: resolve("model"),
        kernelSocketPath: socketPath,
      };
      const common = { workspace_roots: [root], conversation_id: "cursor-native-mode", generation_id: "one" };
      await writeFile(
        transcriptPath,
        `${JSON.stringify({
          role: "user",
          message: {
            content: [
              {
                type: "text",
                text: `<timestamp>Monday, September 7, 2026</timestamp>\n<user_query>\n${marker}\n</user_query>`,
              },
            ],
          },
        })}\n`,
      );

      await runCursorHook({ ...common, hook_event_name: "sessionStart", transcript_path: null }, options);
      expect(
        await runCursorHook(
          {
            ...common,
            hook_event_name: "beforeSubmitPrompt",
            transcript_path: transcriptPath,
            prompt: marker,
          },
          options,
        ),
      ).toEqual({ continue: true });
      expect(
        await runCursorHook(
          {
            ...common,
            hook_event_name: "postToolUse",
            transcript_path: transcriptPath,
            tool_use_id: "native-read",
            tool_name: "Read",
            tool_output: "native prompt mode",
          },
          options,
        ),
      ).toEqual({ additional_context: await cursorRelayed(steerText("native-prompt", marker), "native-prompt", common.workspace_roots[0]!) });
      expect(
        await runCursorHook(
          {
            ...common,
            hook_event_name: "postToolUse",
            transcript_path: transcriptPath,
            tool_use_id: "native-read",
            tool_name: "Read",
            tool_output: "native prompt mode replay",
          },
          options,
        ),
      ).toEqual({});
      await runCursorHook({ ...common, hook_event_name: "sessionEnd", transcript_path: transcriptPath }, options);

      const reader = new SqliteSessionReader(databasePath);
      expect(reader.events("cursor-native-mode").filter((event) => event.kind === "user-prompt")).toHaveLength(1);
      reader.close();
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
});
