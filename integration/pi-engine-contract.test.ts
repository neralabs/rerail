import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { InMemoryCredentialStore } from "@earendil-works/pi-ai";
import { CONTRACT_CURSOR_EXECUTE_MESSAGE, CONTRACT_CURSOR_EXECUTE_RULE_ID, seedContractCodeRuleForRoot } from "./support/contract-code-rule.js";
import { awaitCodeRulesOpenThrough } from "./support/code-rules-open.js";
import { steerText } from "./support/steer-text.js";
import { startTestRepository } from "../test/support/started-repository.js";
import { afterEach, describe, expect, it } from "vitest";
import {
  createAgentSession,
  DefaultResourceLoader,
  ModelRuntime,
  SessionManager,
} from "@earendil-works/pi-coding-agent";
import { createPiConversationRulesExtension } from "../src/hosts/pi/index.js";

const temporaryDirectories: string[] = [];
const originalRerailHome = process.env.RERAIL_HOME;

afterEach(async () => {
  if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
  else process.env.RERAIL_HOME = originalRerailHome;
  await Promise.all(temporaryDirectories.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

describe("Pi native code rule engine", () => {
  it("delivers the contract cursor-execute code rule violation once, in the write's result", async () => {
    const cwd = await mkdtemp(join(tmpdir(), "rerail-pi-kernel-"));
    process.env.RERAIL_HOME = join(cwd, "home");
    startTestRepository(cwd);
    seedContractCodeRuleForRoot(cwd);
    temporaryDirectories.push(cwd);
    await mkdir(join(cwd, "api"));
    await writeFile(join(cwd, "api", "route.py"), "def route():\n    return 1\n");

    const sessionManager = SessionManager.inMemory(cwd);
    const resourceLoader = new DefaultResourceLoader({
      cwd,
      agentDir: join(cwd, ".pi", "agent"),
      extensionFactories: [
        {
          name: "rerail-pi-kernel",
          factory: createPiConversationRulesExtension({
            modelPath: resolve("model"),
            registerCommand: false,
            registerTool: false,
          }),
        },
      ],
      noSkills: true,
      noPromptTemplates: true,
      noThemes: true,
      noContextFiles: true,
    });
    await resourceLoader.reload();
    expect(resourceLoader.getExtensions().errors).toEqual([]);

    const modelRuntime = await ModelRuntime.create({
      credentials: new InMemoryCredentialStore(),
      modelsPath: null,
      modelsStorePath: join(cwd, ".pi", "models"),
      allowModelNetwork: false,
      refreshOnCreate: false,
    });
    const { session } = await createAgentSession({
      cwd,
      agentDir: join(cwd, ".pi", "agent"),
      modelRuntime,
      noTools: "all",
      resourceLoader,
      sessionManager,
    });

    try {
      await session.bindExtensions({});
      // The write's own check runs only for a call that finds the evaluator open: one that finds it still opening once
      // its wait is over goes on without code rules, and only the baseline walk could find the edit, if it read the
      // file after the write. So the test waits for the evaluator, through calls that edit nothing.
      await awaitCodeRulesOpenThrough(sessionManager.getSessionId(), async probe => {
        const probeCallId = `open-probe:${probe}`;
        const input = { path: "api/route.py" };
        await session.extensionRunner.emitToolCall({ type: "tool_call", toolCallId: probeCallId, toolName: "read", input });
        await session.extensionRunner.emitToolResult({
          type: "tool_result", toolCallId: probeCallId, toolName: "read", input,
          content: [{ type: "text", text: "def route():\n    return 1\n" }], details: undefined, isError: false,
        });
      });
      const toolCallId = "write-route-with-cursor";
      await session.extensionRunner.emitToolCall({
        type: "tool_call",
        toolCallId,
        toolName: "write",
        input: {
          path: "api/route.py",
          content: "def route(cursor):\n    cursor.execute('SELECT 1')\n",
        },
      });
      await writeFile(join(cwd, "api", "route.py"), "def route(cursor):\n    cursor.execute('SELECT 1')\n");
      const result = await session.extensionRunner.emitToolResult({
        type: "tool_result",
        toolCallId,
        toolName: "write",
        input: {
          path: "api/route.py",
          content: "def route(cursor):\n    cursor.execute('SELECT 1')\n",
        },
        content: [{ type: "text", text: "Wrote api/route.py" }],
        details: undefined,
        isError: false,
      });
      const steer = steerText(CONTRACT_CURSOR_EXECUTE_RULE_ID, CONTRACT_CURSOR_EXECUTE_MESSAGE, "api/route.py:2:5");
      // The write's check finishes while Pi waits on the write's result, so its steer rides that result.
      expect(result?.content?.filter(block => block.type === "text" && block.text.includes(steer))).toHaveLength(1);
      // The end of the agent's run hands Pi every steer still due: none of them repeats the one the result carried.
      await session.extensionRunner.emitBoundary({ type: "agent_before_settle", outcome: "completed" }, () => ({
        contextEntries: [], contextMessages: [], llmMessages: [], pendingMessages: [], canContinue: false,
      }));
      expect(sessionManager.getBranch()).not.toContainEqual(
        expect.objectContaining({ type: "custom_message", content: expect.stringContaining(steer) }),
      );
    } finally {
      await session.dispose();
    }
  }, 30_000);
});
