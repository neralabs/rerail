import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CodeRuleBridge, generatedEditPaths } from "../../../../src/engine/code-rule/code-rule-bridge.js";
import type { CodeRuleEditBatch, CodeRuleAuditor } from "../../../../src/engine/code-rule/code-rule.js";
import { languageFor, NativeCodeRuleEnforcer } from "../../../../src/engine/code-rule/code-rule-enforcer.js";
import type { AuditRecorder, AuditInput, AuditResult } from "../../../../src/engine/audit/audit.js";
import { bundledCodeRuleDslDirectory } from "../../../../src/engine/code-rule/code-rule-compile.js";
import { codeRuleDialectForPath } from "../../../../src/engine/code-rule/code-rule-languages.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import { testCodeRuleCatalog } from "../../../support/repository-rules.js";

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

async function directory(prefix: string): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), prefix)));
  roots.push(root);
  return root;
}

type EcmaDialect = "typescript" | "tsx" | "javascript";

/** Every TypeScript, TSX and JavaScript routing case: the dialect the path's file is evaluated as, or undefined. */
const ROUTES: ReadonlyArray<readonly [string, EcmaDialect | undefined]> = [
  ["index.ts", "typescript"],
  ["src/deep/nested/module/service.ts", "typescript"],
  ["src/types/global.d.ts", "typescript"],
  ["src/esm/entry.mts", "typescript"],
  ["src/cjs/entry.cts", "typescript"],
  ["src/Legacy/UPPER.TS", "typescript"],
  ["src/components/View.tsx", "tsx"],
  ["src/components/Mixed.Tsx", "tsx"],
  ["scripts/build.js", "javascript"],
  ["scripts/Widget.jsx", "javascript"],
  ["scripts/esm.mjs", "javascript"],
  ["scripts/config.cjs", "javascript"],
  ["scripts/OLD.JS", "javascript"],
  ["src/.d.ts", "typescript"],
  [".ts", undefined],
  ["src/.tsx", undefined],
  ["scripts/.js", undefined],
  ["src/view.ts.bak", undefined],
  ["src/view.coffee", undefined],
  ["src/old.es6", undefined],
];

/** The files an audit must leave alone: the host's ignored directories, at any depth. */
const IGNORED_FILES = [
  "node_modules/pkg/index.js",
  "packages/app/node_modules/dep/index.ts",
  "dist/bundle.js",
  "packages/app/build/out.ts",
  "coverage/lcov-report/prettify.js",
  ".next/server/page.js",
  "vendor/lib/x.ts",
];

/** A program every Ecma dialect reads alike. */
const SOURCE = "const routed = 1;\n";

/** `typescript` queries read every Ecma dialect; `javascript` ones only JavaScript files. */
function routedCodeRule(language: "typescript" | "javascript") {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1", id: `routing:${language}`, revision: 1, mode: "steer",
    scope: { include: ["**/*"], exclude: [] },
    clauses: [{
      id: "routed",
      violation: { code: "ROUTED", anchor: "witness.span", message: `routed as ${language}` },
      forbid: { ast: { language, rule: { kind: "identifier", regex: "^routed$" } } },
    }],
  });
}

/** One log-only repository audit: the code rules reporting on each path, and its failures. */
async function auditRouting(workspaceRoot: string) {
  const home = await directory("rerail-ts-routing-catalog-");
  const capabilitiesPath = join(home, "capabilities.json");
  await writeFile(capabilitiesPath, await readFile(join(bundledCodeRuleDslDirectory(), "capabilities.json")));
  const catalog = testCodeRuleCatalog((["typescript", "javascript"] as const).map(language => routedCodeRule(language)), capabilitiesPath);
  const audits = new Map<string, AuditInput & { result?: AuditResult }>();
  const recorder: AuditRecorder = {
    start(input) { const auditId = `run-${audits.size + 1}`; audits.set(auditId, { ...input }); return auditId; },
    finish(auditId, result) { audits.get(auditId)!.result = result; },
    progress(auditId, result) { audits.get(auditId)!.result = { status: "completed", ...result }; },
    interrupt() {},
  };
  const enforcer = new NativeCodeRuleEnforcer(catalog, { workspaceRoot, steerDelivery: { requestSteer: async () => undefined }, audits: recorder });
  try {
    await enforcer.openWorkspace("routing");
    const [auditId] = await enforcer.audit("routing", { scope: "repository", mode: "log" });
    const result = audits.get(auditId!)?.result;
    const routed: Record<string, string[]> = {};
    for (const violation of result?.violations ?? []) (routed[violation.path ?? ""] ??= []).push(violation.ruleId);
    for (const path of Object.keys(routed)) routed[path]!.sort();
    return { status: result?.status, routed, failures: (result?.failures ?? []).map(failure => `${failure.path ?? ""} ${failure.code}`) };
  } finally {
    await enforcer.close();
  }
}

async function routingTree(git: boolean): Promise<string> {
  const root = await directory("rerail-ts-routing-");
  for (const path of [...ROUTES.map(([path]) => path), ...IGNORED_FILES]) {
    await mkdir(dirname(join(root, path)), { recursive: true });
    await writeFile(join(root, path), SOURCE);
  }
  if (git) {
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    execFileSync("git", ["add", "--force", "."], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
  }
  return root;
}

describe("TypeScript, TSX and JavaScript host routing", () => {
  it.each(ROUTES)("routes %s to %s through every host table", (path, dialect) => {
    expect(languageFor(path)).toBe(dialect);
    expect(codeRuleDialectForPath(path)).toBe(dialect);
    expect(generatedEditPaths("write", { path })).toEqual(dialect === undefined ? [] : [path]);
  });

  it.each([
    ["the native repository index (Git checkout)", true],
    ["the host's directory walk", false],
  ])("audits each file as its dialect through %s, never under ignored directories", async (_name, git) => {
    const root = await routingTree(git);
    const expected = Object.fromEntries(
      ROUTES.flatMap(([path, dialect]) =>
        dialect === undefined ? [] : [[path, dialect === "javascript" ? ["routing:javascript", "routing:typescript"] : ["routing:typescript"]]],
      ),
    );
    await expect(auditRouting(root)).resolves.toEqual({ status: "completed", routed: expected, failures: [] });
  });

  it("labels the files a command writes by dialect, skipping ignored directories and other names", async () => {
    const root = await directory("rerail-ts-routing-bridge-");
    await mkdir(join(root, "src"));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => ({
      revision: batch.revision, disposition: "continue" as const, violations: [], failures: [],
    }));
    const auditor: CodeRuleAuditor = {
      openWorkspace: vi.fn(async () => undefined), acceptsPath: () => true, auditGeneratedEdits, close: vi.fn(async () => undefined),
    };
    const bridge = new CodeRuleBridge(root, auditor);
    try {
      await bridge.openWorkspace("session-1");
      await bridge.begin("shell-1", "shell", { command: "npx tsc --build" });
      for (const path of [...ROUTES.map(([path]) => path), ...IGNORED_FILES]) {
        await mkdir(dirname(join(root, path)), { recursive: true });
        await writeFile(join(root, path), SOURCE);
      }
      await bridge.finish("shell-1");
      const edits = auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits.map(edit => [edit.path, edit.language]));
      expect(Object.fromEntries(edits)).toEqual(Object.fromEntries(ROUTES.filter(([, dialect]) => dialect !== undefined)));
    } finally {
      await bridge.close();
    }
  });
});
