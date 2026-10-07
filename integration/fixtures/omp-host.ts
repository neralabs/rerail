// Loaded only by the standalone OMP executable, never by Vitest/Node.
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import { AuthStorage } from "@oh-my-pi/pi-ai";
import { createMockModel } from "@oh-my-pi/pi-ai/providers/mock";
import type { MockResponse } from "@oh-my-pi/pi-ai/providers/mock";
import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

export default async function fixture(api: ExtensionAPI): Promise<void> {
  const config = JSON.parse(await readFile(process.env.RERAIL_OMP_TEST_CONFIG!, "utf8")) as {
    responses: MockResponse[];
    extension?: string;
    kernel?: boolean;
    modelPath: string;
    evidencePath: string;
    authPath?: string;
  };
  const model = createMockModel({ id: "rerail-script", provider: "rerail-test", responses: config.responses });
  api.registerProvider("rerail-test", {
    api: "rerail-test", baseUrl: "mock://", apiKey: "rerail-test-not-a-credential",
    streamSimple: model.stream,
    models: [{ id: model.id, name: model.name, reasoning: false, input: ["text"], cost: model.cost,
      contextWindow: model.contextWindow, maxTokens: model.maxTokens }],
  });
  const finished = Promise.withResolvers<void>();
  let notificationStarted = false;
  let notificationReleased = false;
  if (config.kernel) {
    // Deliberately blocked observer: tool execution and its policy barrier must not wait for it.
    api.on("tool_execution_start", async () => {
      notificationStarted = true;
      await finished.promise;
      notificationReleased = true;
    });
    api.on("tool_result", () => { finished.resolve(); });
  }
  if (config.extension) {
    // The test selects the installed default or configurable production entrypoint at runtime.
    const production = await import(pathToFileURL(config.extension).href);
    await (config.kernel
      ? production.createCosineRulesExtension({ modelPath: config.modelPath, registerCommand: false, registerTool: false })
      : production.default)(api);
  }
  api.registerCommand("rerail-test-append", {
    description: "Persist a test watermark in the real OMP branch",
    handler: async args => {
      const entry = JSON.parse(args) as { customType: string; data: unknown };
      api.appendEntry(entry.customType, entry.data);
    },
  });
  api.registerCommand("rerail-test-snapshot", {
    description: "Capture observable standalone session evidence",
    handler: async (_args, ctx) => {
      await ctx.waitForIdle();
      let auth: unknown;
      if (config.authPath) {
        const storage = await AuthStorage.create(config.authPath, {
          usageProviderResolver: () => undefined,
          rankingStrategyResolver: () => undefined,
        });
        try {
          // create() opens the store; the credential cache is only populated by reload().
          await storage.reload();
          const credential = storage.get("openai-codex");
          const key = await storage.getApiKey("openai-codex");
          auth = {
            type: credential?.type,
            accessHash: key ? createHash("sha256").update(key).digest("hex") : null,
            refresh: credential?.type === "oauth" ? credential.refresh : null,
            expires: credential?.type === "oauth" ? credential.expires : null,
            accountId: credential?.type === "oauth" ? credential.accountId : null,
          };
        } finally { storage.close(); }
      }
      await writeFile(config.evidencePath, JSON.stringify({
        sessionId: ctx.sessionManager.getSessionId(), branch: ctx.sessionManager.getBranch(),
        calls: model.calls, notificationStarted, notificationReleased, auth,
      }), { mode: 0o600 });
    },
  });
}
