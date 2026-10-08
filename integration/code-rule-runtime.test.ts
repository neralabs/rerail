import { mkdir, mkdtemp, readFile, rm, truncate, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { NativeCodeRuleEnforcer } from "../src/code-rule-enforcer.js";
import type { AuditRecorder, AuditInput, AuditResult } from "../src/audit.js";
import { defaultRerailDatabasePath } from "../src/database.js";
import { SqliteAuditReader } from "../src/audit-store.js";
import { bootstrapCodeRuleDslCatalog, bundledCodeRuleDslDirectory } from "../src/code-rule-dsl-catalog.js";
import { codeRuleSchema } from "../src/code-rule-schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";

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
  } = {},
  limits: Partial<{ maxSourceBytes: number; maxVisitedNodes: number }> = {},
  codeRule: unknown = printCodeRule,
) {
  const capabilitiesPath = join(root, "capabilities.json");
  const capabilities: { limits: { maxSourceBytes: number; maxVisitedNodes: number } } = JSON.parse(
    await readFile(join(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8"),
  );
  Object.assign(capabilities.limits, limits);
  await writeFile(capabilitiesPath, JSON.stringify(capabilities));
  const databasePath = join(root, ".rerail", "code-rules.sqlite");
  const store = new SqliteRuleStore(databasePath);
  store.upsertCodeRule(codeRuleSchema.parse(codeRule));
  store.close();
  const catalog = bootstrapCodeRuleDslCatalog({ capabilitiesPath, databasePath });
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
  return { catalog, deliveries, enforcer };
}
describe("Rerail code rule runtime", () => {
  it("starts generation without waiting for, or inheriting violations from, the initial repository audit", async () => {
    const root = await rootDirectory();
    await writeFile(join(root, "api", "existing.py"), "print('existing')\n");
    const { catalog, deliveries, enforcer } = await fixture(root, { scope: "touched-files", mode: "steer" });
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
      catalog.close();
    }
  });

  it("removes a violation when captured generated code deletes it", async () => {
    const root = await rootDirectory();
    const { catalog, enforcer } = await fixture(root);
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
      catalog.close();
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
    const { catalog, enforcer } = await fixture(root, { scope: "ai-output", audits: recorder });
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
          applicability: "project",
          origin: "user",
        }),
      ]);
      expect(audits.find(({ input }) => input.scope === "ai-output")?.result?.violations).toEqual([
        expect.objectContaining({ path: "api/generated.py", code: "PRINT" }),
      ]);
    } finally {
      await enforcer.close();
      catalog.close();
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
    const { catalog, deliveries, enforcer } = await fixture(root, {}, { maxSourceBytes: 64 });
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
      catalog.close();
    }
  }, 60_000);

  it("records a scope failure rather than treating an unavailable preimage as new AI output", async () => {
    const root = await rootDirectory();
    const { catalog, deliveries, enforcer } = await fixture(root, { scope: "ai-output" });
    const reader = new SqliteAuditReader(defaultRerailDatabasePath());
    try {
      await enforcer.openWorkspace("missing-preimage");
      await enforcer.auditGeneratedEdits("missing-preimage", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{
          path: "api/existing.py", language: "python",
          previousReadError: "Pre-edit source was unreadable", generatedText: "print('possibly old')\n",
        }],
      });
      const result = await settled(enforcer);
      expect(deliveries).toEqual([]);
      expect(result.failures).toEqual([expect.objectContaining({ stage: "scope", code: "source-incomplete" })]);
      expect(reader.list({ scope: "ai-output", sessionId: "missing-preimage" }).audits).toEqual([
        expect.objectContaining({
          status: "failed", violations: [],
          failures: [expect.objectContaining({ stage: "scope", code: "source-incomplete", path: "api/existing.py" })],
        }),
      ]);
    } finally {
      await enforcer.close();
      reader.close();
      catalog.close();
    }
  });

  it("records repository audits and only delivers them in steer mode", async () => {
    const root = await rootDirectory();
    await writeFile(join(root, "api", "existing.py"), "print('existing')\n");
    const { catalog, deliveries, enforcer } = await fixture(root);
    try {
      await enforcer.openWorkspace("diagnostic");
      await enforcer.audit("diagnostic", { scope: "repository", mode: "log" });
      expect(deliveries).toEqual([]);
      await enforcer.audit("diagnostic", { scope: "repository", mode: "steer" });
      expect(deliveries).toHaveLength(1);
    } finally {
      await enforcer.close();
      catalog.close();
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
    const { catalog, enforcer } = await fixture(root, { audits: recorder });
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
      catalog.close();
    }
  });

  it("bounds repeated repository audit failures", async () => {
    const root = await rootDirectory();
    await Promise.all(Array.from({ length: 24 }, async (_, index) => {
      const path = join(root, "api", `oversized-${index}.py`);
      await writeFile(path, "");
      await truncate(path, 32_000_001);
    }));
    const audits: Array<{ input: AuditInput; result?: AuditResult }> = [];
    const recorder: AuditRecorder = {
      start(input) { audits.push({ input }); return `${audits.length}`; },
      finish(auditId, result) { audits[Number(auditId) - 1]!.result = result; },
      progress() {},
      interrupt() {},
    };
    const { catalog, enforcer } = await fixture(root, { audits: recorder });
    try {
      await enforcer.openWorkspace("bounded-audit");
      await enforcer.audit("bounded-audit", { scope: "repository", mode: "log" });
      expect(audits).toEqual([
        expect.objectContaining({
          result: expect.objectContaining({
            status: "failed",
            failures: [
              ...Array.from({ length: 20 }, () => expect.objectContaining({
                code: "budget-exceeded",
                clauseId: "source",
                stage: "read",
              })),
              expect.objectContaining({
                clauseId: "scan",
                code: "budget-exceeded",
                stage: "scan",
              }),
            ],
          }),
        }),
      ]);
    } finally {
      await enforcer.close();
      catalog.close();
    }
  });

  it("caps AST budget errors returned by a repository audit", async () => {
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
    const { catalog, enforcer } = await fixture(root, { audits: recorder }, { maxVisitedNodes: 1 });
    try {
      await enforcer.openWorkspace("bounded-ast-audit");
      await enforcer.audit("bounded-ast-audit", { scope: "repository", mode: "log" });
      expect(audits[0]?.result?.failures).toEqual([
        ...Array.from({ length: 20 }, () => expect.objectContaining({
          code: "budget-exceeded",
          stage: "evaluate",
        })),
        expect.objectContaining({
          clauseId: "scan",
          code: "budget-exceeded",
          stage: "scan",
        }),
      ]);
    } finally {
      await enforcer.close();
      catalog.close();
    }
  });

  it("caps failures persisted through result pages", async () => {
    const root = await rootDirectory();
    await Promise.all(Array.from({ length: 24 }, (_, index) =>
      writeFile(join(root, "api", `complex-paged-${index}.py`), "print('one')\nprint('two')\n"),
    ));
    const pages: Array<Pick<AuditResult, "violations" | "failures">> = [];
    const recorder: AuditRecorder = {
      supportsResultPages: true,
      start() { return "paged"; },
      finish() { throw new Error("paged recorder must finish pages"); },
      progress() {},
      appendResultPage(_auditId, _pageIndex, result) { pages.push(result); },
      finishResultPages() {},
      interrupt() {},
    };
    const { catalog, enforcer } = await fixture(root, { audits: recorder }, { maxVisitedNodes: 1 });
    try {
      await enforcer.openWorkspace("paged-bounded-audit");
      await enforcer.audit("paged-bounded-audit", { scope: "repository", mode: "log" });
      expect(pages.flatMap((page) => page.failures ?? [])).toEqual([
        ...Array.from({ length: 20 }, () => expect.objectContaining({
          code: "budget-exceeded",
          stage: "evaluate",
        })),
        expect.objectContaining({
          clauseId: "scan",
          code: "budget-exceeded",
          stage: "scan",
        }),
      ]);
    } finally {
      await enforcer.close();
      catalog.close();
    }
  });

  it("records touched-files audits without delivery in log mode", async () => {
    const root = await rootDirectory();
    await writeFile(join(root, "api", "existing.py"), "print('existing')\n");
    const { catalog, deliveries, enforcer } = await fixture(root);
    try {
      await enforcer.openWorkspace("diagnostic");
      await enforcer.audit("diagnostic", { scope: "touched-files", mode: "log", paths: ["api/existing.py"] });
      expect(deliveries).toEqual([]);
    } finally {
      await enforcer.close();
      catalog.close();
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
    const { catalog, enforcer } = await fixture(root, { scope: "repository", audits: recorder }, {}, componentStoryCodeRule);
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
      catalog.close();
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
    const { catalog, enforcer } = await fixture(root, { scope: "repository", audits: recorder }, {}, asyncBlockingCodeRule);
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
      catalog.close();
    }
  });
});
