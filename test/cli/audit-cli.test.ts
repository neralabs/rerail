import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { auditControlsResponseSchema } from "../../src/api/index.js";
import { RerailDaemon } from "../../src/daemon/index.js";
import {
  resetRepositoryHistory,
  NativeCodeRuleEnforcer,
  codeRuleSchema,
  SqliteAuditControlStore,
  auditRequestSchema,
} from "../../src/engine/index.js";
import {
  RerailRuntime,
  type RerailRuntimeOptions,
  isRerailStarted,
  workspaceStartState,
} from "../../src/runtime/index.js";
import { runAuditCommand } from "../../src/cli/audit-cli.js";
import { registerHostCommands, type HostCommandHost } from "../../src/harness/index.js";
import { setUpTestRepository, testCodeRuleCatalog } from "../support/repository-rules.js";
import { startRerail } from "../../src/cli/index.js";
import { fakeSetupEnvironment } from "../support/fake-setup-environment.js";
import { SETUP_CANCELLED } from "../../src/harness/index.js";

const NO_PRINT = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:no-print", revision: 1, mode: "block",
  scope: { include: ["api/**/*.py"], exclude: [] },
  clauses: [{
    id: "no-print",
    violation: { code: "PRINT", anchor: "witness.span", message: "Use logging" },
    forbid: { regex: { pattern: "\\bprint\\(", source: "file.content" } },
  }],
});

const fixtures: Array<{ root: string; runtime: RerailRuntime; daemon: RerailDaemon }> = [];
afterEach(async () => {
  for (const { root, runtime, daemon } of fixtures.splice(0)) {
    await runtime.dispose();
    await daemon.close();
    await rm(root, { recursive: true, force: true });
  }
});

async function fixture(createAuditor?: RerailRuntimeOptions["createAuditor"]) {
  const root = await mkdtemp(join(tmpdir(), "rerail-scan-cli-"));
  const home = join(root, "home");
  await mkdir(home);
  await mkdir(join(root, "api"));
  await writeFile(join(root, "api/clean.py"), "value = 42\n");
  await writeFile(join(root, ".gitignore"), ".rerail/\nhome/\nengine.sock\n");
  execFileSync("git", ["init"], { cwd: root });
  execFileSync("git", ["add", ".gitignore", "api/clean.py"], { cwd: root });
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-m", "baseline"], { cwd: root });
  const database = join(home, "rerail.sqlite");
  const repositoryKey = setUpTestRepository(root, { library: [{ kind: "code", rule: NO_PRINT }] }, database);
  const runtime = new RerailRuntime({
    storageRoot: join(root, ".rerail/sessions"), databasePath: database,
    workspaceRoot: root,
    modelPath: resolve("model"), autostart: false,
    ...(createAuditor === undefined ? {} : { createAuditor }),
  });
  await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "omp", sessionId: "s1", cwd: root } });
  const daemon = await RerailDaemon.start({
    version: "test", databasePath: database, workspaceRoot: root, port: 0,
    stateFile: join(home, "daemon.json"), kernelSocketPath: join(root, "engine.sock"),
  });
  fixtures.push({ root, runtime, daemon });
  return { root, home, database, daemon, runtime, repositoryKey };
}

describe("rerail scan and settings CLI", () => {
  it("starts, stops, and restarts this repository through slash commands", async () => {
    const { root } = await fixture();
    const name = basename(root);
    const commands = new Map<string, Parameters<HostCommandHost["registerCommand"]>[1]>();
    let reattached = 0;
    const notices: string[] = [];
    registerHostCommands({ registerCommand(name, command) { commands.set(name, command); }, sendUserMessage() {} }, {
      packageRoot: resolve("."),
      cwd: () => root,
      reattach: async () => { reattached += 1; },
    });
    const asked: string[] = [];
    const ui = {
      notify(message: string) { notices.push(message); },
      select: async (title: string) => { asked.push(title); return undefined; },
      input: async () => undefined,
      confirm: async () => false,
    };

    // Stopping a repository Rerail never recorded changes nothing, so a later start still sets it up.
    await commands.get("rerail-stop")!.handler("", { ui });
    expect(notices.at(-1)).toBe(`○ Rerail has not been started in ${name}, so there is nothing to stop. Start it with /rerail-start.`);
    expect(workspaceStartState(root)).toBe("stopped");

    // Where the repository is not set up, start runs the set-up, which asks before reading; closing the question changes nothing.
    await commands.get("rerail-start")!.handler("", { ui });
    expect(asked).toEqual([`You’re working in ${name}. Can Rerail read it to pick the rule packs that look right?`]);
    expect(notices.at(-1)).toBe(SETUP_CANCELLED);
    expect(workspaceStartState(root)).toBe("stopped");
    // The person's "start" answer, run as the coding agent relays it.
    await startRerail(root, { lane: "agent", say: () => {} }, { answer: "start", environment: fakeSetupEnvironment() });
    expect(isRerailStarted(root)).toBe(true);

    await commands.get("rerail-stop")!.handler("", { ui });
    expect(isRerailStarted(root)).toBe(false);
    expect(notices.at(-1)).toBe([
      `○ Rerail is stopped in ${name}. No rules run here, and nothing is logged.`,
      "  Start it again with /rerail-start.",
      "  To turn off only one rule instead, say “turn that rule off”.",
    ].join("\n"));

    await commands.get("rerail-start")!.handler("", { ui });
    // Restarting a set-up repository ends on its scoped rules link, as the terminal's `rerail start` does.
    expect(notices.at(-1)).toMatch(/rules are on, as you left them\.\n {2}http:\/\/\S+\?repository=[^#\s]+&checkout=[^#\s]+#rules$/u);
    expect(isRerailStarted(root)).toBe(true);
    // Every command that changed something rebuilds the runtime; the cancelled set-up changed nothing.
    expect(reattached).toBe(3);
  }, 60_000);

  it("runs one log-only repository audit for the workspace and exposes it to later sessions", async () => {
    const automaticAuditRequests: Array<{ sessionId: string; scope: string; mode: string }> = [];
    const { root, runtime, daemon } = await fixture(options => ({
      openWorkspace: async () => undefined,
      updateSettings: () => undefined,
      acceptsPath: () => true,
      auditGeneratedEdits: async () => ({ revision: 0, disposition: "continue", violations: [], failures: [] }),
      // The diagnostic is claimed and finished per checkout through the audit hooks, as the router does.
      audit: async (sessionId, input) => {
        if (input.audit !== undefined && !input.audit.claim(options.workspaceRoot)) return [];
        automaticAuditRequests.push({ sessionId, scope: input.scope, mode: input.mode });
        const auditId = options.audits.start({
          sessionId,
          workspaceRoot: options.workspaceRoot,
          scope: input.scope,
          mode: input.mode,
          revision: 0,
          catalogRevision: "test-catalog",
          ruleRevisions: [],
          paths: [],
        });
        options.audits.finish(auditId, { status: "completed", violations: [], failures: [] });
        input.audit?.finish(options.workspaceRoot, auditId);
        return [auditId];
      },
      interrupt: () => undefined,
      close: async () => undefined,
    }));
    await expect.poll(() => automaticAuditRequests).toEqual([{ sessionId: "s1", scope: "repository", mode: "log" }]);
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
    expect(automaticAuditRequests).toEqual([{ sessionId: "s1", scope: "repository", mode: "log" }]);
    expect(await (await fetch(new URL("/api/sessions/s2/audit-settings", daemon.url))).json()).toMatchObject({
      settings: { scope: "touched-files", mode: "steer" },
      repositoryAudit: { status: "completed" },
      repositoryAuditRecord: { sessionId: "s1", scope: "repository", mode: "log" },
    });
  });


  it("executes a real Git diff audit without changing ongoing mode", async () => {
    const { root, home, daemon } = await fixture();
    await writeFile(join(root, "api/clean.py"), "value = 43\n");
    let output = "";
    const code = await runAuditCommand("scan", ["--session", "s1", "--scope", "git-diff", "--wait", "--json"], {
      home, stdout: { write: text => { output += text; return true; } },
    });
    expect(code).toBe(0);
    const request = auditRequestSchema.parse(JSON.parse(output));
    expect(request).toMatchObject({ status: "completed", mode: "log", error: null });
    const response = await fetch(new URL("/api/audits?sessionId=s1", daemon.url));
    expect(await response.json()).toMatchObject({ audits: expect.arrayContaining([
      expect.objectContaining({ auditId: request.auditIds[0], scope: "touched-files", mode: "log", status: "completed", violations: [], failures: [] }),
    ]) });
    const settings = auditControlsResponseSchema.parse(await (await fetch(new URL("/api/sessions/s1/audit-settings", daemon.url))).json());
    expect(settings.settings.mode).toBe("steer");
  });

  it("reports an empty Git diff without starting an audit", async () => {
    const { root, home } = await fixture();
    let output = "";
    const code = await runAuditCommand("scan", ["--session", "s1", "--scope", "git-diff", "--wait", "--json"], {
      home, stdout: { write: text => { output += text; return true; } },
    });
    expect(code).toBe(1);
    const request = auditRequestSchema.parse(JSON.parse(output));
    expect(request).toMatchObject({ status: "failed", auditIds: [], error: expect.stringMatching(/no changed files/i) });
  });

  it("interrupts audits and closes the session store despite undiscovered source edits", async () => {
    const { root, database, runtime, daemon } = await fixture();
    const controls = new SqliteAuditControlStore(database);
    await rm(join(root, "api/clean.py"));
    const request = controls.requestAudit("s1", { scope: "repository" });
    await runtime.closeSession("s1");
    expect(controls.listAuditRequests("s1").find(item => item.id === request.id)).toMatchObject({ status: "interrupted" });
    expect(await (await fetch(new URL("/api/sessions", daemon.url))).json()).toMatchObject({
      sessions: [expect.objectContaining({ sessionId: "s1", endedAt: expect.any(Number) })],
    });
    expect(controls.getSettings("s1").active).toBe(false);
    controls.close();
  });
  it("keeps another live runtime able to audit after its repository history is reset", async () => {
    const { home, database, repositoryKey } = await fixture();
    resetRepositoryHistory(database, repositoryKey);
    let output = "";
    expect(await runAuditCommand("scan", ["--session", "s1", "--scope", "repository", "--wait", "--json"], {
      home, stdout: { write: text => { output += text; return true; } },
    })).toBe(0);
    expect(auditRequestSchema.parse(JSON.parse(output))).toMatchObject({ status: "completed" });
  });

  it("applies saved log mode to new edits while an explicit audit is still running", async () => {
    const gate = Promise.withResolvers<void>();
    const { root, database, runtime, daemon } = await fixture(options => {
      const auditor = new NativeCodeRuleEnforcer(testCodeRuleCatalog([NO_PRINT]), options);
      const audit = auditor.audit.bind(auditor);
      let auditRequestCount = 0;
      auditor.audit = async (...args) => {
        auditRequestCount += 1;
        if (auditRequestCount > 1) await gate.promise;
        return audit(...args);
      };
      return auditor;
    });
    const controls = new SqliteAuditControlStore(database);
    let completion: Promise<readonly unknown[]> | undefined;
    try {
      await runtime.ingest({
        kind: "call", eventId: "write-start", sessionId: "s1",
        toolCallId: "write", toolName: "write", input: { path: "api/clean.py" },
      });
      const request = controls.requestAudit("s1", { scope: "repository" });
      await expect.poll(() => controls.listAuditRequests("s1").find(item => item.id === request.id)?.status).toBe("running");
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
    await expect(runAuditCommand("scan", [], { home: "/unused" })).rejects.toThrow("--session is required");
  });
});
