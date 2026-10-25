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
  defaultRerailDatabasePath,
} from "../../src/engine/index.js";
import {
  RerailRuntime,
  type RerailRuntimeOptions,
  isRerailStarted,
  SqliteSessionStore,
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
      sessionId: () => "omp-s1",
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
    // Rerail may read sessions on this computer already (set-up's one approval; `setup-json.test.ts` asks it).
    await writeFile(join(process.env.RERAIL_HOME!, "consent.json"), JSON.stringify({ sessions: { approvedAt: "2026-10-16T00:00:00.000Z" } }));
    await commands.get("rerail-start")!.handler("", { ui });
    expect(asked).toEqual([`Find rule packs for ${name}?`]);
    expect(notices.at(-1)).toBe(SETUP_CANCELLED);
    expect(workspaceStartState(root)).toBe("stopped");
    // The person's "start" answer, run as the coding agent relays it.
    await startRerail(root, { lane: "agent", say: () => {} }, { answer: "start", environment: fakeSetupEnvironment() });
    expect(isRerailStarted(root)).toBe(true);

    // `/rerail-stop` names this session to the launcher, so the reply names the rule that steered it.
    const store = new SqliteSessionStore(defaultRerailDatabasePath());
    const session = { host: "omp" as const, sessionId: "omp-s1", cwd: root };
    try {
      store.openSession(session);
      store.recordSteer(session, "receipt-1", [NO_PRINT.id]);
    } finally {
      store.close();
    }
    await commands.get("rerail-stop")!.handler("", { ui });
    expect(isRerailStarted(root)).toBe(false);
    expect(notices.at(-1)).toBe([
      `○ Rerail is stopped in ${name}. No rules run here, and nothing is logged.`,
      "  Start it again with /rerail-start.",
      "",
      "  Rerail steered this session on one rule, test:no-print.",
      "  To turn off only that rule, say “turn that rule off”.",
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
        if (input.audit !== undefined && !input.audit.claim(options.workspaceRoot, "test-catalog")) return [];
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
      settings: { scope: "ai-output", mode: "steer" },
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

  it("saves the steer strategy for the session's checkout and keeps the other settings", async () => {
    const { home } = await fixture();
    let output = "";
    expect(await runAuditCommand("settings", ["--session", "s1", "--steer-strategy", "rewind"], {
      home, stdout: { write: text => { output += text; return true; } },
    })).toBe(0);
    expect(output).toMatch(/^Ongoing settings for .+: ai-output\/steer, steer strategy rewind \(/u);
    output = "";
    expect(await runAuditCommand("settings", ["--session", "s1", "--mode", "log", "--json"], {
      home, stdout: { write: text => { output += text; return true; } },
    })).toBe(0);
    expect(auditControlsResponseSchema.parse(JSON.parse(output)).settings).toEqual({ scope: "ai-output", mode: "log", steerStrategy: "rewind" });
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

  it("completes a repository scan whose only gap is a file every rule ran out of work on, and names that file", async () => {
    // Each integer compares itself with every one before it: quadratic, past the clause's allowance on a list of
    // 20,000, while a short list finishes.
    const COSTLY = codeRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:costly", revision: 1, mode: "log",
      scope: { include: ["api/**/*.py"], exclude: [] },
      clauses: [{
        id: "costly",
        violation: { code: "LEAST", anchor: "witness.span", message: "least so far" },
        forbid: { ast: { language: "python", rule: {
          kind: "integer", pattern: "$A",
          not: { follows: { stopBy: "end", kind: "integer", pattern: "$B", compare: { left: "A", op: "le", right: "B" } } },
        } } },
      }],
    });
    const { root, home, daemon } = await fixture(options => new NativeCodeRuleEnforcer(testCodeRuleCatalog([COSTLY]), options));
    await writeFile(join(root, "api/generated.py"), `x = [${Array.from({ length: 20_000 }, (_, index) => index).join(", ")}]\n`);
    await writeFile(join(root, "api/noisy.py"), "x = [1]\n");
    execFileSync("git", ["add", "api"], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-m", "generated"], { cwd: root });
    let output = "";
    const scanCode = await runAuditCommand("scan", ["--session", "s1", "--scope", "repository", "--wait", "--json"], {
      home, stdout: { write: text => { output += text; return true; } },
    });
    expect(scanCode, output).toBe(0);
    const request = auditRequestSchema.parse(JSON.parse(output));
    expect(request).toMatchObject({ status: "completed", error: null });
    const { audits } = await (await fetch(new URL(`/api/audits?auditId=${request.auditIds[0]}`, daemon.url))).json();
    expect(audits).toEqual([expect.objectContaining({
      status: "completed",
      blindFiles: ["api/generated.py"],
      blindFileCount: 1,
      failures: [expect.objectContaining({ path: "api/generated.py", code: "budget-exceeded", blind: "budget" })],
    })]);
    // The other files are read: the baseline's and the one added beside the generated file.
    expect(new Set(audits[0].violations.map((violation: { path: string }) => violation.path))).toEqual(new Set(["api/clean.py", "api/noisy.py"]));
  }, 30_000);

  it("fails a repository scan where a rule finds more than it may report on a file, which is no coverage gap", async () => {
    const { root, home, daemon, runtime } = await fixture();
    // The session's first walk is done before the writes, so it audits neither file on its own: the scan stays the
    // session's latest audit.
    await runtime.codeRulesOpened("s1");
    await runtime.ingest({ kind: "turn-end", eventId: "baseline", sessionId: "s1", text: "" });
    // More matches than the engine's 10,000-witness budget: an output cap, not running out of work.
    await writeFile(join(root, "api/generated.py"), "print(1)\n".repeat(10_001));
    await writeFile(join(root, "api/noisy.py"), "print('checked')\n");
    execFileSync("git", ["add", "api"], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-m", "generated"], { cwd: root });
    let output = "";
    expect(await runAuditCommand("scan", ["--session", "s1", "--scope", "repository", "--wait", "--json"], {
      home, stdout: { write: text => { output += text; return true; } },
    })).toBe(1);
    const request = auditRequestSchema.parse(JSON.parse(output));
    const { audits } = await (await fetch(new URL(`/api/audits?auditId=${request.auditIds[0]}`, daemon.url))).json();
    expect(audits).toEqual([expect.objectContaining({
      status: "failed",
      blindFiles: [],
      blindFileCount: 0,
      failures: [expect.objectContaining({ path: "api/generated.py", code: "witness-limit" })],
      violations: [expect.objectContaining({ path: "api/noisy.py", code: "PRINT" })],
    })]);
    expect(audits[0].failures[0]).not.toHaveProperty("blind");
    const session = await (await fetch(new URL("/api/sessions/s1", daemon.url))).json();
    expect(session.audit).toMatchObject({ status: "failed", gapCount: 0, outOfWorkRuleCount: 0 });
  });

  it("keeps how many failures a scan had past the 20 it lists, and reports them with the session", async () => {
    // An AST rule on files with syntax errors fails on each (`source-partial`), while the regex rule reads them, so
    // no file is blind: 25 files read in part, five past the list's 20. The session's own audit of them fails on them;
    // a repository scan lists them and completes.
    const PRINT_CALL = codeRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:print-call", revision: 1, mode: "log",
      scope: { include: ["api/**/*.py"], exclude: [] },
      clauses: [{
        id: "print-call",
        violation: { code: "PRINT_CALL", anchor: "witness.span", message: "Use logging" },
        forbid: { ast: { language: "python", rule: { pattern: "print($$$ARGS)" } } },
      }],
    });
    const { root, home, daemon } = await fixture(options => new NativeCodeRuleEnforcer(testCodeRuleCatalog([NO_PRINT, PRINT_CALL]), options));
    await Promise.all(Array.from({ length: 25 }, (_, index) =>
      writeFile(join(root, `api/broken-${String(index).padStart(2, "0")}.py`), "value = 1\ndef (\n")));
    execFileSync("git", ["add", "api"], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-m", "broken"], { cwd: root });
    // The session audits the 25 edits on its own; the scan must come after them to be the session's latest audit.
    await vi.waitFor(async () => {
      const { audits } = await (await fetch(new URL("/api/audits?sessionId=s1", daemon.url))).json();
      expect(audits[0]).toMatchObject({ scope: "ai-output", status: "failed", failureCount: 25 });
    }, { timeout: 20_000, interval: 50 });
    let output = "";
    expect(await runAuditCommand("scan", ["--session", "s1", "--scope", "repository", "--wait", "--json"], {
      home, stdout: { write: text => { output += text; return true; } },
    })).toBe(0);
    const request = auditRequestSchema.parse(JSON.parse(output));
    const { audits } = await (await fetch(new URL(`/api/audits?auditId=${request.auditIds[0]}`, daemon.url))).json();
    expect(audits).toEqual([expect.objectContaining({ status: "completed", failureCount: 25, unlistedFailureCount: 5 })]);
    expect(audits[0].failures).toHaveLength(20);
    const session = await (await fetch(new URL("/api/sessions/s1", daemon.url))).json();
    expect(session.audit).toMatchObject({ status: "completed", failureCount: 25, unlistedFailureCount: 5 });
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
          scope: "ai-output", mode: "log", violations: expect.arrayContaining([expect.objectContaining({ code: "PRINT" })]),
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

  it("refuses a steer strategy it does not know before reaching Rerail", async () => {
    await expect(runAuditCommand("settings", ["--session", "s1", "--steer-strategy", "undo"], { home: "/unused" }))
      .rejects.toThrow("--steer-strategy must be message, rewind, or subagent");
    await expect(runAuditCommand("settings", ["--session", "s1", "--steer-strategy"], { home: "/unused" }))
      .rejects.toThrow("--steer-strategy requires a value");
  });
});
