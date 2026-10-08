import { readFile, unlink } from "node:fs/promises";
import { join, resolve } from "node:path";
import type { AssistantMessage, AssistantMessageEvent, Model } from "@oh-my-pi/pi-ai";
import type { ExtensionAPI, ExtensionContext, ExtensionFactory, ToolDefinition } from "@oh-my-pi/pi-coding-agent";
import type {
  PlaygroundChildControl,
  PlaygroundChildEvent,
  PlaygroundOutcome,
  PlaygroundRunInput,
} from "./protocol.js";
import type { AuditRecord } from "../../rerail/dist/audit.js";
import type { CodeRule } from "../../rerail/dist/code-rule-schema.js";
import type { ConversationRule } from "../../rerail/dist/rule-schema.js";
import { initializeWorkerChannel, workerControl } from "./ipc.js";
import { setTimeout as sleep } from "node:timers/promises";

function send(event: PlaygroundChildEvent): void {
  workerControl.send(event);
}

function clockMessage(type: "pause" | "resume" | "step"): string {
  if (type === "step") return "Advancing one scripted boundary";
  if (type === "pause") return "Script paused; terminal stays live";
  return "Script running";
}

function sameModel(a: Model, b: Model): boolean {
  return a.provider === b.provider && a.id === b.id && a.baseUrl === b.baseUrl && a.api === b.api;
}

type PlaygroundRuleDocument =
  | { kind: "conversation"; rule: ConversationRule }
  | { kind: "file"; rule: CodeRule };

interface RuleExecutionConfig {
  databasePath: string;
  conversationRule: ConversationRule | undefined;
  guidance: string;
}

function trace(kind: string, text: string): void {
  send({ type: "trace", entry: { at: Date.now(), kind, text: text.slice(0, 32_768) } });
}

function status(state: "running" | "paused" | "error" | "stopped", message: string): void {
  send({ type: "status", state, message: message.slice(0, 16_384) });
}

function ruleExecutionConfig(
  document: PlaygroundRuleDocument,
  rerailHome: string | undefined,
): RuleExecutionConfig {
  if (!rerailHome) throw new Error("Isolated Rerail home is missing");
  const databasePath = join(rerailHome, "rerail.sqlite");
  if (document.kind === "conversation")
    return { databasePath, conversationRule: document.rule, guidance: document.rule.action.content };
  return {
    databasePath,
    conversationRule: undefined,
    // A define clause holds shared definitions and reports nothing, so it has no violation to explain.
    guidance: document.rule.clauses
      .flatMap((clause) => (clause.violation === undefined ? [] : [`${clause.id}: ${clause.violation.message}`]))
      .join("\n\n"),
  };
}

export default async function playgroundHarness(api: ExtensionAPI): Promise<void> {
  const runInputPath = process.env.RERAIL_PLAYGROUND_INPUT;
  const daemonUrl = process.env.RERAIL_PLAYGROUND_DAEMON_URL;
  const cwd = process.cwd();
  await initializeWorkerChannel();

  function outcome(observed: PlaygroundOutcome["observed"], detail: string): void {
    send({ type: "outcome", outcome: { observed, detail: detail.slice(0, 32_768) } });
  }
  class CannotExercise extends Error {}

  /** One permit advances one scripted stream delta or awaited actual tool boundary. */
  class ScriptClock {
    paused = false;
    stopped = false;
    private permits = 0;
    private readonly waiters = new Set<() => void>();
    control(type: "pause" | "resume" | "step"): void {
      this.paused = type !== "resume";
      this.permits = type === "step" ? this.permits + 1 : 0;
      for (const wake of this.waiters) wake();
      status(this.paused ? "paused" : "running", clockMessage(type));
    }
    stop(): void {
      this.stopped = true;
      for (const wake of this.waiters) wake();
    }
    async boundary(signal?: AbortSignal): Promise<void> {
      let wake: (() => void) | undefined;
      const aborted = () => wake?.();
      signal?.addEventListener("abort", aborted, { once: true });
      try {
        if (!this.paused) await new Promise<void>((done) => setTimeout(done, 40));
        while (this.paused && this.permits === 0 && !this.stopped && !signal?.aborted) {
          await new Promise<void>((done) => {
            wake = done;
            this.waiters.add(done);
          });
          this.waiters.delete(wake!);
        }
        if (this.stopped || signal?.aborted) throw new Error("Script stopped");
        if (this.paused) this.permits--;
      } finally {
        if (wake) this.waiters.delete(wake);
        signal?.removeEventListener("abort", aborted);
      }
    }
  }

  const clock = new ScriptClock();
  const lifetimeAbort = new AbortController();
  let lifetime: NodeJS.Timeout | undefined;
  let runtimeContext: ExtensionContext | undefined;
  let activePrompt = false;
  let realAgent = false;
  let closing = false;
  let forceExit: NodeJS.Timeout | undefined;

  /** Stop through the root host's signal teardown; the fallback is only for a wedged host. */
  function shutdown(): void {
    if (closing) return;
    closing = true;
    clearTimeout(lifetime);
    clock.stop();
    lifetimeAbort.abort(new Error("Playground session stopped"));
    runtimeContext?.abort();
    forceExit = setTimeout(() => process.exit(0), 2_000);
    forceExit.unref();
    process.kill(process.pid, "SIGTERM");
  }

  /** The supervisor writes the replay input beside the workspace; the argument must stay inside that sandbox root. */
  async function loadRunInput(): Promise<PlaygroundRunInput> {
    const [{ runInputSchema }, { sandboxFilePath }] = await Promise.all([
      import("./protocol.js"),
      import("./replay-tools.js"),
    ]);
    if (!runInputPath) throw new CannotExercise("Replay input file is missing.");
    let inputPath: string;
    try {
      inputPath = await sandboxFilePath(resolve(cwd, ".."), runInputPath);
    } catch (error) {
      throw new CannotExercise(`Replay input file must be a sandbox file: ${String(error)}`);
    }
    const parsed = runInputSchema.safeParse(JSON.parse(await readFile(inputPath, "utf8")));
    if (!parsed.success) throw new CannotExercise(`Invalid replay scenario: ${parsed.error.message}`);
    const { selection, scenario, rule } = parsed.data;
    if (rule.id !== selection.ruleId || scenario.id !== selection.scenarioId || scenario.ruleKind !== rule.kind) {
      throw new CannotExercise("Scenario identity or rule kind is incompatible with the selected saved rule.");
    }
    return parsed.data;
  }

  async function loadSdk() {
    const [
      { createMockModel },
      { AssistantMessageEventStream },
      { createConversationRulesExtension, deliverOmpGuidance, bundledModelPath, MATCH_ENTRY_TYPE },
      { RULE_MESSAGE_TYPE, ADAPTER_CAPABILITIES },
      { ruleDocumentDigest },
      { parseConversationRule },
      { codeRuleSchema },
      { ThinkingLevel },
      { installReplayFixtures, replayTools, REPLAY_TOOL_NAMES },
    ] = await Promise.all([
      import("@oh-my-pi/pi-ai"),
      import("@oh-my-pi/pi-ai/utils/event-stream"),
      import("../../rerail/dist/extension.js"),
      import("../../rerail/dist/adapter-contract.js"),
      import("../../rerail/dist/stable-json.js"),
      import("../../rerail/dist/rule-schema.js"),
      import("../../rerail/dist/code-rule-schema.js"),
      import("@oh-my-pi/pi-agent-core"),
      import("./replay-tools.js"),
    ]);
    return {
      createMockModel,
      AssistantMessageEventStream,
      createConversationRulesExtension,
      deliverOmpGuidance,
      bundledModelPath,
      MATCH_ENTRY_TYPE,
      RULE_MESSAGE_TYPE,
      ADAPTER_CAPABILITIES,
      ruleDocumentDigest,
      parseConversationRule,
      codeRuleSchema,
      ThinkingLevel,
      installReplayFixtures,
      replayTools,
      REPLAY_TOOL_NAMES,
    };
  }

  interface AccessOnlyCredential {
    access: string;
    accountId: string;
    expires: number;
  }
  function parseAccessSnapshot(value: unknown): AccessOnlyCredential {
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Invalid snapshot");
    const fields = value as Record<string, unknown>;
    const { access, accountId, expires } = fields;
    if (
      Object.keys(fields).length !== 3 ||
      typeof access !== "string" ||
      !access ||
      typeof accountId !== "string" ||
      !accountId ||
      typeof expires !== "number" ||
      !Number.isFinite(expires) ||
      expires < Date.now() + 180_000
    )
      throw new Error("Invalid or short-lived snapshot");
    return { access, accountId, expires };
  }
  /** The private snapshot is consumed once; it never outlives the read regardless of validity. */
  async function loadAccessSnapshot(): Promise<AccessOnlyCredential> {
    const snapshotPath = resolve(cwd, ".playground-home", "real-agent-auth.json");
    try {
      return parseAccessSnapshot(JSON.parse(await readFile(snapshotPath, "utf8")));
    } catch {
      throw new CannotExercise(
        "Access-only OAuth snapshot is absent, invalid, or expires before the three-minute session lifetime.",
      );
    } finally {
      await unlink(snapshotPath).catch((error) => {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT")
          throw new Error("Could not remove private OAuth snapshot");
      });
    }
  }
  async function discoverRealModel(accessOnly: AccessOnlyCredential): Promise<Model> {
    const [{ fetchCodexModels }, { buildModel }] = await Promise.all([
      import("@oh-my-pi/pi-catalog/discovery/codex"),
      import("@oh-my-pi/pi-catalog/build"),
    ]);
    const discovery = await fetchCodexModels({
      accessToken: accessOnly.access,
      accountId: accessOnly.accountId,
      paths: ["/codex/models"],
      signal: AbortSignal.any([lifetimeAbort.signal, AbortSignal.timeout(15_000)]),
    });
    const advertisedModel = discovery?.models[0];
    if (!advertisedModel)
      throw new CannotExercise("Authenticated account discovery returned no usable model; generation was not started.");
    const model = buildModel(advertisedModel);
    trace("model-selected", `${model.provider}/${model.id}: first account-advertised model; no fallback.`);
    return model;
  }
  function providerModel(model: Model) {
    if (model.contextWindow === null || model.maxTokens === null)
      throw new CannotExercise(`Discovered model ${model.provider}/${model.id} has no bounded context metadata.`);
    return {
      id: model.id,
      name: model.name,
      api: model.api,
      reasoning: model.reasoning,
      input: [...model.input],
      cost: model.cost,
      contextWindow: model.contextWindow,
      maxTokens: model.maxTokens,
      ...(model.thinking === undefined ? {} : { thinking: model.thinking }),
      ...(model.headers === undefined ? {} : { headers: model.headers }),
      ...(model.compatConfig === undefined ? {} : { compat: model.compatConfig }),
      ...(model.premiumMultiplier === undefined ? {} : { premiumMultiplier: model.premiumMultiplier }),
    };
  }

  type Observation = [PlaygroundOutcome["observed"], string];
  function auditObservation(audits: AuditRecord[], ruleId: string): Observation {
    const failures = audits.flatMap((audit) => audit.failures);
    const violations = audits.flatMap((audit) => audit.violations).filter((violation) => violation.ruleId === ruleId);
    trace(
      "policy-evaluation",
      JSON.stringify({
        runs: audits.map((audit) => ({ status: audit.status, scope: audit.scope, revision: audit.revision })),
        findings: violations,
        failures,
      }),
    );
    if (failures.length) {
      const unexercised = failures.every(
        (failure) =>
          failure.code === "missing-capability" || failure.code === "source-incomplete" || failure.code === "source-partial",
      );
      return [unexercised ? "cannot-exercise" : "error", failures.map((f) => `${f.code}: ${f.message}`).join("\n")];
    }
    if (audits.some((audit) => audit.status === "running" || audit.status === "failed" || audit.status === "interrupted"))
      return ["error", "Native policy execution did not finish successfully within the bounded result wait."];
    if (!audits.some((audit) => audit.status === "completed"))
      return ["cannot-exercise", "No native policy evaluation completed for this scenario."];
    if (violations.length)
      return [
        "match",
        `${violations.length} finding(s) from the unchanged selected policy, evaluated by the production native host runtime.`,
      ];
    return ["no-match", "Production native policy evaluation completed with no findings for the selected rule."];
  }

  try {
    const { selection, scenario, rule } = await loadRunInput();
    realAgent = selection.mode === "agent";
    if (realAgent) {
      lifetime = setTimeout(() => {
        outcome("error", "Real agent exceeded its 180-second lifetime; pending requests and tools were aborted.");
        shutdown();
      }, 180_000);
      lifetime.unref();
    }

    const sdk = await loadSdk();
    const {
      createMockModel,
      AssistantMessageEventStream,
      createConversationRulesExtension,
      deliverOmpGuidance,
      bundledModelPath,
      MATCH_ENTRY_TYPE,
      RULE_MESSAGE_TYPE,
      ADAPTER_CAPABILITIES,
      ThinkingLevel,
    } = sdk;
    const definition: unknown = JSON.parse(rule.definition);
    const document =
      rule.kind === "conversation"
        ? { kind: "conversation" as const, rule: sdk.parseConversationRule(definition) }
        : { kind: "file" as const, rule: sdk.codeRuleSchema.parse(definition) };
    if (document.rule.id !== rule.id || sdk.ruleDocumentDigest(document) !== rule.digest)
      throw new CannotExercise("Pinned rule definition does not match its id and digest.");
    try {
      await sdk.installReplayFixtures(cwd, scenario);
    } catch (error) {
      throw new CannotExercise(`Invalid fixtures: ${String(error)}`);
    }
    const toolNames: string[] = [...sdk.REPLAY_TOOL_NAMES, ...(selection.allowCommands ? ["bash"] : [])];
    let toolExtension: ExtensionFactory;
    try {
      toolExtension = await sdk.replayTools(cwd, selection.allowCommands, scenario);
    } catch (error) {
      throw new CannotExercise(`Incompatible scenario tools: ${String(error)}`);
    }
    const { databasePath, conversationRule, guidance } = ruleExecutionConfig(
      document,
      process.env.RERAIL_HOME,
    );
    let matched = false;
    let evaluatedBoundary = false;
    let previewDelivered = false;
    let executionError: string | undefined;
    let toolError: string | undefined;
    let requestCount = 0;
    let scriptedIndex = 0;
    let reported = false;
    const ruleMode = document.rule.mode;

    const preview = (): void => {
      if (selection.mode !== "preview" || previewDelivered) return;
      const context = runtimeContext;
      if (!context) throw new Error("Preview delivery attempted before the root session started");
      previewDelivered = true;
      if (ruleMode === "log") {
        trace("preview", `Evaluator bypassed explicitly; a log ${rule.kind} rule is recorded only, so no guidance is sent to the model.`);
        return;
      }
      trace("preview", `Evaluator bypassed explicitly; saved ${rule.kind} content uses production ${ruleMode} delivery.`);
      void deliverOmpGuidance(api, {
        sessionId: context.sessionManager.getSessionId(),
        requested: ruleMode,
        mechanism: ADAPTER_CAPABILITIES.omp.delivery[ruleMode],
        content: guidance,
        ruleIds: [],
      });
      trace("delivered", `Production OMP queued ${ruleMode} guidance into the root session.`);
    };
    const boundary = (source: string, edge: string): void => {
      if (conversationRule?.sample.source === source && conversationRule.sample.boundary === edge) {
        evaluatedBoundary = true;
        preview();
      }
    };
    const conversationObservation = (): Observation => {
      if (matched) return ["match", "The unchanged saved cosine rule matched through the native production evaluator."];
      if (evaluatedBoundary) return ["no-match", "The saved sample boundary was exercised without a native match."];
      return ["cannot-exercise", "The run did not reach the saved rule's sample source and boundary."];
    };
    const settledAudits = async (): Promise<AuditRecord[]> => {
      const context = runtimeContext;
      if (!context) throw new Error("Root session context is unavailable");
      if (!daemonUrl) throw new Error("Isolated playground daemon URL is missing");
      const url = new URL("/api/audits", daemonUrl);
      url.searchParams.set("sessionId", context.sessionManager.getSessionId());
      url.searchParams.set("limit", "100");
      const list = async () => {
        const response = await fetch(url, { signal: AbortSignal.timeout(5_000) });
        if (!response.ok) throw new Error(`Policy history unavailable (${response.status})`);
        return (await response.json() as { audits: AuditRecord[] }).audits;
      };
      const deadline = Date.now() + 15_000;
      let audits = await list();
      while (audits.some((audit) => audit.status === "running") && Date.now() < deadline && !closing) {
        await sleep(50);
        audits = await list();
      }
      return audits;
    };
    const report = async (): Promise<void> => {
      if (reported || closing) return;
      reported = true;
      if (executionError || toolError) {
        outcome("error", executionError ?? toolError!);
        return;
      }
      if (selection.mode === "preview") {
        const previewed = ruleMode === "log"
          ? "Saved log rule previewed: recorded only, no guidance sent. Evaluator was bypassed; no match is claimed."
          : `Saved rule content previewed using ${ruleMode}. Evaluator was bypassed; no match is claimed.`;
        outcome(
          previewDelivered ? "preview" : "cannot-exercise",
          previewDelivered ? previewed : "The scenario did not reach the saved rule's sample boundary.",
        );
        return;
      }
      if (rule.kind === "conversation") {
        outcome(...conversationObservation());
        return;
      }
      outcome(...auditObservation(await settledAudits(), rule.id));
    };

    const systemPrompt = realAgent
      ? "Complete the user's scenario task using the provided sandbox fixtures. Available tools operate only in this isolated workspace. Be concise and truthful about actual results. Never claim an action you did not perform."
      : "You are the local scripted Rerail scenario provider. Tool calls execute genuinely in the sandbox. No remote model requests are permitted.";

    const instrumentation: ExtensionFactory = (rootApi) => {
      rootApi.on("before_agent_start", () => {
        boundary("user-input", "message");
        if (!conversationRule) preview();
        return { systemPrompt: [systemPrompt] };
      });
      rootApi.on("message_update", (event) => {
        if (
          event.message.role === "assistant" &&
          event.assistantMessageEvent.type === "text_delta" &&
          event.assistantMessageEvent.delta.trim()
        )
          boundary("assistant-text", "sentence");
      });
      rootApi.on("message_end", (event) => {
        if (event.message.role !== "assistant") return;
        if (event.message.stopReason === "error" || event.message.stopReason === "aborted")
          executionError = event.message.errorMessage ?? `Provider stopped: ${event.message.stopReason}`;
        else if (event.message.content.some((block) => block.type === "text" && block.text.trim())) {
          boundary("assistant-text", "message");
          boundary("assistant-text", "sentence");
        }
      });
      rootApi.on("turn_end", (event) => {
        if (
          event.message.role === "assistant" &&
          event.message.content.some((block) => block.type === "text" && block.text.trim())
        )
          boundary("assistant-text", "turn");
      });
      rootApi.on("tool_call", (event) => {
        trace("tool-call", `${event.toolName}: ${JSON.stringify(event.input)}`);
        boundary(`tool:${event.toolName}`, "tool-call");
        if (!toolNames.includes(event.toolName)) {
          toolError = `Unavailable tool ${event.toolName}`;
          return { block: true, reason: toolError };
        }
        if (closing) return { block: true, reason: "Playground session stopped" };
      });
      rootApi.on("tool_result", (event) => {
        trace("tool-result", `${event.toolName}: ${JSON.stringify(event.content)}`);
        if (event.isError) toolError = `Tool ${event.toolName} failed: ${JSON.stringify(event.content)}`;
        boundary(`tool:${event.toolName}`, "tool-call");
      });
      rootApi.on("agent_start", () => {
        activePrompt = true;
      });
      rootApi.on("agent_end", async (event) => {
        if (event.willContinue) return;
        activePrompt = false;
        trace(
          "idle",
          "Task completed. Explicit terminal input starts another turn; Step starts a scripted continuation.",
        );
        try {
          await report();
        } catch (error) {
          executionError = `Outcome reporting failed: ${String(error)}`;
          outcome("error", executionError);
        }
        if (!realAgent && !closing) clock.control("pause");
      });
    };
    const detector: ExtensionFactory = (rootApi) => {
      const tracedApi = new Proxy(rootApi, {
        get(target, key, receiver) {
          if (key === "appendEntry")
            return (customType: string, data: unknown) => {
              target.appendEntry(customType, data);
              if (customType === MATCH_ENTRY_TYPE) {
                matched = true;
                trace("detected", `Production native cosine match: ${JSON.stringify(data)}`);
              }
            };
          if (key === "on")
            return (name: Parameters<ExtensionAPI["on"]>[0], handler: (...args: unknown[]) => unknown) => {
              const wrapped = async (...args: unknown[]) => {
                try {
                  return await handler(...args);
                } catch (error) {
                  executionError = `Production evaluator failed: ${String(error)}`;
                  trace("evaluator-error", executionError);
                  throw error;
                }
              };
              Reflect.apply(target.on, target, [name, wrapped]);
            };
          if (key === "sendMessage")
            return (...args: Parameters<ExtensionAPI["sendMessage"]>) => {
              const [message, options] = args;
              target.sendMessage(...args);
              if (typeof message !== "string" && message.customType === RULE_MESSAGE_TYPE)
                trace(
                  "delivered",
                  `Production OMP accepted ${options?.deliverAs ?? "steer"} guidance into the root session.`,
                );
            };
          return Reflect.get(target, key, receiver);
        },
      });
      return createConversationRulesExtension({
        modelPath: bundledModelPath(),
        databasePath: databasePath,
        ruleFiles: [],
        registerCommand: false,
        registerTool: false,
      })(tracedApi);
    };

    let accessOnly: AccessOnlyCredential | undefined;
    let model: Model;
    if (realAgent) {
      accessOnly = await loadAccessSnapshot();
      model = await discoverRealModel(accessOnly);
      api.registerProvider(model.provider, {
        baseUrl: model.baseUrl,
        api: model.api,
        apiKey: accessOnly.access,
        models: [providerModel(model)],
      });
    } else {
      model = createMockModel({ id: "rerail-local-script", provider: "mock" });
      api.registerProvider(model.provider, {
        baseUrl: model.baseUrl,
        api: model.api,
        apiKey: "local-script-only",
        models: [providerModel(model)],
        streamSimple: (selectedModel, context, options) => {
          const output = new AssistantMessageEventStream();
          const run = async () => {
            if (closing || lifetimeAbort.signal.aborted) throw new Error("Playground session stopped");
            if (!sameModel(selectedModel, model)) throw new Error("Playground model switching is disabled.");
            const call = ++requestCount;
            const texts: string[] = [];
            let tools: (typeof scenario.steps)[number]["toolCalls"] = [];
            while (scriptedIndex < scenario.steps.length) {
              const step = scenario.steps[scriptedIndex++]!;
              texts.push(step.text);
              tools = step.toolCalls;
              if (tools.length) break;
            }
            if (!texts.some((text) => text.trim()) && tools.length === 0) {
              texts.push("The scripted inputs are complete.");
              trace(
                "script-complete",
                "Emitting the neutral scripted completion response; no extra tool action is scripted.",
              );
            }
            trace(
              "model-context",
              `Scripted provider call ${call}; saved guidance ${context.messages.some((message) => message.role !== "assistant" && JSON.stringify(message).includes(guidance)) ? "present" : "absent"} in actual outgoing context.`,
            );
            const signal = options?.signal
              ? AbortSignal.any([options.signal, lifetimeAbort.signal])
              : lifetimeAbort.signal;
            const message: AssistantMessage = {
              role: "assistant",
              api: selectedModel.api,
              provider: selectedModel.provider,
              model: selectedModel.id,
              content: [],
              stopReason: "stop",
              timestamp: Date.now(),
              usage: {
                input: 0,
                output: 0,
                cacheRead: 0,
                cacheWrite: 0,
                totalTokens: 0,
                cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
              },
            };
            output.push({ type: "start", partial: message });
            const block = { type: "text" as const, text: "" };
            message.content.push(block);
            output.push({ type: "text_start", contentIndex: 0, partial: message });
            for (const delta of texts.join("\n").match(/.{1,40}/gs) ?? []) {
              await output.trackLocalWork(clock.boundary(signal));
              block.text += delta;
              output.push({ type: "text_delta", contentIndex: 0, delta, partial: message });
            }
            output.push({ type: "text_end", contentIndex: 0, content: block.text, partial: message });
            for (const [index, tool] of tools.entries()) {
              await output.trackLocalWork(clock.boundary(signal));
              const toolCall = {
                type: "toolCall" as const,
                id: `playground-${call}-${index}`,
                name: tool.name,
                arguments: tool.arguments,
              };
              const contentIndex = message.content.length;
              message.content.push(toolCall);
              const toolEvents: AssistantMessageEvent[] = [
                { type: "toolcall_start", contentIndex, partial: message },
                { type: "toolcall_delta", contentIndex, delta: JSON.stringify(tool.arguments), partial: message },
                { type: "toolcall_end", contentIndex, toolCall, partial: message },
              ];
              for (const event of toolEvents) output.push(event);
              message.stopReason = "toolUse";
            }
            output.push({ type: "done", reason: message.stopReason === "toolUse" ? "toolUse" : "stop", message });
          };
          void run().catch((error) => output.fail(error));
          return output;
        },
      });
    }

    const deniedBash = "Direct terminal shell is disabled; use the sandbox bash tool with Allow commands.";
    api.on("user_bash", () => {
      const bytes = Buffer.byteLength(deniedBash);
      return {
        result: {
          output: deniedBash,
          exitCode: 1,
          cancelled: false,
          truncated: false,
          totalLines: 1,
          totalBytes: bytes,
          outputLines: 1,
          outputBytes: bytes,
        },
      };
    });
    const deniedPython = "Direct Python execution is disabled in the rule playground.";
    api.on("user_python", () => {
      const bytes = Buffer.byteLength(deniedPython);
      return {
        result: {
          output: deniedPython,
          exitCode: 1,
          cancelled: false,
          truncated: false,
          totalLines: 1,
          totalBytes: bytes,
          outputLines: 1,
          outputBytes: bytes,
          displayOutputs: [],
          stdinRequested: false,
        },
      };
    });

    const pacedToolApi = new Proxy(api, {
      get(target, key, receiver) {
        if (key === "registerTool")
          return (tool: ToolDefinition) => {
            target.registerTool({
              ...tool,
              async execute(toolCallId, params, signal, onUpdate, context) {
                if (!realAgent) await clock.boundary(signal);
                return tool.execute(toolCallId, params, signal, onUpdate, context);
              },
            });
          };
        return Reflect.get(target, key, receiver);
      },
    });
    await toolExtension(pacedToolApi);
    await instrumentation(api);
    if (selection.mode !== "preview") await detector(api);
    api.on("before_provider_request", (_event, context) => {
      if (!context.model || !sameModel(context.model, model))
        throw new Error("Playground model switching and fallback are disabled.");
      if (accessOnly && accessOnly.expires <= Date.now())
        throw new Error("Access-only credential expired; automatic refresh is disabled.");
      if (realAgent)
        trace(
          "model-context",
          `Real model request ${++requestCount}: ${context.model.provider}/${context.model.id}; actual provider text and tool decisions, unchanged saved rule evaluation.`,
        );
    });

    api.on("session_start", async (_event, context) => {
      runtimeContext = context;
      const registeredModel = context.modelRegistry.find(model.provider, model.id);
      if (!registeredModel || !sameModel(registeredModel, model))
        throw new Error("Registered playground model is unavailable from the root model catalog.");
      if (!(await api.setModel(registeredModel)))
        throw new Error("Registered playground model has no access-only credential.");
      if (!context.model || !sameModel(context.model, registeredModel))
        throw new Error("Root session did not activate the registered playground model.");
      await api.setActiveTools(toolNames);
      const activeTools = api.getActiveTools();
      if (
        activeTools.some((name) => !toolNames.includes(name)) ||
        toolNames.some((name) => !activeTools.includes(name))
      )
        throw new Error("Root session did not apply the exact playground tool allowlist.");
      await api.setSessionName("Rerail rule playground");
      if (realAgent) api.setThinkingLevel(ThinkingLevel.Low);
      trace(
        "isolation",
        "Standalone OMP owns the interactive session; selected saved rule only, fresh repeat state, sandbox fixtures/tools and OS writes. No ambient extensions, skills, rules, LSP, title model, or unsandboxed commands.",
      );
      status("running", `${selection.mode}: ${rule.id} / ${scenario.title}`);
    });
    api.on("session_shutdown", () => {
      clearTimeout(lifetime);
      clearTimeout(forceExit);
      clock.stop();
      try {
        status("stopped", "OMP sandbox session closed");
      } catch {
        // The supervisor may have closed its capability socket while root teardown was finishing.
      }
    });

    workerControl.on("message", (message: PlaygroundChildControl) => {
      if (message.type === "shutdown") {
        shutdown();
        return;
      }
      if (message.type === "command-result") return;
      if (!["pause", "resume", "step"].includes(message.type)) return;
      if (realAgent) {
        trace("control", "Remote generation cannot be paused; Stop cancels it.");
        return;
      }
      clock.control(message.type);
      if (message.type === "step" && !activePrompt && !clock.stopped)
        api.sendUserMessage("Continue this scenario with the pending Rerail guidance in your current context.");
    });
    workerControl.on("disconnect", shutdown);
  } catch (error) {
    clock.stop();
    lifetimeAbort.abort(error);
    setImmediate(() => process.kill(process.pid, "SIGTERM"));
    try {
      const observed = error instanceof CannotExercise ? "cannot-exercise" : "error";
      outcome(observed, String(error));
      trace("error", String(error));
      status("error", String(error));
    } catch {
      // Failing closed does not depend on a still-connected reporting channel.
    }
    throw error;
  }
}
