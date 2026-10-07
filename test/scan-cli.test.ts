import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { policyControlsResponseSchema } from "../src/api/operations.js";
import { RerailDaemon } from "../src/daemon.js";
import { resetRepositoryHistory } from "../src/database.js";
import { resolveRepositoryContext } from "../src/scoped-rule-catalog.js";
import { NativeExecutablePolicyEnforcer } from "../src/executable-policy-enforcer.js";
import { bootstrapPolicyDslCatalog } from "../src/policy-dsl-catalog.js";
import { RerailRuntime, type RerailRuntimeOptions } from "../src/runtime.js";
import { runScanCommand } from "../src/scan-cli.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { SqliteScanControlStore } from "../src/scan-control.js";
import { scanRequestSchema } from "../src/scan-control-schema.js";
import { registerRerailCommands, type RerailCommandHost } from "../src/rerail-command.js";

const fixtures: Array<{ root: string; runtime: RerailRuntime; daemon: RerailDaemon }> = [];
afterEach(async () => {
  for (const { root, runtime, daemon } of fixtures.splice(0)) {
    await runtime.dispose();
    await daemon.close();
    await rm(root, { recursive: true, force: true });
  }
});

async function fixture(createPolicyEvaluator?: RerailRuntimeOptions["createPolicyEvaluator"]) {
  const root = await mkdtemp(join(tmpdir(), "rerail-scan-cli-"));
  const home = join(root, "home");
  await mkdir(home);
  await mkdir(join(root, "api"));
  await writeFile(join(root, "api/clean.py"), "value = 42\n");
  await writeFile(join(root, ".gitignore"), ".rerail/\nhome/\nkernel.sock\n");
  execFileSync("git", ["init"], { cwd: root });
  execFileSync("git", ["add", ".gitignore", "api/clean.py"], { cwd: root });
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-m", "baseline"], { cwd: root });
  // A pre-catalog checkout database: the runtime imports its rules into the Rerail database.
  const rules = new SqliteRuleStore(join(root, ".rerail/transcripts.sqlite"));
  rules.upsertPolicy({
    format: "rerail.policy/v1", id: "test:no-print", revision: 1,
    scope: { include: ["api/**/*.py"], exclude: [] },
    checks: [{
      id: "no-print",
      finding: { code: "PRINT", severity: "error", anchor: "witness.span", message: "Use logging" },
      forbid: { regex: { pattern: "\\bprint\\(", source: "file.content" } },
    }],
  });
  rules.close();
  const database = join(home, "rerail.sqlite");
  const runtime = new RerailRuntime({
    storageRoot: join(root, ".rerail/sessions"), databasePath: database,
    workspaceRoot: root,
    modelPath: resolve("model"), autostart: false,
    ...(createPolicyEvaluator === undefined ? {} : { createPolicyEvaluator }),
  });
  await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "omp", sessionId: "s1", cwd: root } });
  const daemon = await RerailDaemon.start({
    version: "test", databasePath: database, workspaceRoot: root, port: 0,
    stateFile: join(home, "daemon.json"), kernelSocketPath: join(root, "kernel.sock"),
  });
  fixtures.push({ root, runtime, daemon });
  return { root, home, database, daemon, runtime };
}
function registeredCommands(root: string) {
  const commands = new Map<string, Parameters<RerailCommandHost["registerCommand"]>[1]>();
  registerRerailCommands({ registerCommand(name, command) { commands.set(name, command); } }, {
    packageRoot: resolve("."),
    cwd: () => root,
    sessionId: () => "s1",
  });
  return commands;
}

function selections(...values: Array<string | undefined>) {
  return async (_title: string, choices: string[]): Promise<string | undefined> => {
    const value = values.shift();
    if (value === undefined) return undefined;
    const choice = choices.find(item => item.startsWith(value));
    if (choice === undefined) throw new Error(`Choice ${value} was not offered`);
    return choice;
  };
}


describe("rerail scan and settings CLI", () => {
  it("disables and re-enables this checkout through slash commands", async () => {
    const { root } = await fixture();
    const commands = new Map<string, Parameters<RerailCommandHost["registerCommand"]>[1]>();
    let detached = 0;
    let reattached = 0;
    const notices: string[] = [];
    registerRerailCommands({ registerCommand(name, command) { commands.set(name, command); } }, {
      packageRoot: resolve("."),
      cwd: () => root,
      detach: async () => { detached += 1; },
      reattach: async () => { reattached += 1; },
    });
    const ui = {
      notify(message: string) { notices.push(message); },
      select: async () => undefined,
      input: async () => undefined,
      confirm: async () => false,
    };

    await commands.get("rerail-disable")!.handler("", { ui });
    expect(await readFile(join(root, ".rerail", "disabled"), "utf8")).toBe("");
    expect(detached).toBe(1);
    expect(notices.at(-1)).toContain(`Rerail disabled for ${root}`);

    await commands.get("rerail-enable")!.handler("", { ui });
    await expect(readFile(join(root, ".rerail", "disabled"))).rejects.toMatchObject({ code: "ENOENT" });
    expect(reattached).toBe(1);
    expect(notices.at(-1)).toBe(`Rerail enabled for ${root}.`);
  });

  it("edits persisted settings interactively from a compiled Bun host", async () => {
    const { root, home, daemon } = await fixture();
    const command = registeredCommands(root).get("rerail-settings");
    expect(command).toBeDefined();
    const originalExecutable = process.execPath;
    const originalBun = Object.getOwnPropertyDescriptor(process.versions, "bun");
    const errors: string[] = [];
    try {
      vi.stubEnv("RERAIL_HOME", home);
      Object.defineProperty(process.versions, "bun", { configurable: true, value: "compiled-host" });
      process.execPath = join(root, "not-a-javascript-interpreter");
      await command!.handler("", {
        ui: {
          notify(message, level) { if (level === "error") errors.push(message); },
          select: selections("Ongoing scope", "repository", "Ongoing mode", "log", "Done"),
          input: async () => undefined,
          confirm: async () => false,
        },
      });
      expect(errors).toEqual([]);
      const settings = policyControlsResponseSchema.parse(await (await fetch(new URL("/api/sessions/s1/policy-settings", daemon.url))).json());
      expect(settings.settings).toEqual({ scope: "repository", mode: "log" });
    } finally {
      process.execPath = originalExecutable;
      if (originalBun === undefined) Reflect.deleteProperty(process.versions, "bun");
      else Object.defineProperty(process.versions, "bun", originalBun);
      vi.unstubAllEnvs();
    }
  }, 15_000);

  it("cancels settings and scan dialogs without saving or submitting work", async () => {
    const { root, home, daemon } = await fixture();
    const commands = registeredCommands(root);
    const errors: string[] = [];
    const ui = {
      notify(message: string, level: string) { if (level === "error") errors.push(message); },
      select: selections("Ongoing mode", undefined),
      input: async () => undefined,
      confirm: async () => false,
    };
    try {
      vi.stubEnv("RERAIL_HOME", home);
      await commands.get("rerail-settings")!.handler("", { ui });
      ui.select = selections(undefined);
      await commands.get("rerail-scan")!.handler("", { ui });
      expect(errors).toEqual([]);
      const settings = policyControlsResponseSchema.parse(await (await fetch(new URL("/api/sessions/s1/policy-settings", daemon.url))).json());
      expect(settings.settings).toEqual({ scope: "touched-files", mode: "resteer" });
      expect(await (await fetch(new URL("/api/sessions/s1/scans", daemon.url))).json()).toEqual({ scans: [] });
    } finally { vi.unstubAllEnvs(); }
  }, 15_000);

  it("scans Git diff paths containing spaces without changing defaults", async () => {
    const { root, home, daemon } = await fixture();
    await writeFile(join(root, "api/clean file.py"), "value = 42\n");
    const errors: string[] = [];
    try {
      vi.stubEnv("RERAIL_HOME", home);
      await registeredCommands(root).get("rerail-scan")!.handler("", {
        ui: {
          notify(message, level) { if (level === "error") errors.push(message); },
          select: selections("git-diff"),
          input: async () => undefined,
          confirm: async () => false,
        },
      });
      expect(errors).toEqual([]);
      expect(await (await fetch(new URL("/api/policy-runs?sessionId=s1", daemon.url))).json()).toMatchObject({
        runs: expect.arrayContaining([expect.objectContaining({
          paths: ["api/clean file.py"], mode: "log", status: "completed", findings: [], failures: [],
        })]),
      });
      const settings = policyControlsResponseSchema.parse(await (await fetch(new URL("/api/sessions/s1/policy-settings", daemon.url))).json());
      expect(settings.settings.mode).toBe("resteer");
    } finally { vi.unstubAllEnvs(); }
  }, 15_000);
  it("runs one log-only repository diagnostic for the workspace and exposes it to later sessions", async () => {
    const automaticScans: Array<{ sessionId: string; scope: string; mode: string }> = [];
    const { root, runtime, daemon } = await fixture(options => ({
      openWorkspace: async () => undefined,
      updateSettings: () => undefined,
      acceptsPath: () => true,
      evaluateGeneratedEdits: async () => ({ revision: 0, disposition: "continue", findings: [], failures: [] }),
      scan: async (sessionId, input) => {
        automaticScans.push({ sessionId, scope: input.scope, mode: input.mode });
        const runId = options.executions.start({
          sessionId,
          workspaceRoot: options.workspaceRoot,
          scope: input.scope,
          mode: input.mode,
          revision: 0,
          catalogRevision: "test-catalog",
          ruleRevisions: [],
          paths: [],
        });
        options.executions.finish(runId, { status: "completed", findings: [], failures: [] });
        return [runId];
      },
      interrupt: () => undefined,
      close: async () => undefined,
    }));
    await expect.poll(() => automaticScans).toEqual([{ sessionId: "s1", scope: "repository", mode: "log" }]);
    const firstControls = await (await fetch(new URL("/api/sessions/s1/policy-settings", daemon.url))).json();
    expect(firstControls).toMatchObject({
      repositoryDiagnostic: { status: "completed" },
      repositoryDiagnosticRun: {
        sessionId: "s1", scope: "repository", mode: "log", findings: [], failures: [],
      },
    });

    await runtime.ingest({
      kind: "session-start",
      eventId: "start-2",
      session: { host: "omp", sessionId: "s2", cwd: root },
    });
    expect(automaticScans).toEqual([{ sessionId: "s1", scope: "repository", mode: "log" }]);
    expect(await (await fetch(new URL("/api/sessions/s2/policy-settings", daemon.url))).json()).toMatchObject({
      settings: { scope: "touched-files", mode: "resteer" },
      repositoryDiagnostic: { status: "completed" },
      repositoryDiagnosticRun: { sessionId: "s1", scope: "repository", mode: "log" },
    });
  });


  it("executes a real Git diff scan without changing ongoing mode", async () => {
    const { root, home, daemon } = await fixture();
    await writeFile(join(root, "api/clean.py"), "value = 43\n");
    let output = "";
    const code = await runScanCommand("scan", ["--session", "s1", "--scope", "git-diff", "--wait", "--json"], {
      home, stdout: { write: text => { output += text; return true; } },
    });
    expect(code).toBe(0);
    const scan = scanRequestSchema.parse(JSON.parse(output));
    expect(scan).toMatchObject({ status: "completed", mode: "log", error: null });
    const response = await fetch(new URL("/api/policy-runs?sessionId=s1", daemon.url));
    expect(await response.json()).toMatchObject({ runs: expect.arrayContaining([
      expect.objectContaining({ runId: scan.runIds[0], scope: "touched-files", mode: "log", status: "completed", findings: [], failures: [] }),
    ]) });
    const settings = policyControlsResponseSchema.parse(await (await fetch(new URL("/api/sessions/s1/policy-settings", daemon.url))).json());
    expect(settings.settings.mode).toBe("resteer");
  });

  it("reports an empty Git diff without starting a policy run", async () => {
    const { root, home } = await fixture();
    let output = "";
    const code = await runScanCommand("scan", ["--session", "s1", "--scope", "git-diff", "--wait", "--json"], {
      home, stdout: { write: text => { output += text; return true; } },
    });
    expect(code).toBe(1);
    const scan = scanRequestSchema.parse(JSON.parse(output));
    expect(scan).toMatchObject({ status: "failed", runIds: [], error: expect.stringMatching(/no changed files/i) });
  });

  it("interrupts scans and closes the transcript despite undiscovered source edits", async () => {
    const { root, database, runtime, daemon } = await fixture();
    const controls = new SqliteScanControlStore(database);
    await rm(join(root, "api/clean.py"));
    const scan = controls.requestScan("s1", { scope: "repository" });
    await runtime.closeSession("s1");
    expect(controls.listScans("s1").find(item => item.id === scan.id)).toMatchObject({ status: "interrupted" });
    expect(await (await fetch(new URL("/api/sessions", daemon.url))).json()).toMatchObject({
      sessions: [expect.objectContaining({ sessionId: "s1", endedAt: expect.any(Number) })],
    });
    expect(controls.getSettings("s1").active).toBe(false);
    controls.close();
  });
  it("keeps another live runtime able to scan after its repository history is reset", async () => {
    const { root, home, database } = await fixture();
    resetRepositoryHistory(database, resolveRepositoryContext(root).repositoryId);
    let output = "";
    expect(await runScanCommand("scan", ["--session", "s1", "--scope", "repository", "--wait", "--json"], {
      home, stdout: { write: text => { output += text; return true; } },
    })).toBe(0);
    expect(scanRequestSchema.parse(JSON.parse(output))).toMatchObject({ status: "completed" });
  });

  it("applies saved log mode to new edits while an explicit scan is still running", async () => {
    const gate = Promise.withResolvers<void>();
    const { root, database, runtime, daemon } = await fixture(options => {
      const catalog = bootstrapPolicyDslCatalog({ databasePath: join(options.workspaceRoot, ".rerail/transcripts.sqlite") });
      const evaluator = new NativeExecutablePolicyEnforcer(catalog, options);
      const close = evaluator.close.bind(evaluator);
      evaluator.close = async () => { await close(); catalog.close(); };
      const scan = evaluator.scan.bind(evaluator);
      let scanCount = 0;
      evaluator.scan = async (...args) => {
        scanCount += 1;
        if (scanCount > 1) await gate.promise;
        return scan(...args);
      };
      return evaluator;
    });
    const controls = new SqliteScanControlStore(database);
    let completion: Promise<readonly unknown[]> | undefined;
    try {
      await runtime.ingest({
        kind: "tool-call", eventId: "write-start", sessionId: "s1",
        toolCallId: "write", toolName: "write", input: { path: "api/clean.py" },
      });
      const scan = controls.requestScan("s1", { scope: "repository" });
      await expect.poll(() => controls.listScans("s1").find(item => item.id === scan.id)?.status).toBe("running");
      controls.updateSettings("s1", { mode: "log" });
      await writeFile(join(root, "api/clean.py"), "print('new edit')\n");
      completion = runtime.ingest({
        kind: "tool-result", eventId: "write-finished", sessionId: "s1",
        toolCallId: "write", toolName: "write", output: "written",
      });
      await expect.poll(async () => {
        const response = await fetch(new URL("/api/sessions/s1/events", daemon.url));
        return await response.json();
      }).toMatchObject({ events: expect.arrayContaining([expect.objectContaining({ kind: "tool-result" })]) });
      gate.resolve();
      await completion;
      const delivered = await runtime.ingest({ kind: "turn-end", eventId: "turn", sessionId: "s1", text: "done" });
      expect(delivered).toEqual([]);
      expect(await (await fetch(new URL("/api/policy-runs?sessionId=s1", daemon.url))).json()).toMatchObject({
        runs: expect.arrayContaining([expect.objectContaining({
          scope: "touched-files", mode: "log", findings: expect.arrayContaining([expect.objectContaining({ code: "PRINT" })]),
        })]),
      });
    } finally {
      gate.resolve();
      await completion;
      controls.close();
    }
  });


  it("requires a session instead of guessing the latest one", async () => {
    await expect(runScanCommand("scan", [], { home: "/unused" })).rejects.toThrow("--session is required");
  });
});
