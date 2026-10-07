import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { policyControlsResponseSchema } from "../src/api/operations.js";
import { RerailDaemon } from "../src/daemon.js";
import { resetRepositoryHistory } from "../src/database.js";
import { openScopedRuleCatalog, resolveRepositoryContext } from "../src/scoped-rule-catalog.js";
import { NativeExecutablePolicyEnforcer } from "../src/executable-policy-enforcer.js";
import { bootstrapPolicyDslCatalog } from "../src/policy-dsl-catalog.js";
import { RerailRuntime, type RerailRuntimeOptions } from "../src/runtime.js";
import { runScanCommand } from "../src/scan-cli.js";
import { SqliteScanControlStore } from "../src/scan-control.js";
import { scanRequestSchema } from "../src/scan-control-schema.js";
import { registerRerailCommands, type RerailCommandHost } from "../src/rerail-command.js";
import { isRerailStarted, workspaceStartState } from "../src/repository-start.js";

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
  const database = join(home, "rerail.sqlite");
  const rules = openScopedRuleCatalog(root, database);
  rules.upsertPolicy({
    format: "rerail.policy/v1", id: "test:no-print", revision: 1, mode: "block",
    scope: { include: ["api/**/*.py"], exclude: [] },
    clauses: [{
      id: "no-print",
      violation: { code: "PRINT", anchor: "witness.span", message: "Use logging" },
      forbid: { regex: { pattern: "\\bprint\\(", source: "file.content" } },
    }],
  });
  rules.close();
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
  it("starts, stops, and restarts this repository through slash commands", async () => {
    const { root } = await fixture();
    const name = basename(root);
    const commands = new Map<string, Parameters<RerailCommandHost["registerCommand"]>[1]>();
    let reattached = 0;
    const notices: string[] = [];
    registerRerailCommands({ registerCommand(name, command) { commands.set(name, command); } }, {
      packageRoot: resolve("."),
      cwd: () => root,
      reattach: async () => { reattached += 1; },
    });
    const ui = {
      notify(message: string) { notices.push(message); },
      select: async () => undefined,
      input: async () => undefined,
      confirm: async () => false,
    };

    // Stopping a repository that was never set up leaves it unset, so a later start still sets it up.
    await commands.get("rerail-stop")!.handler("", { ui });
    expect(notices.at(-1)).toBe(`○ Rerail has not been started in ${name}, so there is nothing to stop. Start it with /rerail-start.`);
    expect(workspaceStartState(root)).toBe("never");

    await commands.get("rerail-start")!.handler("", { ui });
    expect(notices.at(-1)).toMatch(new RegExp(`^● Rerail is on in ${name}\\. \\d+ rules are on\\.$`, "u"));
    expect(isRerailStarted(root)).toBe(true);

    await commands.get("rerail-stop")!.handler("", { ui });
    expect(isRerailStarted(root)).toBe(false);
    expect(notices.at(-1)).toBe([
      `○ Rerail is stopped in ${name}. No rules run here, and nothing is logged.`,
      "  Start it again with /rerail-start.",
      "  To turn off only one rule instead, say “turn that rule off”.",
    ].join("\n"));

    await commands.get("rerail-start")!.handler("", { ui });
    expect(notices.at(-1)).toMatch(/rules are on, as you left them\.$/u);
    expect(isRerailStarted(root)).toBe(true);
    // Every command rebuilds the runtime, which attaches only where Rerail is started.
    expect(reattached).toBe(4);
  }, 60_000);

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
      const settings = policyControlsResponseSchema.parse(await (await fetch(new URL("/api/sessions/s1/audit-settings", daemon.url))).json());
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
      const settings = policyControlsResponseSchema.parse(await (await fetch(new URL("/api/sessions/s1/audit-settings", daemon.url))).json());
      expect(settings.settings).toEqual({ scope: "touched-files", mode: "steer" });
      expect(await (await fetch(new URL("/api/sessions/s1/audit-requests", daemon.url))).json()).toEqual({ auditRequests: [] });
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
      expect(await (await fetch(new URL("/api/audits?sessionId=s1", daemon.url))).json()).toMatchObject({
        audits: expect.arrayContaining([expect.objectContaining({
          paths: ["api/clean file.py"], mode: "log", status: "completed", violations: [], failures: [],
        })]),
      });
      const settings = policyControlsResponseSchema.parse(await (await fetch(new URL("/api/sessions/s1/audit-settings", daemon.url))).json());
      expect(settings.settings.mode).toBe("steer");
    } finally { vi.unstubAllEnvs(); }
  }, 15_000);
  it("runs one log-only repository diagnostic for the workspace and exposes it to later sessions", async () => {
    const automaticScans: Array<{ sessionId: string; scope: string; mode: string }> = [];
    const { root, runtime, daemon } = await fixture(options => ({
      openWorkspace: async () => undefined,
      updateSettings: () => undefined,
      acceptsPath: () => true,
      evaluateGeneratedEdits: async () => ({ revision: 0, disposition: "continue", violations: [], failures: [] }),
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
        options.executions.finish(runId, { status: "completed", violations: [], failures: [] });
        return [runId];
      },
      interrupt: () => undefined,
      close: async () => undefined,
    }));
    await expect.poll(() => automaticScans).toEqual([{ sessionId: "s1", scope: "repository", mode: "log" }]);
    const firstControls = await (await fetch(new URL("/api/sessions/s1/audit-settings", daemon.url))).json();
    expect(firstControls).toMatchObject({
      repositoryAudit: { status: "completed" },
      repositoryAuditRecord: {
        sessionId: "s1", scope: "repository", mode: "log", violations: [], failures: [],
      },
    });

    await runtime.ingest({
      kind: "session-start",
      eventId: "start-2",
      session: { host: "omp", sessionId: "s2", cwd: root },
    });
    expect(automaticScans).toEqual([{ sessionId: "s1", scope: "repository", mode: "log" }]);
    expect(await (await fetch(new URL("/api/sessions/s2/audit-settings", daemon.url))).json()).toMatchObject({
      settings: { scope: "touched-files", mode: "steer" },
      repositoryAudit: { status: "completed" },
      repositoryAuditRecord: { sessionId: "s1", scope: "repository", mode: "log" },
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
    const response = await fetch(new URL("/api/audits?sessionId=s1", daemon.url));
    expect(await response.json()).toMatchObject({ audits: expect.arrayContaining([
      expect.objectContaining({ auditId: scan.auditIds[0], scope: "touched-files", mode: "log", status: "completed", violations: [], failures: [] }),
    ]) });
    const settings = policyControlsResponseSchema.parse(await (await fetch(new URL("/api/sessions/s1/audit-settings", daemon.url))).json());
    expect(settings.settings.mode).toBe("steer");
  });

  it("reports an empty Git diff without starting a policy run", async () => {
    const { root, home } = await fixture();
    let output = "";
    const code = await runScanCommand("scan", ["--session", "s1", "--scope", "git-diff", "--wait", "--json"], {
      home, stdout: { write: text => { output += text; return true; } },
    });
    expect(code).toBe(1);
    const scan = scanRequestSchema.parse(JSON.parse(output));
    expect(scan).toMatchObject({ status: "failed", auditIds: [], error: expect.stringMatching(/no changed files/i) });
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
      const databasePath = join(options.workspaceRoot, "home", "rerail.sqlite");
      const catalog = bootstrapPolicyDslCatalog({ databasePath, store: openScopedRuleCatalog(options.workspaceRoot, databasePath) });
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
        kind: "call", eventId: "write-start", sessionId: "s1",
        toolCallId: "write", toolName: "write", input: { path: "api/clean.py" },
      });
      const scan = controls.requestScan("s1", { scope: "repository" });
      await expect.poll(() => controls.listScans("s1").find(item => item.id === scan.id)?.status).toBe("running");
      controls.updateSettings("s1", { mode: "log" });
      await writeFile(join(root, "api/clean.py"), "print('new edit')\n");
      completion = runtime.ingest({
        kind: "result", eventId: "write-finished", sessionId: "s1",
        toolCallId: "write", toolName: "write", output: "written",
      });
      await expect.poll(async () => {
        const response = await fetch(new URL("/api/sessions/s1/events", daemon.url));
        return await response.json();
      }).toMatchObject({ events: expect.arrayContaining([expect.objectContaining({ kind: "result" })]) });
      gate.resolve();
      await completion;
      const delivered = await runtime.ingest({ kind: "turn-end", eventId: "turn", sessionId: "s1", text: "done" });
      expect(delivered).toEqual([]);
      expect(await (await fetch(new URL("/api/audits?sessionId=s1", daemon.url))).json()).toMatchObject({
        audits: expect.arrayContaining([expect.objectContaining({
          scope: "touched-files", mode: "log", violations: expect.arrayContaining([expect.objectContaining({ code: "PRINT" })]),
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
