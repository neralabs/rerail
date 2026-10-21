import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import type { AuditInput, AuditRecorder, AuditResult } from "../../../../src/engine/audit/audit.js";
import { bundledCodeRuleDslDirectory } from "../../../../src/engine/code-rule/code-rule-compile.js";
import { NativeCodeRuleEnforcer } from "../../../../src/engine/code-rule/code-rule-enforcer.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import { repositoryPath } from "../../../support/oracles.js";
import { testCodeRuleCatalog } from "../../../support/repository-rules.js";

// The JavaScript and TypeScript packs split ECMA files by type (their READMEs): a core ESLint port that
// typescript-eslint replaces runs on JavaScript files only, a typescript-eslint port on TypeScript files only.

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

async function directory(prefix: string): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), prefix)));
  roots.push(root);
  return root;
}

/**
 * The packs' own documents: ESLint's no-redeclare, default-param-last and max-params with their typescript-eslint twins,
 * and no-require-imports.
 */
const CODE_RULES = await Promise.all(
  [
    ["javascript", "candidates", "eslint-no-redeclare.json"],
    ["typescript", "candidates", "typescript-eslint-no-redeclare.json"],
    ["typescript", "candidates", "typescript-eslint-no-require-imports.json"],
    ["javascript", "candidates", "eslint-default-param-last.json"],
    ["typescript", "", "typescript-eslint-default-param-last.json"],
    ["javascript", "candidates", "eslint-max-params.json"],
    ["typescript", "candidates", "typescript-eslint-max-params.json"],
  ].map(async ([pack, tier, file]) => codeRuleSchema.parse(JSON.parse(await readFile(repositoryPath("packs", pack!, tier!, "rules", file!), "utf8")))),
);

/**
 * TS overloads redeclare nothing. A value sharing an interface's name is no redeclaration to TypeScript, but
 * typescript-eslint reports it: its declaration-merge exemption covers no type/value pair (its own fixture
 * no-redeclare/019.ts, a type alias and a const, is reported).
 * CommonJS files may `require`.
 * A JavaScript file's parameters are JavaScript's grammar (`assignment_pattern`, plain identifiers), not TypeScript's
 * (`required_parameter`): the core ports read them there, the typescript-eslint twins read TypeScript's.
 */
const FILES = {
  "src/queries.ts": "export function f(a: string): void;\nexport function f(a: number): void;\nexport function f(a: unknown) {}\n\nexport interface Shape { a: number }\nexport const Shape = { a: 1 };\n",
  "src/plain.js": "var x = 1;\nvar x = 2;\n",
  "src/a.ts": "const x = require('x');\nexport default x;\n",
  "jest.config.js": "const path = require(\"node:path\");\nmodule.exports = { rootDir: path.resolve(__dirname) };\n",
  "tool.cjs": "const path = require(\"node:path\");\nmodule.exports = path.sep;\n",
  "src/params.js": "export function f(a = 1, b) {}\nexport function g(a, b, c, d) {}\n",
  "src/params.ts": "export function f(a = 1, b: number) {}\nexport function g(a: number, b: number, c: number, d: number) {}\n",
};

async function repository(git: boolean): Promise<string> {
  const root = await directory("rerail-ecma-scope-");
  for (const [path, text] of Object.entries(FILES)) {
    await mkdir(dirname(join(root, path)), { recursive: true });
    await writeFile(join(root, path), text);
  }
  if (git) {
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    execFileSync("git", ["add", "--force", "."], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
  }
  return root;
}

/** One log-only repository audit of `workspaceRoot`: each file's violations as `rule/clause:line`, and the failures. */
async function audit(workspaceRoot: string) {
  const home = await directory("rerail-ecma-scope-catalog-");
  const capabilitiesPath = join(home, "capabilities.json");
  await writeFile(capabilitiesPath, await readFile(join(bundledCodeRuleDslDirectory(), "capabilities.json")));
  const audits = new Map<string, AuditInput & { result?: AuditResult }>();
  const recorder: AuditRecorder = {
    start(input) { const auditId = `run-${audits.size + 1}`; audits.set(auditId, { ...input }); return auditId; },
    finish(auditId, result) { audits.get(auditId)!.result = result; },
    progress(auditId, result) { audits.get(auditId)!.result = { status: "completed", ...result }; },
    interrupt() {},
  };
  const enforcer = new NativeCodeRuleEnforcer(testCodeRuleCatalog(CODE_RULES, capabilitiesPath), {
    workspaceRoot, steerDelivery: { requestSteer: async () => undefined }, audits: recorder,
  });
  try {
    await enforcer.openWorkspace("ecma");
    const [auditId] = await enforcer.audit("ecma", { scope: "repository", mode: "log" });
    const result = audits.get(auditId!)?.result;
    const reported: Record<string, string[]> = {};
    for (const violation of result?.violations ?? []) {
      (reported[violation.path ?? ""] ??= []).push(`${violation.ruleId}/${violation.clauseId}:${(violation.span?.startLine ?? -1) + 1}`);
    }
    for (const path of Object.keys(reported)) reported[path]!.sort();
    return { status: result?.status, reported, failures: (result?.failures ?? []).map(failure => `${failure.path ?? ""} ${failure.code}`) };
  } finally {
    await enforcer.close();
  }
}

describe("the JavaScript and TypeScript packs on one repository", () => {
  it.each([
    ["the native repository index (Git checkout)", true],
    ["the host's directory walk", false],
  ])("report each file only through the port for its type, through %s", async (_name, git) => {
    expect(await audit(await repository(git))).toEqual({
      status: "completed",
      failures: [],
      reported: {
        "src/plain.js": ["eslint-no-redeclare/redeclared:2"],
        "src/params.js": ["eslint-default-param-last/default-param-not-last:1", "eslint-max-params/function:2"],
        "src/params.ts": ["typescript-eslint-default-param-last/default-param-not-last:1", "typescript-eslint-max-params/function-declaration:2"],
        "src/queries.ts": ["typescript-eslint-no-redeclare/redeclared:6"],
        "src/a.ts": ["typescript-eslint-no-require-imports/require-call:1"],
      },
    });
  });
});
