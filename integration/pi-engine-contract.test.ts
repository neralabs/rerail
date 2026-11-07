import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { InMemoryCredentialStore } from "@earendil-works/pi-ai";
import { CONTRACT_CURSOR_EXECUTE_MESSAGE, CONTRACT_CURSOR_EXECUTE_RULE_ID, seedContractCodeRuleForRoot } from "./support/contract-code-rule.js";
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
  it("delivers the contract cursor-execute code rule violation through Pi's native session queue", async () => {
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
      await session.extensionRunner.emitToolResult({
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
      await expect.poll(() => sessionManager.getBranch(), { timeout: 10_000, interval: 10 }).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            type: "custom_message",
            content: expect.stringContaining(steer),
          }),
        ]),
      );
    } finally {
      await session.dispose();
    }
  }, 30_000);
});
