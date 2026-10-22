import { mkdir, mkdtemp, readFile, rm, truncate, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  NativeCodeRuleEnforcer,
  type AuditRecorder,
  type AuditInput,
  type AuditResult,
  defaultRerailDatabasePath,
  SqliteAuditReader,
  SqliteAuditStore,
  bundledCodeRuleDslDirectory,
  codeRuleSchema,
} from "../src/engine/index.js";
import { testCodeRuleCatalog } from "../test/support/repository-rules.js";

const directories: string[] = [];
afterEach(async () => {
  await Promise.all(directories.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

async function rootDirectory(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-executable-policy-"));
  directories.push(root);
  await mkdir(join(root, "api"));
  return root;
}

async function settled(enforcer: NativeCodeRuleEnforcer) {
  await expect.poll(() => enforcer.snapshotStatus().pending, { timeout: 15_000, interval: 10 }).toBe(false);
  return enforcer.snapshotStatus();
}

const printCodeRule = {
  format: "rerail.policy/v1",
  id: "test:print",
  revision: 1,
  mode: "steer",
  scope: { include: ["api/**/*.py"], exclude: [] },
  clauses: [
    {
      id: "print-clause",
      violation: { code: "PRINT", anchor: "witness.span", message: "Prefer logging" },
      forbid: { ast: { language: "python", rule: { pattern: "print($$$ARGS)" } } },
    },
  ],
};

/** E1: an exported component without a story of the same name beside it. */
const componentStoryCodeRule = {
  format: "rerail.policy/v1",
  id: "test:component-story",
  revision: 1,
  mode: "steer",
  scope: { include: ["api/**/*.tsx"], exclude: [] },
  clauses: [
    {
      id: "component-without-story",
      violation: { code: "STORY", anchor: "witness.span", message: "Component has no story" },
      require_each: {
        subject: {
          evidence: {
            as: "component",
            paths: { exclude: ["**/*.stories.tsx"] },
            query: { ast: { language: "typescript", rule: { pattern: "export function $NAME($$$PARAMS) { $$$BODY }" } } },
            project: { name: "capture.NAME", directory: "file.directory" },
          },
        },
        evidence: {
          evidence: {
            as: "story",
            paths: { include: ["**/*.stories.tsx"] },
            query: { ast: { language: "typescript", rule: { pattern: "export default $META" } } },
            project: { name: "file.stem", directory: "file.directory" },
            where: { all: [{ eq: ["component.name", "story.name"] }, { eq: ["component.directory", "story.directory"] }] },
          },
        },
      },
    },
  ],
};

/** An async function's un-awaited call that reaches blocking I/O through sync functions, in any file. */
const asyncBlockingCodeRule = {
  format: "rerail.policy/v1",
  id: "test:async-blocking",
  revision: 1,
  mode: "steer",
  scope: { include: ["api/**/*.py"], exclude: [] },
  clauses: [
    {
      id: "async-blocking",
      violation: { code: "BLOCKING", anchor: "witness.span", message: "blocks via $CHAIN" },
      forbid: {
        ast: {
          language: "python",
          rule: {
            kind: "call",
            not: { inside: { kind: "await" } },
            inside: { kind: "function_definition", regex: "^async ", stopBy: { kind: "function_definition" } },
            calls: {
              through: "sync",
              chain: "CHAIN",
              has: {
                stopBy: { kind: "function_definition" },
                kind: "call",
                has: { field: "function", resolves: { import: ["time.sleep"] } },
              },
            },
          },
        },
      },
    },
  ],
};

async function fixture(
  root: string,
  options: {
    scope?: "repository" | "touched-files" | "ai-output";
    mode?: "steer" | "log";
    audits?: AuditRecorder;
    timingLog?: (line: string) => void;
  } = {},
  limits: Partial<{ maxSourceBytes: number; maxVisitedNodes: number; maxVisitsPerNode: number }> = {},
  codeRule: unknown = printCodeRule,
  ...moreCodeRules: unknown[]
) {
  const capabilitiesPath = join(root, "capabilities.json");
  const capabilities: { limits: { maxSourceBytes: number; maxVisitedNodes: number; maxVisitsPerNode: number } } = JSON.parse(
    await readFile(join(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8"),
  );
  Object.assign(capabilities.limits, limits);
  await writeFile(capabilitiesPath, JSON.stringify(capabilities));
  const catalog = testCodeRuleCatalog([codeRule, ...moreCodeRules].map((rule) => codeRuleSchema.parse(rule)), capabilitiesPath);
  const deliveries: string[] = [];
  const enforcer = new NativeCodeRuleEnforcer(catalog, {
    workspaceRoot: root,
    steerDelivery: {
      async requestSteer(_sessionId, _kind, content) {
        deliveries.push(content);
      },
    },
    ...options,
  });
  return { deliveries, enforcer };
}
describe("Rerail code rule runtime", () => {
  it("starts generation without waiting for, or inheriting violations from, the initial repository audit", async () => {
    const root = await rootDirectory();
    await writeFile(join(root, "api", "existing.py"), "print('existing')\n");
    const { deliveries, enforcer } = await fixture(root, { scope: "touched-files", mode: "steer" });
    try {
      await enforcer.openWorkspace("generation");
      const repositoryAudit = enforcer.audit("generation", { scope: "repository", mode: "log" });
      const generated = "print('generated')\n";
      await writeFile(join(root, "api", "generated.py"), generated);
      await enforcer.auditGeneratedEdits("generation", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/generated.py", language: "python", generatedText: generated }],
      });
      const result = await settled(enforcer);
      await repositoryAudit;
      expect(result.violations).toEqual([expect.objectContaining({ path: "api/generated.py", code: "PRINT" })]);
      expect(deliveries).toHaveLength(1);
    } finally {
      await enforcer.close();
    }
  });

  it("removes a violation when captured generated code deletes it", async () => {
    const root = await rootDirectory();
    const { enforcer } = await fixture(root);
    try {
      await enforcer.openWorkspace("deletion");
      await enforcer.auditGeneratedEdits("deletion", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/generated.py", language: "python", generatedText: "print('bad')\n" }],
      });
      await settled(enforcer);
      await enforcer.auditGeneratedEdits("deletion", {
        revision: 2,
        checkpoints: ["edit"],
        edits: [{ path: "api/generated.py", language: "python", previousText: "print('bad')\n", deleted: true }],
      });
      expect((await settled(enforcer)).violations).toEqual([]);
    } finally {
      await enforcer.close();
    }
  });

  it("excludes untouched code from AI-output audit history", async () => {
    const root = await rootDirectory();
    const generated = "print('old')\nprint('generated')\n";
    const audits: Array<{ input: AuditInput; result?: AuditResult }> = [];
    const recorder: AuditRecorder = {
      start(input) { audits.push({ input }); return `${audits.length}`; },
      finish(auditId, result) { audits[Number(auditId) - 1]!.result = result; },
      progress() {},
      interrupt() {},
    };
    const { enforcer } = await fixture(root, { scope: "ai-output", audits: recorder });
    try {
      await enforcer.openWorkspace("ai-output");
      await enforcer.auditGeneratedEdits("ai-output", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/generated.py", language: "python", previousText: "print('old')\n", generatedText: generated }],
      });
      await settled(enforcer);
      expect(audits.find(({ input }) => input.scope === "ai-output")?.input.ruleProvenance).toEqual([
        expect.objectContaining({
          ruleId: "test:print",
          revision: 1,
          source: { kind: "library_rule", id: "test:print", packId: null },
          placement: [expect.objectContaining({ folder: "", on: true, mode: "steer" })],
        }),
      ]);
      expect(audits.find(({ input }) => input.scope === "ai-output")?.result?.violations).toEqual([
        expect.objectContaining({ path: "api/generated.py", code: "PRINT" }),
      ]);
    } finally {
      await enforcer.close();
    }
  });

  it("keeps an in-scope code rule evaluable beside more than 10000 out-of-scope source files and edits", async () => {
    const root = await rootDirectory();
    await mkdir(join(root, "web"));
    await writeFile(join(root, "web", "bundle.py"), `# ${"x".repeat(100_000)}\n`);
    // Exceed the former workspace-wide attachment cap without unbounded concurrent writes.
    for (let start = 0; start < 10_001; start += 100) {
      await Promise.all(Array.from({ length: Math.min(100, 10_001 - start) }, (_, offset) =>
        writeFile(join(root, "web", `${start + offset}.py`), ""),
      ));
    }
    await writeFile(join(root, "api", "route.py"), "print('in scope')\n");
    const { deliveries, enforcer } = await fixture(root, {}, { maxSourceBytes: 64 });
    const reader = new SqliteAuditReader(defaultRerailDatabasePath());
    try {
      await enforcer.openWorkspace("scope-budget");
      await enforcer.audit("scope-budget", { scope: "repository", mode: "log" });
      expect(deliveries).toEqual([]);
      expect(reader.list({ scope: "repository", sessionId: "scope-budget" }).audits).toEqual([
        expect.objectContaining({
          status: "completed",
          failures: [],
          violations: [expect.objectContaining({ path: "api/route.py", code: "PRINT" })],
        }),
      ]);
      await enforcer.audit("scope-budget", { scope: "repository", mode: "steer" });
      expect(deliveries).toEqual([expect.stringContaining("api/route.py")]);
      await enforcer.auditGeneratedEdits("scope-budget", {
        revision: 1, checkpoints: ["edit"],
        edits: [{ path: "web/bundle.py", language: "python", generatedText: "print('out of scope')\n" }],
      });
      await settled(enforcer);
      expect(deliveries).toHaveLength(1);
      await enforcer.auditGeneratedEdits("scope-budget", {
        revision: 2, checkpoints: ["edit"],
        edits: [{ path: "api/route.py", language: "python", generatedText: "print('changed')\n" }],
      });
      const result = await settled(enforcer);
      expect(result.failures).toEqual([]);
      expect(result.violations).toEqual([expect.objectContaining({ path: "api/route.py", code: "PRINT" })]);
      expect(deliveries).toHaveLength(2);
    } finally {
      await enforcer.close();
      reader.close();
    }
  }, 60_000);

  it("reports the whole file of an edit whose earlier text is unknown, and only the generated lines once it is known", async () => {
    const root = await rootDirectory();
    const { deliveries, enforcer } = await fixture(root, { scope: "ai-output" });
    const reader = new SqliteAuditReader(defaultRerailDatabasePath());
    try {
      await enforcer.openWorkspace("missing-preimage");
      // Its generated lines are unknown: a line it never wrote may be one of them, so the file is reported whole.
      const unknown = "print('possibly old')\nvalue = 1\n";
      await enforcer.auditGeneratedEdits("missing-preimage", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/existing.py", language: "python", previousReadError: "Pre-edit source was unreadable", generatedText: unknown }],
      });
      const first = await settled(enforcer);
      expect(first.failures).toEqual([]);
      expect(deliveries).toEqual([expect.stringContaining("api/existing.py:1:")]);
      // The next edit knows what it changed: the old print is not its own.
      await enforcer.auditGeneratedEdits("missing-preimage", {
        revision: 2,
        checkpoints: ["edit"],
        edits: [{ path: "api/existing.py", language: "python", previousText: unknown, generatedText: "print('possibly old')\nvalue = 2\n" }],
      });
      await settled(enforcer);
      expect(deliveries).toHaveLength(1);
      expect(reader.list({ scope: "ai-output", sessionId: "missing-preimage" }).audits.map(audit => [audit.status, audit.violations.length, audit.failures]))
        .toEqual([["completed", 0, []], ["completed", 1, []]]);
    } finally {
      await enforcer.close();
      reader.close();
    }
  });

  it("records repository audits and only delivers them in steer mode", async () => {
    const root = await rootDirectory();
    await writeFile(join(root, "api", "existing.py"), "print('existing')\n");
    const { deliveries, enforcer } = await fixture(root);
    try {
      await enforcer.openWorkspace("diagnostic");
      await enforcer.audit("diagnostic", { scope: "repository", mode: "log" });
      expect(deliveries).toEqual([]);
      await enforcer.audit("diagnostic", { scope: "repository", mode: "steer" });
      expect(deliveries).toHaveLength(1);
    } finally {
      await enforcer.close();
    }
  });

  it("accepts a valid source file within the engine's byte limit", async () => {
    const root = await rootDirectory();
    await writeFile(join(root, "api", "large.py"), `# ${"x".repeat(20_000_000)}\nprint('large')\n`);
    const audits: Array<{ input: AuditInput; result?: AuditResult }> = [];
    const recorder: AuditRecorder = {
      start(input) { audits.push({ input }); return `${audits.length}`; },
      finish(auditId, result) { audits[Number(auditId) - 1]!.result = result; },
      progress() {},
      interrupt() {},
    };
    const { enforcer } = await fixture(root, { audits: recorder });
    try {
      await enforcer.openWorkspace("large-source");
      await enforcer.audit("large-source", { scope: "repository", mode: "log" });
      expect(audits[0]?.result).toMatchObject({
        status: "completed",
        failures: [],
        violations: [expect.objectContaining({ code: "PRINT", path: "api/large.py" })],
      });
    } finally {
      await enforcer.close();
    }
  });

  it("lists at most 20 read failures and keeps scanning the files after them", async () => {
    const root = await rootDirectory();
    await Promise.all(Array.from({ length: 24 }, async (_, index) => {
      const path = join(root, "api", `oversized-${index}.py`);
      await writeFile(path, "");
      await truncate(path, 32_000_001);
    }));
    await Promise.all(Array.from({ length: 40 }, (_, index) =>
      writeFile(join(root, "api", `route-${index}.py`), "print('in scope')\n"),
    ));
    const audits: Array<{ input: AuditInput; result?: AuditResult }> = [];
    const progress: Array<Pick<AuditResult, "failures" | "unlistedFailureCount">> = [];
    const recorder: AuditRecorder = {
      start(input) { audits.push({ input }); return `${audits.length}`; },
      finish(auditId, result) { audits[Number(auditId) - 1]!.result = result; },
      progress(_auditId, result) { progress.push(result); },
      interrupt() {},
    };
    const { enforcer } = await fixture(root, { audits: recorder });
    try {
      await enforcer.openWorkspace("bounded-audit");
      await enforcer.audit("bounded-audit", { scope: "repository", mode: "log" });
      expect(audits).toEqual([
        expect.objectContaining({
          result: expect.objectContaining({
            status: "failed",
            // The list stops at 20; nothing claims the scan stopped, and it did not.
            failures: Array.from({ length: 20 }, () => expect.objectContaining({
              code: "budget-exceeded",
              clauseId: "source",
              stage: "read",
            })),
          }),
        }),
      ]);
      expect(audits[0]?.result?.violations).toHaveLength(40);
      // A running audit's list is capped as its finished one is, with the count it leaves out.
      expect(progress.length).toBeGreaterThan(0);
      expect(progress.every(({ failures }) => (failures?.length ?? 0) <= 20)).toBe(true);
      expect(progress.at(-1)).toMatchObject({ unlistedFailureCount: 4 });
    } finally {
      await enforcer.close();
    }
  }, 60_000);

  it("keeps how many failures the list leaves out, and counts them in the summary", async () => {
    const root = await rootDirectory();
    await Promise.all(Array.from({ length: 24 }, async (_, index) => {
      const path = join(root, "api", `oversized-${index}.py`);
      await writeFile(path, "");
      await truncate(path, 32_000_001);
    }));
    await writeFile(join(root, "api", "route.py"), "print('in scope')\n");
    const store = new SqliteAuditStore(join(root, "audits.sqlite"));
    // The store records result pages; without them it records the finished result at once.
    const direct: AuditRecorder = {
      start: (input) => store.start(input),
      finish: (auditId, result) => store.finish(auditId, result),
      progress: (auditId, result) => store.progress(auditId, result),
      interrupt: (sessionId) => store.interrupt(sessionId),
    };
    try {
      for (const [sessionId, audits] of [["direct", direct], ["paged", store]] as const) {
        const { enforcer } = await fixture(root, { audits });
        try {
          await enforcer.openWorkspace(sessionId);
          await enforcer.audit(sessionId, { scope: "repository", mode: "log" });
        } finally {
          await enforcer.close();
        }
        const [audit] = store.list({ scope: "repository", sessionId }).audits;
        expect(audit, sessionId).toMatchObject({ status: "failed", failureCount: 24, unlistedFailureCount: 4 });
        expect(audit?.failures, sessionId).toHaveLength(20);
        expect(store.latestSummary(sessionId), sessionId).toMatchObject({ failureCount: 24, unlistedFailureCount: 4 });
      }
    } finally {
      store.close();
    }
  }, 60_000);

  it("lists once a file whose syntax kept every rule out: a repository audit completes, a touched-files scan fails", async () => {
    const root = await rootDirectory();
    await writeFile(join(root, "api", "broken.py"), "print(3)\ndef (:\n");
    await writeFile(join(root, "api", "route.py"), "print('checked')\n");
    // Unsound under syntax errors: the rule does not run on a file that has them.
    const unsound = {
      ...printCodeRule,
      id: "test:unsound",
      clauses: [{
        id: "unsound",
        violation: { code: "CALL", anchor: "witness.span", message: "call" },
        forbid: { ast: { language: "python", rule: { kind: "call", not: { follows: { kind: "import_statement", stopBy: "end" } } } } },
      }],
    };
    const audits: Array<{ input: AuditInput; result?: AuditResult }> = [];
    const recorder: AuditRecorder = {
      start(input) { audits.push({ input }); return `${audits.length}`; },
      finish(auditId, result) { audits[Number(auditId) - 1]!.result = result; },
      progress() {},
      interrupt() {},
    };
    const { enforcer } = await fixture(root, { audits: recorder }, {}, unsound);
    try {
      await enforcer.openWorkspace("syntax-blind");
      await enforcer.audit("syntax-blind", { scope: "repository", mode: "log" });
      await enforcer.audit("syntax-blind", { scope: "touched-files", mode: "log", paths: ["api/broken.py"] });
      const blind = [expect.objectContaining({ path: "api/broken.py", code: "source-incomplete", blind: "syntax" })];
      expect(audits[0]?.result).toMatchObject({
        status: "completed",
        failures: blind,
        violations: [expect.objectContaining({ path: "api/route.py", code: "CALL" })],
      });
      expect(audits[1]?.result).toMatchObject({ status: "failed", failures: blind });
    } finally {
      await enforcer.close();
    }
  });

  it("lists every file no rule could check, once each, without failing the audit", async () => {
    const root = await rootDirectory();
    await Promise.all(Array.from({ length: 24 }, (_, index) =>
      writeFile(join(root, "api", `complex-${index}.py`), "print('one')\nprint('two')\n"),
    ));
    const audits: Array<{ input: AuditInput; result?: AuditResult }> = [];
    const recorder: AuditRecorder = {
      start(input) { audits.push({ input }); return `${audits.length}`; },
      finish(auditId, result) { audits[Number(auditId) - 1]!.result = result; },
      progress() {},
      interrupt() {},
    };
    const { enforcer } = await fixture(root, { audits: recorder }, { maxVisitedNodes: 1, maxVisitsPerNode: 0 });
    try {
      await enforcer.openWorkspace("bounded-ast-audit");
      await enforcer.audit("bounded-ast-audit", { scope: "repository", mode: "log" });
      expect(audits[0]?.result?.status).toBe("completed");
      expect(audits[0]?.result?.failures).toEqual(Array.from({ length: 24 }, () => expect.objectContaining({
        code: "budget-exceeded",
        stage: "evaluate",
        blind: "budget",
      })));
      expect(new Set(audits[0]?.result?.failures?.map((failure) => failure.path)).size).toBe(24);
    } finally {
      await enforcer.close();
    }
  });

  it("lists every file no rule could check through result pages", async () => {
    const root = await rootDirectory();
    await Promise.all(Array.from({ length: 24 }, (_, index) =>
      writeFile(join(root, "api", `complex-paged-${index}.py`), "print('one')\nprint('two')\n"),
    ));
    const pages: Array<Pick<AuditResult, "violations" | "failures">> = [];
    const statuses: string[] = [];
    const recorder: AuditRecorder = {
      supportsResultPages: true,
      start() { return "paged"; },
      finish() { throw new Error("paged recorder must finish pages"); },
      progress() {},
      appendResultPage(_auditId, _pageIndex, result) { pages.push(result); },
      finishResultPages(_auditId, status) { statuses.push(status); },
      interrupt() {},
    };
    const { enforcer } = await fixture(root, { audits: recorder }, { maxVisitedNodes: 1, maxVisitsPerNode: 0 });
    try {
      await enforcer.openWorkspace("paged-bounded-audit");
      await enforcer.audit("paged-bounded-audit", { scope: "repository", mode: "log" });
      expect(pages.flatMap((page) => page.failures ?? [])).toEqual(Array.from({ length: 24 }, () => expect.objectContaining({
        code: "budget-exceeded",
        stage: "evaluate",
        blind: "budget",
      })));
      expect(statuses).toEqual(["completed"]);
    } finally {
      await enforcer.close();
    }
  });

  it("completes a paged repository audit whose only failures are files with syntax errors, listing at most 20 of them", async () => {
    const root = await rootDirectory();
    await Promise.all(Array.from({ length: 24 }, (_, index) =>
      writeFile(join(root, "api", `broken-${String(index).padStart(2, "0")}.py`), "def f(:\n    pass\nprint('x')\n"),
    ));
    await writeFile(join(root, "api", "zz_clean.py"), "print('clean')\n");
    const pages: Array<Pick<AuditResult, "violations" | "failures">> = [];
    const statuses: string[] = [];
    const unlistedCounts: number[] = [];
    const recorder: AuditRecorder = {
      supportsResultPages: true,
      start() { return "paged"; },
      finish() { throw new Error("paged recorder must finish pages"); },
      progress() {},
      appendResultPage(_auditId, _pageIndex, result) { pages.push(result); },
      finishResultPages(_auditId, status, unlisted) { statuses.push(status); unlistedCounts.push(unlisted ?? 0); },
      interrupt() {},
    };
    const timing: string[] = [];
    const { enforcer } = await fixture(root, { audits: recorder, timingLog: line => timing.push(line) });
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    try {
      await enforcer.openWorkspace("paged-syntax-audit");
      await enforcer.audit("paged-syntax-audit", { scope: "repository", mode: "log" });
      // The daemon log counts every violation and failure of a paged audit, though none are kept in memory.
      const violationCount = pages.flatMap((page) => page.violations ?? []).length;
      expect(violationCount).toBeGreaterThan(0);
      expect(timing.join(""))
        .toMatch(new RegExp(`repository audit completed in \\d+ ms .*: \\d+ files, ${violationCount} violations, 24 failures`));
      expect(stderr.mock.calls.map(([chunk]) => String(chunk)).join("")).not.toContain("repository audit");
      expect(statuses).toEqual(["completed"]);
      expect(pages.flatMap((page) => page.violations ?? []).map((violation) => violation.path)).toContain("api/zz_clean.py");
      // At most 20 are listed; the audit keeps how many it left out, and that never fails it.
      expect(pages.flatMap((page) => page.failures ?? [])).toEqual(
        Array.from({ length: 20 }, () => expect.objectContaining({ code: "source-partial", stage: "evaluate" })),
      );
      expect(unlistedCounts).toEqual([4]);
    } finally {
      stderr.mockRestore();
      await enforcer.close();
    }
  });
  it("completes an audit where a rule runs out of work on one file, and says what would let it finish", async () => {
    const root = await rootDirectory();
    // Each integer compares itself with every one before it: quadratic in a list of 20,000, past any allowance the
    // file earns, while the same rule finishes on a list of 2,000.
    const numbers = Array.from({ length: 20_000 }, (_, index) => index).join(", ");
    await writeFile(join(root, "api", "generated.py"), `x = [${numbers}]\n`);
    await writeFile(join(root, "api", "route.py"), "print('checked')\n");
    const costlyCodeRule = {
      ...printCodeRule,
      id: "test:costly",
      clauses: [
        ...printCodeRule.clauses,
        {
          id: "costly",
          violation: { code: "SLOW", anchor: "witness.span", message: "slow" },
          forbid: {
            ast: {
              language: "python",
              rule: {
                kind: "integer", pattern: "$A",
                not: { follows: { stopBy: "end", kind: "integer", pattern: "$B", compare: { left: "A", op: "le", right: "B" } } },
              },
            },
          },
        },
      ],
    };
    const audits: Array<{ input: AuditInput; result?: AuditResult }> = [];
    const recorder: AuditRecorder = {
      start(input) { audits.push({ input }); return `${audits.length}`; },
      finish(auditId, result) { audits[Number(auditId) - 1]!.result = result; },
      progress() {},
      interrupt() {},
    };
    const { enforcer } = await fixture(root, { audits: recorder }, {}, costlyCodeRule);
    try {
      await enforcer.openWorkspace("out-of-work");
      await enforcer.audit("out-of-work", { scope: "repository", mode: "log" });
      expect(audits[0]?.result).toMatchObject({
        status: "completed",
        violations: [expect.objectContaining({ path: "api/route.py", code: "PRINT" })],
        failures: [expect.objectContaining({
          clauseId: "costly", code: "budget-exceeded", path: "api/generated.py", stage: "evaluate",
          message: expect.stringContaining("simplify the rule, or exclude generated or vendored files from its scope"),
        })],
      });
    } finally {
      await enforcer.close();
    }
  }, 60_000);

  it("lists a failure that fails the audit ahead of coverage gaps, however many gaps come before it", async () => {
    const root = await rootDirectory();
    // `first` runs out on 25 files; `second` fails on a later file with syntax errors, which fails a scan of touched
    // files (a repository audit lists it as a file read in part).
    await mkdir(join(root, "api", "first"));
    await mkdir(join(root, "api", "second"));
    const integers = `x = [${Array.from({ length: 200 }, (_, index) => index).join(", ")}]\n`;
    await Promise.all([
      ...Array.from({ length: 25 }, (_, index) => writeFile(join(root, "api", "first", `f${String(index).padStart(2, "0")}.py`), integers)),
      writeFile(join(root, "api", "second", "z.py"), "x = 1\ndef (\n"),
    ]);
    // A regex clause that finishes keeps each file from being blind.
    const rule = (id: string, folder: string) => ({
      format: "rerail.policy/v1", id, revision: 1, mode: "log",
      scope: { include: [`api/${folder}/**`], exclude: [] },
      clauses: [
        { id: "integers", violation: { code: "INT", anchor: "witness.span", message: "int" }, forbid: { ast: { language: "python", rule: { kind: "integer" } } } },
        { id: "marker", violation: { code: "MARK", anchor: "witness.span", message: "mark" }, forbid: { regex: { source: "file.content", pattern: "NEVER_PRESENT" } } },
      ],
    });
    const store = new SqliteAuditStore(join(root, "audits.sqlite"));
    const direct: AuditRecorder = {
      start: (input) => store.start(input),
      finish: (auditId, result) => store.finish(auditId, result),
      progress: (auditId, result) => store.progress(auditId, result),
      interrupt: (sessionId) => store.interrupt(sessionId),
    };
    try {
      for (const [sessionId, audits] of [["direct", direct], ["paged", store]] as const) {
        const { enforcer } = await fixture(root, { audits }, { maxVisitedNodes: 50, maxVisitsPerNode: 0 },
          rule("test:first", "first"), rule("test:second", "second"));
        const paths = [...Array.from({ length: 25 }, (_, index) => `api/first/f${String(index).padStart(2, "0")}.py`), "api/second/z.py"];
        try {
          await enforcer.openWorkspace(sessionId);
          await enforcer.audit(sessionId, { scope: "touched-files", mode: "log", paths });
        } finally {
          await enforcer.close();
        }
        const [audit] = store.list({ scope: "touched-files", sessionId }).audits;
        expect(audit?.status, sessionId).toBe("failed");
        expect(audit?.failures.filter((failure) => failure.ruleId === "test:second"), sessionId).toEqual([
          expect.objectContaining({ path: "api/second/z.py", code: expect.stringMatching(/^source-/) }),
        ]);
      }
    } finally {
      store.close();
    }
  }, 60_000);

  it("names every rule that ran out of work, and counts it, however many failures one rule had before it", async () => {
    const root = await rootDirectory();
    // `first` runs out on 25 files, past the 20 failures listed besides; `second` runs out on one file after them.
    await mkdir(join(root, "api", "first"));
    await mkdir(join(root, "api", "second"));
    const integers = `x = [${Array.from({ length: 200 }, (_, index) => index).join(", ")}]\n`;
    await Promise.all([
      ...Array.from({ length: 25 }, (_, index) => writeFile(join(root, "api", "first", `f${String(index).padStart(2, "0")}.py`), integers)),
      writeFile(join(root, "api", "second", "z.py"), integers),
    ]);
    // A regex clause that finishes keeps each file from being blind: only the AST clause runs out.
    const rule = (id: string, folder: string) => ({
      format: "rerail.policy/v1", id, revision: 1, mode: "log",
      scope: { include: [`api/${folder}/**`], exclude: [] },
      clauses: [
        { id: "integers", violation: { code: "INT", anchor: "witness.span", message: "int" }, forbid: { ast: { language: "python", rule: { kind: "integer" } } } },
        { id: "marker", violation: { code: "MARK", anchor: "witness.span", message: "mark" }, forbid: { regex: { source: "file.content", pattern: "NEVER_PRESENT" } } },
      ],
    });
    const store = new SqliteAuditStore(join(root, "audits.sqlite"));
    // The store records result pages; without them it records the finished result at once.
    const direct: AuditRecorder = {
      start: (input) => store.start(input),
      finish: (auditId, result) => store.finish(auditId, result),
      progress: (auditId, result) => store.progress(auditId, result),
      interrupt: (sessionId) => store.interrupt(sessionId),
    };
    try {
      for (const [sessionId, audits] of [["direct", direct], ["paged", store]] as const) {
        const { enforcer } = await fixture(root, { audits }, { maxVisitedNodes: 50, maxVisitsPerNode: 0 },
          rule("test:first", "first"), rule("test:second", "second"));
        try {
          await enforcer.openWorkspace(sessionId);
          await enforcer.audit(sessionId, { scope: "repository", mode: "log" });
        } finally {
          await enforcer.close();
        }
        const [audit] = store.list({ scope: "repository", sessionId }).audits;
        expect(audit?.status, sessionId).toBe("completed");
        expect(new Set(audit?.failures.map((failure) => failure.ruleId)), sessionId).toEqual(new Set(["test:first", "test:second"]));
        expect(audit?.failures.find((failure) => failure.ruleId === "test:second"), sessionId).toMatchObject({
          clauseId: "integers", code: "budget-exceeded", path: "api/second/z.py",
          message: expect.stringContaining("simplify the rule, or exclude generated or vendored files from its scope"),
        });
        // One failure of each rule, and 20 others, are listed; the summary counts all 26.
        expect(store.latestSummary(sessionId), sessionId).toMatchObject({
          status: "completed", failureCount: 26, unlistedFailureCount: 4, gapCount: 22, blindFileCount: 0, outOfWorkRuleCount: 2,
        });
      }
    } finally {
      store.close();
    }
  }, 60_000);

  it("records touched-files audits without delivery in log mode", async () => {
    const root = await rootDirectory();
    await writeFile(join(root, "api", "existing.py"), "print('existing')\n");
    const { deliveries, enforcer } = await fixture(root);
    try {
      await enforcer.openWorkspace("diagnostic");
      await enforcer.audit("diagnostic", { scope: "touched-files", mode: "log", paths: ["api/existing.py"] });
      expect(deliveries).toEqual([]);
    } finally {
      await enforcer.close();
    }
  });

  it("adds and retracts a cross-file evidence violation in an unchanged file", async () => {
    const root = await rootDirectory();
    const story = "export default { title: \"story\" };\n";
    await writeFile(join(root, "api", "Button.tsx"), "export function Button() { return null; }\n");
    await writeFile(join(root, "api", "Card.tsx"), "export function Card() { return null; }\n");
    await writeFile(join(root, "api", "Button.stories.tsx"), story);
    const audits: Array<{ input: AuditInput; result?: AuditResult }> = [];
    const recorder: AuditRecorder = {
      start(input) { audits.push({ input }); return `${audits.length}`; },
      finish(auditId, result) { audits[Number(auditId) - 1]!.result = result; },
      progress() {},
      interrupt() {},
    };
    const { enforcer } = await fixture(root, { scope: "repository", audits: recorder }, {}, componentStoryCodeRule);
    try {
      await enforcer.openWorkspace("stories");
      await enforcer.audit("stories", { scope: "repository", mode: "log" });
      expect(audits[0]?.result?.violations).toEqual([
        expect.objectContaining({ path: "api/Card.tsx", code: "STORY" }),
      ]);
      // Repository scope re-reads the checkout, as an agent's saved edit would be.
      await writeFile(join(root, "api", "Card.stories.tsx"), story);
      await enforcer.auditGeneratedEdits("stories", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/Card.stories.tsx", language: "typescript", generatedText: story }],
      });
      expect((await settled(enforcer)).violations).toEqual([]);
      await rm(join(root, "api", "Button.stories.tsx"));
      await enforcer.auditGeneratedEdits("stories", {
        revision: 2,
        checkpoints: ["edit"],
        edits: [{ path: "api/Button.stories.tsx", language: "typescript", previousText: story, deleted: true }],
      });
      expect((await settled(enforcer)).violations).toEqual([
        expect.objectContaining({ path: "api/Button.tsx", code: "STORY" }),
      ]);
    } finally {
      await enforcer.close();
    }
  });

  it("adds and retracts a calls violation in an unchanged file when a callee's file changes", async () => {
    const root = await rootDirectory();
    const sleeps = "import time\n\ndef pause():\n    time.sleep(1)\n";
    const idle = "def pause():\n    pass\n";
    await writeFile(join(root, "api", "__init__.py"), "");
    await writeFile(join(root, "api", "handler.py"), "from api.util import pause\n\nasync def handler():\n    pause()\n");
    await writeFile(join(root, "api", "util.py"), sleeps);
    const audits: Array<{ input: AuditInput; result?: AuditResult }> = [];
    const recorder: AuditRecorder = {
      start(input) { audits.push({ input }); return `${audits.length}`; },
      finish(auditId, result) { audits[Number(auditId) - 1]!.result = result; },
      progress() {},
      interrupt() {},
    };
    const { enforcer } = await fixture(root, { scope: "repository", audits: recorder }, {}, asyncBlockingCodeRule);
    try {
      await enforcer.openWorkspace("calls");
      await enforcer.audit("calls", { scope: "repository", mode: "log" });
      expect(audits[0]?.result?.violations).toEqual([
        expect.objectContaining({ path: "api/handler.py", code: "BLOCKING", message: "blocks via api.util.pause" }),
      ]);
      // The callee's file loses its blocking call: the violation in the unchanged caller goes.
      await writeFile(join(root, "api", "util.py"), idle);
      await enforcer.auditGeneratedEdits("calls", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/util.py", language: "python", generatedText: idle, previousText: sleeps }],
      });
      expect((await settled(enforcer)).violations).toEqual([]);
      await writeFile(join(root, "api", "util.py"), sleeps);
      await enforcer.auditGeneratedEdits("calls", {
        revision: 2,
        checkpoints: ["edit"],
        edits: [{ path: "api/util.py", language: "python", generatedText: sleeps, previousText: idle }],
      });
      expect((await settled(enforcer)).violations).toEqual([
        expect.objectContaining({ path: "api/handler.py", code: "BLOCKING" }),
      ]);
    } finally {
      await enforcer.close();
    }
  });
});
