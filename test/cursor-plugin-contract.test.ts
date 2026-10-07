import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { z } from "zod";
import type { AdapterDelivery, AdapterEvent, RerailEventLoop } from "../src/adapter-contract.js";
import { cursorHookEvent, cursorHookOutput, handleCursorEvent } from "../src/cursor-hook.js";

async function json(path: string): Promise<unknown> {
  return JSON.parse(await readFile(path, "utf8")) as unknown;
}

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryDirectory(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "rerail-cursor-hook-"));
  temporaryDirectories.push(path);
  return path;
}

function delivery(requested: AdapterDelivery["requested"], content: string, ruleIds: readonly string[] = []): AdapterDelivery {
  return { sessionId: "cursor-session", requested, mechanism: "deferred-context", content, ruleIds };
}

function scriptedLoop(script: Readonly<Record<string, readonly AdapterDelivery[]>>): RerailEventLoop & { events: AdapterEvent[] } {
  const events: AdapterEvent[] = [];
  return {
    events,
    async ingest(event) {
      events.push(event);
      return script[event.kind] ?? [];
    },
  };
}

const ManifestSchema = z.object({
  name: z.literal("rerail"),
  hooks: z.literal("./hooks/cursor.json"),
});
const HooksSchema = z.object({
  version: z.literal(1),
  hooks: z.record(z.string(), z.array(z.object({ command: z.string() }))),
});

describe("Cursor plugin contract", () => {
  it("uses a relative packaged command for every supported hook", async () => {
    expect(ManifestSchema.parse(await json(".cursor-plugin/plugin.json"))).toMatchObject({ name: "rerail" });
    const hooks = HooksSchema.parse(await json("hooks/cursor.json"));
    expect(Object.keys(hooks.hooks).sort()).toEqual([
      "afterAgentResponse",
      "beforeSubmitPrompt",
      "postToolUse",
      "postToolUseFailure",
      "preToolUse",
      "sessionEnd",
      "sessionStart",
      "stop",
    ]);
    for (const handlers of Object.values(hooks.hooks)) {
      expect(handlers).toEqual([{ command: "node ./dist/cursor-hook.js" }]);
    }
  });

  it("normalizes documented Cursor hook payloads and supported delivery outputs", () => {
    const common = {
      conversation_id: "cursor-session",
      generation_id: "generation-1",
      model: "cursor-model",
      cursor_version: "1.7.2",
      workspace_roots: ["/workspace"],
      user_email: null,
      transcript_path: null,
    };
    expect(cursorHookEvent({
      ...common,
      hook_event_name: "beforeSubmitPrompt",
      prompt: "Preserve session state",
      attachments: [],
    })).toEqual({
      kind: "user-prompt",
      eventId: "cursor-session:prompt:generation-1",
      sessionId: "cursor-session",
      session: { host: "cursor", sessionId: "cursor-session", cwd: "/workspace" },
      text: "Preserve session state",
    });
    expect(cursorHookEvent({
      ...common,
      hook_event_name: "postToolUse",
      tool_name: "Write",
      tool_input: { path: "src/a.ts" },
      tool_output: "written",
      tool_use_id: "tool-1",
      cwd: "/workspace",
    })).toMatchObject({
      kind: "tool-result",
      toolCallId: "tool-1",
      toolName: "Write",
      output: "written",
      failed: false,
    });

    const delivery = {
      sessionId: "cursor-session",
      requested: "steer" as const,
      mechanism: "deferred-context" as const,
      content: "Preserve the session.",
      ruleIds: [],
    };
    expect(cursorHookOutput("beforeSubmitPrompt", [delivery])).toEqual({ continue: true });
    expect(cursorHookOutput("postToolUse", [delivery])).toEqual({ additional_context: "Preserve the session." });
    expect(cursorHookOutput("stop", [delivery])).toEqual({ followup_message: "Preserve the session." });
  });

  it("maps Cursor session and assistant lifecycle hooks to the shared transcript event vocabulary", () => {
    const common = { conversation_id: "cursor-session", generation_id: "generation-1", workspace_roots: ["/workspace"] };

    expect(cursorHookEvent({ ...common, hook_event_name: "sessionStart" })).toMatchObject({
      kind: "session-start",
      eventId: "cursor-session:session-start:generation-1",
      session: { host: "cursor", sessionId: "cursor-session", cwd: "/workspace" },
    });
    expect(cursorHookEvent({ ...common, hook_event_name: "afterAgentResponse", text: "Completed." })).toMatchObject({
      kind: "assistant-message",
      eventId: "cursor-session:assistant:generation-1",
      messageId: "generation-1",
      text: "Completed.",
    });
    expect(cursorHookEvent({ ...common, hook_event_name: "stop" })).toMatchObject({
      kind: "turn-end",
      eventId: "cursor-session:turn:generation-1",
      text: "",
    });
    expect(cursorHookEvent({ ...common, hook_event_name: "sessionEnd" })).toMatchObject({
      kind: "session-end",
      eventId: "cursor-session:session-end:generation-1",
    });
  });

  it("preserves pre-tool policy diagnostics until Cursor can expose them after the tool", async () => {
    const root = await temporaryDirectory();
    const storage = join(root, "sessions");
    const common = {
      conversation_id: "cursor-session",
      cwd: root,
      tool_use_id: "write-1",
      tool_name: "Write",
      tool_input: { file_path: "api/route.py", content: "cursor.execute('SELECT 1')" },
    };
    const diagnostic = "CURSOR_EXECUTE: Direct cursor access bypasses the typed API.";
    expect(await handleCursorEvent(
      { ...common, hook_event_name: "preToolUse" },
      scriptedLoop({ "tool-call": [delivery("steer", diagnostic)] }),
      storage,
    )).toEqual({ permission: "allow" });
    expect(await handleCursorEvent(
      { ...common, hook_event_name: "postToolUse", tool_output: "written" },
      scriptedLoop({}),
      storage,
    )).toEqual({ additional_context: diagnostic });
    expect(await handleCursorEvent(
      { ...common, tool_use_id: "read-2", tool_name: "Read", hook_event_name: "postToolUse" },
      scriptedLoop({}),
      storage,
    )).toEqual({});
  });

  it("durably carries follow-up deliveries to stop, then clears them when the session ends", async () => {
    const root = await temporaryDirectory();
    const common = { conversation_id: "cursor-session", generation_id: "generation-1", workspace_roots: [root] };
    const promptLoop = scriptedLoop({
      "user-prompt": [delivery("followUp", "Continue the plan."), delivery("observe", "not user-visible")],
    });

    expect(await handleCursorEvent(
      { ...common, hook_event_name: "beforeSubmitPrompt", prompt: "Start work" },
      promptLoop,
      join(root, "sessions"),
    )).toEqual({ continue: true });
    expect(promptLoop.events.map(event => event.kind)).toEqual(["user-prompt"]);
    expect(await handleCursorEvent(
      { ...common, hook_event_name: "stop" },
      scriptedLoop({}),
      join(root, "sessions"),
    )).toEqual({ followup_message: "Continue the plan." });
    expect(await handleCursorEvent(
      { ...common, hook_event_name: "stop" },
      scriptedLoop({}),
      join(root, "sessions"),
    )).toEqual({});

    await handleCursorEvent(
      { ...common, hook_event_name: "beforeSubmitPrompt", prompt: "Another turn" },
      scriptedLoop({ "user-prompt": [delivery("nextTurn", "Discard this.")] }),
      join(root, "sessions"),
    );
    expect(await handleCursorEvent(
      { ...common, hook_event_name: "sessionEnd" },
      scriptedLoop({}),
      join(root, "sessions"),
    )).toEqual({});
    expect(await handleCursorEvent(
      { ...common, hook_event_name: "stop" },
      scriptedLoop({}),
      join(root, "sessions"),
    )).toEqual({});
  });
});
