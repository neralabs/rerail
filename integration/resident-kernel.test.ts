import { appendFile, mkdtemp, mkdir, realpath, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { ingestKernelEvent, startKernelService } from "../src/kernel-service.js";
import { runCodexHook } from "../src/codex-hook.js";
import { RerailDaemon } from "../src/daemon.js";
import { runCursorHook } from "../src/cursor-hook.js";
import { stopDaemon } from "../src/daemon-control.js";
import type { AdapterDelivery } from "../src/adapter-contract.js";
import { SqliteTranscriptReader } from "../src/transcript-store.js";
import { seedContractPolicy } from "./contract-policy.js";
const originalRerailHome = process.env.RERAIL_HOME;
let isolatedRerailHome: string;

beforeAll(async () => {
  isolatedRerailHome = await mkdtemp(join(tmpdir(), "rerail-resident-home-"));
  process.env.RERAIL_HOME = isolatedRerailHome;
  seedContractPolicy(isolatedRerailHome, isolatedRerailHome);
});

afterAll(async () => {
  if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = originalRerailHome;
  await rm(isolatedRerailHome, { recursive: true, force: true });
});

async function awaitPolicySteer(
  initial: readonly AdapterDelivery[],
  poll: (eventId: string) => Promise<readonly AdapterDelivery[]>,
  content: string,
): Promise<readonly AdapterDelivery[]> {
  const deliveries = [...initial];
  let pollCount = 0;
  await expect.poll(async () => {
    deliveries.push(...await poll(`policy-poll:${++pollCount}`));
    return deliveries.filter(delivery => delivery.requested === "steer" && delivery.content.includes(content));
  }, { timeout: 10_000, interval: 10 }).toEqual([
    expect.objectContaining({ requested: "steer", content: expect.stringContaining(content) }),
  ]);
  return deliveries.filter(delivery => delivery.requested === "steer" && delivery.content.includes(content));
}

describe("resident kernel service", () => {
  it("preserves executable-policy correlation across one-shot hook connections", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-resident-kernel-"));
    const socketPath = join(root, "kernel.sock");
    const storageRoot = join(root, ".rerail", "sessions");
    let service: { close(): Promise<void> } | undefined;
    const session = { host: "omp" as const, sessionId: "resident-kernel", cwd: root };

    try {
      await mkdir(join(root, "api"));
      await writeFile(join(root, "api", "route.py"), "def route():\n    return 1\n");
      service = await startKernelService({ socketPath, storageRoot, modelPath: resolve("model") });

      await ingestKernelEvent(socketPath, { kind: "session-start", eventId: "start", session });
      await ingestKernelEvent(socketPath, {
        kind: "tool-call",
        eventId: "call:write-route",
        toolCallId: "write-route",
        sessionId: session.sessionId,
        toolName: "write",
        input: { path: "api/route.py" },
      });

      await writeFile(join(root, "api", "route.py"), 'def route(cursor):\n    cursor.execute("SELECT 1")\n');
      const deliveries = await ingestKernelEvent(socketPath, {
        kind: "tool-result",
        eventId: "result:write-route",
        toolCallId: "write-route",
        sessionId: session.sessionId,
        toolName: "write",
        output: { ok: true },
      });

      expect(await awaitPolicySteer(
        deliveries,
        eventId => ingestKernelEvent(socketPath, {
          kind: "user-prompt",
          eventId,
          sessionId: session.sessionId,
          text: "continue",
        }),
        "CURSOR_EXECUTE",
      )).toEqual([
        expect.objectContaining({
          sessionId: session.sessionId,
          mechanism: "native-queue",
          content: expect.stringContaining("CURSOR_EXECUTE"),
        }),
      ]);
      await ingestKernelEvent(socketPath, { kind: "session-end", eventId: "end", sessionId: session.sessionId });
    } finally {
      await service?.close();
      await rm(socketPath, { force: true });
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("keeps policy state when each Codex hook invocation closes its client", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-resident-hooks-"));
    const socketPath = join(root, "kernel.sock");
    const storageRoot = join(root, ".rerail/sessions");
    let service: { close(): Promise<void> } | undefined;
    try {
      await mkdir(join(root, "api"));
      await writeFile(join(root, "api/route.py"), "def route():\n    return 1\n");
      service = await startKernelService({ socketPath, storageRoot, modelPath: resolve("model") });
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
      expect(initial).toEqual({});
      expect(await runCodexHook(
        {
          ...common,
          hook_event_name: "Stop",
          turn_id: "turn-1",
          stop_hook_active: false,
          last_assistant_message: "Done.",
        },
        options,
      )).toEqual({
        decision: "block",
        reason: expect.stringContaining("CURSOR_EXECUTE"),
      });
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("serves the native execution kernel for hooks throughout the daemon lifetime", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-daemon-kernel-"));
    let daemon: RerailDaemon | undefined;
    try {
      await mkdir(join(root, ".claude"));
      await writeFile(
        join(root, ".claude/cosine-rules.json"),
        JSON.stringify({
          version: 1,
          rules: [
            {
              id: "resident",
              reference: "Preserve database transaction atomicity",
              match: { operator: "gte", threshold: 0.95 },
              sample: { source: "user-input", boundary: "message", minimumTokens: 1 },
              action: { delivery: "steer", content: "DAEMON_KERNEL_MATCH" },
              repeat: { mode: "once" },
            },
          ],
        }),
      );
      daemon = await RerailDaemon.start({
        version: "test",
        port: 0,
        databasePath: join(root, ".rerail/rerail.sqlite"),
        workspaceRoot: root,
        repositoryContext: { repositoryId: "resident-kernel-test", workspaceRoot: root },
      });
      const session = { host: "claude-code" as const, sessionId: "daemon-session", cwd: root };
      await ingestKernelEvent(daemon.state().kernelSocketPath, { kind: "session-start", eventId: "start", session });
      const deliveries = await ingestKernelEvent(daemon.state().kernelSocketPath, {
        kind: "user-prompt",
        eventId: "prompt",
        sessionId: session.sessionId,
        text: "Preserve database transaction atomicity",
      });
      const socketPath = daemon.state().kernelSocketPath;
      expect(await awaitPolicySteer(
        deliveries,
        eventId => ingestKernelEvent(socketPath, {
          kind: "user-prompt",
          eventId,
          sessionId: session.sessionId,
          text: "continue",
        }),
        "DAEMON_KERNEL_MATCH",
      )).toEqual([
        expect.objectContaining({ content: "DAEMON_KERNEL_MATCH", mechanism: "additional-context" }),
      ]);
    } finally {
      await daemon?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("retains captured pre-edit source when the daemon restarts during a tool call", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-kernel-restart-"));
    const socketPath = join(root, "kernel.sock");
    const options = { socketPath, storageRoot: join(root, ".rerail/sessions"), modelPath: resolve("model") };
    let service: { close(): Promise<void> } | undefined;
    const session = { host: "codex" as const, sessionId: "restart", cwd: root };
    try {
      await mkdir(join(root, "api"));
      await writeFile(join(root, "api/route.py"), "def route():\n    return 1\n");
      service = await startKernelService(options);
      await ingestKernelEvent(socketPath, { kind: "session-start", eventId: "start", session });
      await ingestKernelEvent(socketPath, {
        kind: "tool-call",
        eventId: "call",
        toolCallId: "write",
        sessionId: session.sessionId,
        toolName: "write",
        input: { path: "api/route.py" },
      });
      await service.close();
      service = undefined;
      await writeFile(join(root, "api/route.py"), "def route(cursor):\n    cursor.execute('SELECT 1')\n");
      service = await startKernelService(options);
      const deliveries = await ingestKernelEvent(socketPath, {
        kind: "tool-result",
        eventId: "result",
        toolCallId: "write",
        sessionId: session.sessionId,
        session,
        toolName: "write",
        output: "written",
      });
      expect(await awaitPolicySteer(
        deliveries,
        eventId => ingestKernelEvent(socketPath, {
          kind: "user-prompt",
          eventId,
          sessionId: session.sessionId,
          session,
          text: "continue",
        }),
        "CURSOR_EXECUTE",
      )).toEqual([expect.objectContaining({ content: expect.stringContaining("CURSOR_EXECUTE") })]);
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("isolates identical session IDs in different projects served by one daemon", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-kernel-projects-"));
    const socketPath = join(root, "kernel.sock");
    let service: { close(): Promise<void> } | undefined;
    try {
      service = await startKernelService({
        socketPath,
        storageRoot: join(root, "default"),
        modelPath: resolve("model"),
      });
      for (const project of ["first", "second"]) {
        const cwd = join(root, project);
        await mkdir(join(cwd, ".claude"), { recursive: true });
        await writeFile(
          join(cwd, ".claude/cosine-rules.json"),
          JSON.stringify({
            version: 1,
            rules: [
              {
                id: "project-rule",
                reference: "Preserve database transaction atomicity",
                match: { operator: "gte", threshold: 0.95 },
                sample: { source: "user-input", boundary: "message", minimumTokens: 1 },
                action: { delivery: "steer", content: project },
                repeat: { mode: "once" },
              },
            ],
          }),
        );
        const configuration = {
          storageRoot: join(cwd, ".rerail/sessions"),
          modelPath: resolve("model"),
          databasePath: join(cwd, ".rerail/rerail.sqlite"),
        };
        const session = { host: "claude-code" as const, sessionId: "same-id", cwd };
        await ingestKernelEvent(socketPath, { kind: "session-start", eventId: "start", session }, configuration);
        const deliveries = await ingestKernelEvent(
          socketPath,
          {
            kind: "user-prompt",
            eventId: "prompt",
            sessionId: session.sessionId,
            text: "Preserve database transaction atomicity",
          },
          configuration,
        );
        expect(await awaitPolicySteer(
          deliveries,
          eventId => ingestKernelEvent(socketPath, {
            kind: "user-prompt",
            eventId: `${project}:${eventId}`,
            sessionId: session.sessionId,
            session,
            text: "continue",
          }, configuration),
          project,
        )).toEqual([expect.objectContaining({ content: project, sessionId: session.sessionId })]);
      }
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("delivers a configured Codex follow-up even when Stop has no assistant message", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-codex-cosine-"));
    const socketPath = join(root, "kernel.sock");
    const storageRoot = join(root, ".rerail/sessions");
    let service: { close(): Promise<void> } | undefined;
    try {
      await mkdir(join(root, ".codex"));
      await writeFile(
        join(root, ".codex/cosine-rules.json"),
        JSON.stringify({
          version: 1,
          rules: [
            {
              id: "follow-up",
              reference: "Preserve database transaction atomicity",
              match: { operator: "gte", threshold: 0.95 },
              sample: { source: "user-input", boundary: "message", minimumTokens: 1 },
              action: { delivery: "followUp", content: "CODEX_FOLLOW_UP" },
              repeat: { mode: "once" },
            },
          ],
        }),
      );
      service = await startKernelService({ socketPath, storageRoot, modelPath: resolve("model") });
      const options = { storageRoot, modelPath: resolve("model"), kernelSocketPath: socketPath };
      const common = { cwd: root, session_id: "codex-cosine", turn_id: "turn" };
      await runCodexHook({ ...common, hook_event_name: "SessionStart", source: "startup" }, options);
      expect(
        await runCodexHook(
          { ...common, hook_event_name: "UserPromptSubmit", prompt: "Preserve database transaction atomicity" },
          options,
        ),
      ).toEqual({});
      expect(await runCodexHook({ ...common, hook_event_name: "Stop", last_assistant_message: null }, options)).toEqual(
        { decision: "block", reason: "CODEX_FOLLOW_UP" },
      );
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("defers Cursor failure-event steering to the next supported tool context", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-cursor-delivery-"));
    const socketPath = join(root, "kernel.sock");
    const storageRoot = join(root, ".rerail/sessions");
    let service: { close(): Promise<void> } | undefined;
    try {
      await mkdir(join(root, ".cursor"));
      await writeFile(
        join(root, ".cursor/cosine-rules.json"),
        JSON.stringify({
          version: 1,
          rules: [
            {
              id: "recovery",
              reference: "The requested operation failed",
              match: { operator: "gte", threshold: 0.95 },
              sample: { source: "tool:Write", boundary: "tool-call", minimumTokens: 1 },
              action: { delivery: "steer", content: "CURSOR_RECOVERY" },
              repeat: { mode: "once" },
            },
          ],
        }),
      );
      service = await startKernelService({ socketPath, storageRoot, modelPath: resolve("model") });
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
      ).toEqual({ additional_context: "CURSOR_RECOVERY" });
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("observes Cursor Read and Write phases sharing one native tool call ID", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-cursor-phases-")));
    const socketPath = join(root, "kernel.sock");
    const storageRoot = join(root, ".rerail/sessions");
    let service: { close(): Promise<void> } | undefined;
    try {
      await mkdir(join(root, ".cursor"));
      await mkdir(join(root, "api"));
      const file = join(root, "api/route.py");
      await writeFile(file, "def route():\n    return 1\n");
      const input = { content: "def route(cursor):\n    cursor.execute('SELECT 1')\n", file_path: file };
      await writeFile(
        join(root, ".cursor/cosine-rules.json"),
        JSON.stringify({
          version: 1,
          rules: [
            {
              id: "write-call",
              reference: JSON.stringify(input),
              match: { operator: "gte", threshold: 0.95 },
              sample: { source: "tool:Write", boundary: "tool-call", minimumTokens: 1 },
              action: { delivery: "steer", content: "CURSOR_WRITE_CALL" },
              repeat: { mode: "once" },
            },
            {
              id: "write-result",
              reference: "The route was written",
              match: { operator: "gte", threshold: 0.95 },
              sample: { source: "tool:Write", boundary: "tool-call", minimumTokens: 1 },
              action: { delivery: "steer", content: "CURSOR_WRITE_RESULT" },
              repeat: { mode: "once" },
            },
          ],
        }),
      );
      service = await startKernelService({ socketPath, storageRoot, modelPath: resolve("model") });
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
        const read = { ...common, tool_use_id: `policy-poll:${++pollCount}`, tool_name: "Read" };
        outputs.push(await runCursorHook({ ...read, hook_event_name: "preToolUse", tool_input: { file_path: file } }, options));
        outputs.push(await runCursorHook({ ...read, hook_event_name: "postToolUse", tool_output: input.content }, options));
        const content = outputs.flatMap(Object.values).filter((value): value is string => typeof value === "string").join("\n");
        return ["CURSOR_WRITE_CALL", "CURSOR_WRITE_RESULT", "CURSOR_EXECUTE"].filter(marker => !content.includes(marker));
      }, { timeout: 10_000, interval: 10 }).toEqual([]);
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("holds Cursor next-turn delivery until the next prompt's first tool context", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-cursor-next-turn-"));
    const socketPath = join(root, "kernel.sock");
    const storageRoot = join(root, ".rerail/sessions");
    let service: { close(): Promise<void> } | undefined;
    try {
      await mkdir(join(root, ".cursor"));
      await writeFile(
        join(root, ".cursor/cosine-rules.json"),
        JSON.stringify({
          version: 1,
          rules: [
            {
              id: "next",
              reference: "Preserve database transaction atomicity",
              match: { operator: "gte", threshold: 0.95 },
              sample: { source: "user-input", boundary: "message", minimumTokens: 1 },
              action: { delivery: "nextTurn", content: "CURSOR_NEXT_TURN" },
              repeat: { mode: "once" },
            },
          ],
        }),
      );
      service = await startKernelService({ socketPath, storageRoot, modelPath: resolve("model") });
      const options = { storageRoot, modelPath: resolve("model"), kernelSocketPath: socketPath };
      const common = { workspace_roots: [root], conversation_id: "cursor-next" };
      await runCursorHook({ ...common, hook_event_name: "sessionStart" }, options);
      await runCursorHook(
        {
          ...common,
          generation_id: "first",
          hook_event_name: "beforeSubmitPrompt",
          prompt: "Preserve database transaction atomicity",
        },
        options,
      );
      expect(
        await runCursorHook(
          {
            ...common,
            generation_id: "first",
            hook_event_name: "preToolUse",
            tool_use_id: "first",
            tool_name: "Read",
            tool_input: { path: "file.txt" },
          },
          options,
        ),
      ).toEqual({ permission: "allow" });
      expect(await runCursorHook({ ...common, generation_id: "first", hook_event_name: "stop" }, options)).toEqual({});
      await runCursorHook(
        {
          ...common,
          generation_id: "second",
          hook_event_name: "beforeSubmitPrompt",
          prompt: "Preserve database transaction atomicity",
        },
        options,
      );
      expect(
        await runCursorHook(
          {
            ...common,
            generation_id: "second",
            hook_event_name: "preToolUse",
            tool_use_id: "second",
            tool_name: "Read",
            tool_input: { path: "file.txt" },
          },
          options,
        ),
      ).toEqual({ permission: "allow" });
      expect(await runCursorHook(
        {
          ...common,
          generation_id: "second",
          hook_event_name: "postToolUse",
          tool_use_id: "second",
          tool_name: "Read",
          tool_output: "file contents",
        },
        options,
      )).toEqual({ additional_context: "CURSOR_NEXT_TURN" });
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
  it("starts a Codex continuation when a turn-boundary cosine rule fires at Stop", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-codex-stop-rule-"));
    const socketPath = join(root, "kernel.sock");
    const storageRoot = join(root, ".rerail/sessions");
    let service: { close(): Promise<void> } | undefined;
    try {
      await mkdir(join(root, ".codex"));
      await writeFile(
        join(root, ".codex/cosine-rules.json"),
        JSON.stringify({
          version: 1,
          rules: [
            {
              id: "turn-follow-up",
              reference: "Preserve database transaction atomicity",
              match: { operator: "gte", threshold: 0.95 },
              sample: { source: "assistant-text", boundary: "turn", minimumTokens: 1 },
              action: { delivery: "followUp", content: "CONTINUE_AFTER_STOP" },
              repeat: { mode: "once" },
            },
          ],
        }),
      );
      service = await startKernelService({ socketPath, storageRoot, modelPath: resolve("model") });
      const options = { storageRoot, modelPath: resolve("model"), kernelSocketPath: socketPath };
      const common = { cwd: root, session_id: "stop-rule", turn_id: "turn" };
      await runCodexHook({ ...common, hook_event_name: "SessionStart", source: "startup" }, options);
      expect(
        await runCodexHook(
          { ...common, hook_event_name: "Stop", last_assistant_message: "Preserve database transaction atomicity" },
          options,
        ),
      ).toEqual({ decision: "block", reason: "CONTINUE_AFTER_STOP" });
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
      RERAIL_KERNEL_SOCKET: process.env.RERAIL_KERNEL_SOCKET,
    };
    process.env.RERAIL_HOME = home;
    process.env.RERAIL_AUTOSTART = "0";
    delete process.env.RERAIL_KERNEL_SOCKET;
    try {
      await expect(
        runCodexHook(
          { session_id: "no-autostart", cwd: root, hook_event_name: "SessionStart", source: "startup" },
          { storageRoot: join(root, "sessions"), modelPath: resolve("model") },
        ),
      ).rejects.toThrow("RERAIL_AUTOSTART");
      await expect(stat(home)).rejects.toMatchObject({ code: "ENOENT" });
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
    const socketPath = join(root, "kernel.sock");
    const storageRoot = join(root, ".rerail/sessions");
    const databasePath = join(root, ".rerail/rerail.sqlite");
    const transcriptPath = join(root, "cursor-transcript.jsonl");
    const marker =
      "PRIVATE_CURSOR_TRANSCRIPT_COSINE\n<timestamp>literal</timestamp>\n<user_query>\nPreserve these user-supplied tags.\n</user_query>";
    let service: { close(): Promise<void> } | undefined;
    try {
      await mkdir(join(root, ".cursor"));
      await writeFile(
        join(root, ".cursor/cosine-rules.json"),
        JSON.stringify({
          version: 1,
          rules: [
            {
              id: "transcript-prompt",
              reference: marker,
              match: { operator: "gte", threshold: 0.95 },
              sample: { source: "user-input", boundary: "message", minimumTokens: 1 },
              action: { delivery: "steer", content: marker },
              repeat: { mode: "once" },
            },
          ],
        }),
      );
      service = await startKernelService({ socketPath, storageRoot, modelPath: resolve("model") });
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
      ).toEqual({ additional_context: marker });
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

      const reader = new SqliteTranscriptReader(databasePath);
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
    const socketPath = join(root, "kernel.sock");
    const storageRoot = join(root, ".rerail/sessions");
    const databasePath = join(root, ".rerail/rerail.sqlite");
    const transcriptPath = join(root, "cursor-transcript.jsonl");
    const marker = "PRIVATE_CURSOR_NATIVE_MODE_COSINE";
    let service: { close(): Promise<void> } | undefined;
    try {
      await mkdir(join(root, ".cursor"));
      await writeFile(
        join(root, ".cursor/cosine-rules.json"),
        JSON.stringify({
          version: 1,
          rules: [
            {
              id: "native-prompt",
              reference: marker,
              match: { operator: "gte", threshold: 0.95 },
              sample: { source: "user-input", boundary: "message", minimumTokens: 1 },
              action: { delivery: "steer", content: marker },
              repeat: { mode: "once" },
            },
          ],
        }),
      );
      service = await startKernelService({ socketPath, storageRoot, modelPath: resolve("model") });
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
      ).toEqual({ additional_context: marker });
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

      const reader = new SqliteTranscriptReader(databasePath);
      expect(reader.events("cursor-native-mode").filter((event) => event.kind === "user-prompt")).toHaveLength(1);
      reader.close();
    } finally {
      await service?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
});
