import { afterEach, describe, expect, test } from "vitest";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { DefaultResourceLoader, SessionManager, type ExtensionContext } from "@earendil-works/pi-coding-agent";
import { isRerailStarted } from "../src/repository-start.js";
import { setUpTestRepository } from "../test/support/repository-rules.js";
import { startTestRepository } from "../test/support/started-repository.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

describe("installed Pi extension", () => {
  test("loads through Pi's resource loader with native tools and lifecycle hooks", async () => {
    const cwd = await mkdtemp(join(tmpdir(), "rerail-pi-extension-"));
    temporaryDirectories.push(cwd);
    const loader = new DefaultResourceLoader({
      cwd,
      agentDir: join(cwd, ".pi", "agent"),
      additionalExtensionPaths: [resolve("dist/pi-extension.js")],
      noSkills: true,
      noPromptTemplates: true,
      noThemes: true,
      noContextFiles: true,
    });

    await loader.reload();
    const loaded = loader.getExtensions();
    expect(loaded.errors).toEqual([]);
    expect(loaded.extensions).toHaveLength(1);
    const extension = loaded.extensions[0];
    expect(extension).toBeDefined();
    expect(extension!.tools.has("conversation_rules")).toBe(true);
    expect(extension!.commands.has("cosine-rules")).toBe(true);
    expect(extension!.commands.has("rerail-start")).toBe(true);
    expect(extension!.commands.has("rerail-stop")).toBe(true);
    expect([...extension!.handlers.keys()]).toEqual(expect.arrayContaining([
      "session_start",
      "session_tree",
      "session_shutdown",
      "input",
      "message_update",
      "message_end",
      "tool_call",
      "turn_end",
    ]));
    expect(extension!.handlers.has("session_switch")).toBe(false);
    expect(extension!.handlers.has("session_branch")).toBe(false);
  });

  test("stops Rerail in a live repository through the installed slash command", async () => {
    const cwd = await mkdtemp(join(tmpdir(), "rerail-pi-stop-"));
    temporaryDirectories.push(cwd);
    startTestRepository(cwd);
    const loader = new DefaultResourceLoader({
      cwd,
      agentDir: join(cwd, ".pi", "agent"),
      additionalExtensionPaths: [resolve("dist/pi-extension.js")],
      noSkills: true,
      noPromptTemplates: true,
      noThemes: true,
      noContextFiles: true,
    });
    await loader.reload();
    const extension = loader.getExtensions().extensions[0]!;
    const context = {
      cwd,
      sessionManager: SessionManager.inMemory(cwd),
    } as unknown as ExtensionContext;
    await extension.handlers.get("session_start")![0]!({ type: "session_start", reason: "startup" }, context);
    const notices: string[] = [];
    const command = extension.commands.get("rerail-stop")!;
    await command.handler("", {
      ...context,
      hasUI: true,
      ui: {
        notify(message: string) { notices.push(message); },
        select: async () => undefined,
        input: async () => undefined,
        confirm: async () => false,
      },
    } as unknown as Parameters<typeof command.handler>[1]);
    expect(isRerailStarted(cwd)).toBe(false);
    expect(notices).toEqual([expect.stringContaining("○ Rerail is stopped in")]);
    await extension.handlers.get("input")![0]!({ type: "input", text: "This must not be recorded." }, context);
    await extension.handlers.get("session_shutdown")![0]!({ type: "session_shutdown", reason: "quit" }, context);
  }, 30_000); // `/rerail-stop` runs the real CLI, a Node process of its own.

  test("loads the repository's conversation rules at session start through the packaged extension's registered tool", async () => {
    const cwd = await mkdtemp(join(tmpdir(), "rerail-pi-rules-"));
    temporaryDirectories.push(cwd);
    setUpTestRepository(cwd, {
      library: [{
        kind: "conversation",
        rule: {
          id: "checkout",
          mode: "log",
          reference: "Guest checkout remains available.",
          match: { operator: "gte", threshold: 0.99 },
          sample: { source: "assistant-text", boundary: "message", minimumTokens: 1 },
          action: { content: "Check checkout requirements." },
        },
      }],
    });
    const loader = new DefaultResourceLoader({
      cwd,
      agentDir: join(cwd, ".pi", "agent"),
      additionalExtensionPaths: [resolve("dist/pi-extension.js")],
      noSkills: true,
      noPromptTemplates: true,
      noThemes: true,
      noContextFiles: true,
    });

    await loader.reload();
    const loaded = loader.getExtensions();
    expect(loaded.errors).toEqual([]);
    const extension = loaded.extensions[0]!;
    const sessionStart = extension.handlers.get("session_start")?.[0];
    const sessionShutdown = extension.handlers.get("session_shutdown")?.[0];
    expect(sessionStart).toBeDefined();
    expect(sessionShutdown).toBeDefined();
    const context = {
      cwd,
      sessionManager: SessionManager.inMemory(cwd),
    } as unknown as ExtensionContext;
    let started = false;
    try {
      started = true;
      await sessionStart!({ type: "session_start", reason: "startup" }, context);

      const result = await extension.tools.get("conversation_rules")!.definition.execute(
        "call-1",
        { action: "list" },
        undefined,
        undefined,
        context,
      );
      expect(result).toMatchObject({
        details: {
          rules: [expect.objectContaining({ rule: expect.objectContaining({ id: "checkout" }) })],
        },
      });
    } finally {
      if (started) await sessionShutdown!({ type: "session_shutdown", reason: "quit" }, context);
    }
  });
});
