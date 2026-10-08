import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CodeRuleBridge, generatedEditPaths } from "../src/code-rule-bridge.js";
import type { CodeRuleEditBatch, CodeRuleAuditor } from "../src/code-rule.js";
import { languageFor, NativeCodeRuleEnforcer } from "../src/code-rule-enforcer.js";
import type { AuditRecorder, AuditInput, AuditResult } from "../src/audit.js";
import { bundledCodeRuleDslDirectory } from "../src/code-rule-dsl-catalog.js";
import { codeRuleDialectForPath } from "../src/code-rule-languages.js";
import { codeRuleSchema } from "../src/code-rule-schema.js";
import { testCodeRuleCatalog } from "./support/repository-rules.js";

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

async function directory(prefix: string): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), prefix)));
  roots.push(root);
  return root;
}

/**
 * Every PHP and Blade routing case: the dialect the path's file is evaluated as, or undefined when the host evaluates
 * no file there. `vendor/` paths route by name like any other; the walks never reach them (see the audit cases).
 */
const ROUTES: ReadonlyArray<readonly [string, "php" | "blade" | undefined]> = [
  ["Kernel.php", "php"],
  ["app/Http/Controllers/Admin/UserController.php", "php"],
  ["app/Legacy/UPPER.PHP", "php"],
  ["app/Legacy/Mixed.Php", "php"],
  ["resources/views/page.blade.php", "blade"],
  ["resources/views/layouts/deep/nested/Shell.Blade.PHP", "blade"],
  ["resources/views/archive.v1.blade.php", "blade"],
  ["resources/blade.php", "php"],
  ["config/.blade.php", "php"],
  [".php", undefined],
  ["config/.php", undefined],
  ["resources/views/x.blade.php.bak", undefined],
  ["resources/views/page.phtml", undefined],
  ["resources/views/page.blade", undefined],
  ["src/module.php5", undefined],
];

/** The files an audit must leave alone: ignored directories at any depth, and paths no dialect reads. */
const IGNORED_FILES = [
  "vendor/acme/lib.php",
  "vendor/acme/resources/views/x.blade.php",
  "packages/tool/vendor/nested.php",
  "node_modules/pkg/index.php",
];

/** A PHP program both dialects read: `$routed` is found by a php query on a php file and a blade query on a blade file. */
const SOURCE = "<?php $routed = 1; ?>\n";

function routedCodeRule(language: "php" | "blade") {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1", id: `routing:${language}`, revision: 1, mode: "steer",
    scope: { include: ["**/*"], exclude: [] },
    clauses: [{
      id: "routed",
      violation: { code: "ROUTED", anchor: "witness.span", message: `routed as ${language}` },
      forbid: { ast: { language, rule: { kind: "variable_name", regex: "^\\$routed$" } } },
    }],
  });
}

/** One log-only repository audit: the code rules reporting on each path (`routing:php`, `routing:blade`), and its failures. */
async function auditRouting(workspaceRoot: string) {
  const home = await directory("rerail-routing-catalog-");
  const capabilitiesPath = join(home, "capabilities.json");
  await writeFile(capabilitiesPath, await readFile(join(bundledCodeRuleDslDirectory(), "capabilities.json")));
  const catalog = testCodeRuleCatalog((["php", "blade"] as const).map(language => routedCodeRule(language)), capabilitiesPath);
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
  const root = await directory("rerail-routing-");
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

describe("PHP and Blade host routing", () => {
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
    const expected = Object.fromEntries(ROUTES.flatMap(([path, dialect]) => dialect === undefined ? [] : [[path, [`routing:${dialect}`]]]));
    await expect(auditRouting(root)).resolves.toEqual({ status: "completed", routed: expected, failures: [] });
  });

  it("labels the files a command writes by dialect, skipping ignored directories and other names", async () => {
    const root = await directory("rerail-routing-bridge-");
    await mkdir(join(root, "app"));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => ({
      revision: batch.revision, disposition: "continue" as const, violations: [], failures: [],
    }));
    const auditor: CodeRuleAuditor = {
      openWorkspace: vi.fn(async () => undefined), acceptsPath: () => true, auditGeneratedEdits, close: vi.fn(async () => undefined),
    };
    const bridge = new CodeRuleBridge(root, auditor);
    try {
      await bridge.openWorkspace("session-1");
      await bridge.begin("shell-1", "shell", { command: "php artisan make:view" });
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
