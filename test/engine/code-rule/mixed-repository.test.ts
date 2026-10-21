import { execFileSync } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { NativeCodeRuleEnforcer } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import type { AuditRecorder, AuditInput, AuditResult } from "../../../src/engine/audit/audit.js";
import { bundledCodeRuleDslDirectory } from "../../../src/engine/code-rule/code-rule-compile.js";
import { codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { fixturesPath } from "../../support/oracles.js";
import { testCodeRuleCatalog } from "../../support/repository-rules.js";

// T6 of the language-pack standard (docs/design/rerail-language-conformance.md): Python, TypeScript, PHP and Blade
// in one repository, through the built addon and the TypeScript host, by Git index and by directory walk.

const FIXTURE = fixturesPath("host", "mixed-repository");
const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

async function directory(prefix: string): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), prefix)));
  roots.push(root);
  return root;
}

const escapes = { stopBy: "end", kind: "function_call_expression", has: { field: "function", regex: "^htmlspecialchars$" } };
const clause = (id: string, language: string, rule: object, embedded?: string) => ({
  id, violation: { code: id.toUpperCase(), anchor: "witness.span", message: id }, forbid: { ast: { language, ...(embedded === undefined ? {} : { embedded }), rule } },
});
/** One code rule per language, each scoped to every file: a code rule reporting on another family's file would leak. */
const CODE_RULES = [
  { id: "mixed:python", clauses: [clause("os-system", "python", { kind: "attribute", resolves: { import: ["os.system"] } })] },
  { id: "mixed:typescript", clauses: [clause("console-log", "typescript", { pattern: "console.log($ARG)" })] },
  { id: "mixed:php", clauses: [clause("reaches-escape", "php", { kind: "function_call_expression", calls: { has: escapes } })] },
  { id: "mixed:blade", clauses: [
    clause("reaches-escape", "blade", { kind: "function_call_expression", calls: { has: escapes } }, "compiled-view"),
    clause("raw-echo", "blade", { kind: "raw_echo" }),
  ] },
].map(({ id, clauses }) => codeRuleSchema.parse({ format: "rerail.policy/v1", id, revision: 1, mode: "steer", scope: { include: ["**/*"], exclude: [] }, clauses }));

/** What each file reports, as `rule/clause`: the fixture's escapes reach PHP's htmlspecialchars from PHP and Blade alike. */
const EXPECTED = {
  "api/app/service.py": ["mixed:python/os-system"],
  "app/Http/Controller.php": ["mixed:php/reaches-escape"],
  "resources/views/page.blade.php": ["mixed:blade/raw-echo", "mixed:blade/reaches-escape"],
  "web/src/View.tsx": ["mixed:typescript/console-log"],
  "web/src/app.ts": ["mixed:typescript/console-log"],
};

async function enforcerFor(workspaceRoot: string) {
  const home = await directory("rerail-mixed-catalog-");
  const capabilitiesPath = join(home, "capabilities.json");
  await writeFile(capabilitiesPath, await readFile(join(bundledCodeRuleDslDirectory(), "capabilities.json")));
  const catalog = testCodeRuleCatalog(CODE_RULES, capabilitiesPath);
  const audits = new Map<string, AuditInput & { result?: AuditResult }>();
  const recorder: AuditRecorder = {
    start(input) { const auditId = `run-${audits.size + 1}`; audits.set(auditId, { ...input }); return auditId; },
    finish(auditId, result) { audits.get(auditId)!.result = result; },
    progress(auditId, result) { audits.get(auditId)!.result = { status: "completed", ...result }; },
    interrupt() {},
  };
  const enforcer = new NativeCodeRuleEnforcer(catalog, { workspaceRoot, steerDelivery: { requestSteer: async () => undefined }, audits: recorder });
  await enforcer.openWorkspace("mixed");
  /** One log-only repository audit: each file's `rule/clause` violations, the IDs of all violations, and the failures. */
  const audit = async () => {
    const [auditId] = await enforcer.audit("mixed", { scope: "repository", mode: "log" });
    const result = audits.get(auditId!)?.result;
    const reported: Record<string, string[]> = {};
    for (const violation of result?.violations ?? []) (reported[violation.path ?? ""] ??= []).push(`${violation.ruleId}/${violation.clauseId}`);
    for (const path of Object.keys(reported)) reported[path]!.sort();
    return {
      status: result?.status, reported,
      ids: Object.fromEntries((result?.violations ?? []).map(violation => [`${violation.path}:${violation.clauseId}:${violation.span?.startByte}`, violation.id])),
      failures: (result?.failures ?? []).map(failure => `${failure.path ?? ""} ${failure.code}`),
    };
  };
  return { enforcer, audit, close: async () => { await enforcer.close(); } };
}

async function mixedRepository(git: boolean): Promise<string> {
  const root = await directory("rerail-mixed-");
  await cp(FIXTURE, root, { recursive: true });
  // A dependency under vendor/ calls the helper too; the host never evaluates it. (Rerail keeps no vendor/ directory.)
  await mkdir(join(root, "vendor", "acme"), { recursive: true });
  await writeFile(join(root, "vendor", "acme", "lib.php"), "<?php\n\nfunction acme(string $value): string\n{\n    return escape_html($value);\n}\n");
  if (git) {
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    execFileSync("git", ["add", "--force", "."], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
  }
  return root;
}

describe("a repository mixing Python, TypeScript, PHP and Blade", () => {
  it.each([
    ["the native repository index (Git checkout)", true],
    ["the host's directory walk", false],
  ])("reports each language's violations on its own files only, through %s, and follows a PHP edit across languages", async (_name, git) => {
    const root = await mixedRepository(git);
    const { enforcer, audit, close } = await enforcerFor(root);
    try {
      const before = await audit();
      // vendor/ calls the helper too, but the host never evaluates it.
      expect({ status: before.status, reported: before.reported, failures: before.failures }).toEqual({ status: "completed", reported: EXPECTED, failures: [] });

      // Renaming the PHP helper changes what the PHP and Blade callers in other files reach, and nothing else.
      const helpers = "<?php\n\nfunction escape_html_renamed(string $value): string\n{\n    return htmlspecialchars($value, ENT_QUOTES);\n}\n";
      await writeFile(join(root, "app/Support/helpers.php"), helpers);
      await enforcer.auditGeneratedEdits("mixed", {
        revision: 1, checkpoints: ["edit"], edits: [{ path: "app/Support/helpers.php", language: "php", generatedText: helpers }],
      });
      await enforcer.settle();
      const after = await audit();
      const { "app/Http/Controller.php": _controller, ...rest } = EXPECTED;
      expect({ status: after.status, reported: after.reported, failures: after.failures }).toEqual({
        status: "completed", failures: [],
        reported: { ...rest, "resources/views/page.blade.php": ["mixed:blade/raw-echo"] },
      });
      // The other languages' violations are the same violations, not re-derived ones.
      for (const [key, id] of Object.entries(after.ids)) expect(before.ids[key]).toBe(id);
    } finally {
      await close();
    }
  });
});
