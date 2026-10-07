import { execFileSync } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { NativeExecutablePolicyEnforcer } from "../src/executable-policy-enforcer.js";
import type { PolicyExecutionRecorder, PolicyRunInput, PolicyRunResult } from "../src/policy-execution.js";
import { bootstrapPolicyDslCatalog, bundledPolicyDslDirectory } from "../src/policy-dsl-catalog.js";
import { policyRuleSchema } from "../src/policy-schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";

// T6 of the language-pack standard (docs/design/rerail-language-conformance.md): Python, TypeScript, PHP and Blade
// in one repository, through the built addon and the TypeScript host, by Git index and by directory walk.

const FIXTURE = fileURLToPath(new URL("./fixtures/mixed-repository", import.meta.url));
const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

async function directory(prefix: string): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), prefix)));
  roots.push(root);
  return root;
}

const escapes = { stopBy: "end", kind: "function_call_expression", has: { field: "function", regex: "^htmlspecialchars$" } };
const check = (id: string, language: string, rule: object) => ({
  id, violation: { code: id.toUpperCase(), anchor: "witness.span", message: id }, forbid: { ast: { language, rule } },
});
/** One policy per language, each scoped to every file: a policy reporting on another family's file would leak. */
const POLICIES = [
  { id: "mixed:python", clauses: [check("os-system", "python", { kind: "attribute", resolves: { import: ["os.system"] } })] },
  { id: "mixed:typescript", clauses: [check("console-log", "typescript", { pattern: "console.log($ARG)" })] },
  { id: "mixed:php", clauses: [check("reaches-escape", "php", { kind: "function_call_expression", calls: { has: escapes } })] },
  { id: "mixed:blade", clauses: [
    check("reaches-escape", "blade", { kind: "function_call_expression", calls: { has: escapes } }),
    check("raw-echo", "blade", { kind: "php_statement", regex: "^\\{!!" }),
  ] },
].map(({ id, clauses }) => policyRuleSchema.parse({ format: "rerail.policy/v1", id, revision: 1, mode: "steer", scope: { include: ["**/*"], exclude: [] }, clauses }));

/** What each file reports, as `policy/check`: the fixture's escapes reach PHP's htmlspecialchars from PHP and Blade alike. */
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
  await writeFile(capabilitiesPath, await readFile(join(bundledPolicyDslDirectory(), "capabilities.json")));
  const databasePath = join(home, "policies.sqlite");
  const store = new SqliteRuleStore(databasePath);
  for (const policy of POLICIES) store.upsertPolicy(policy);
  store.close();
  const catalog = bootstrapPolicyDslCatalog({ capabilitiesPath, databasePath });
  const runs = new Map<string, PolicyRunInput & { result?: PolicyRunResult }>();
  const executions: PolicyExecutionRecorder = {
    start(input) { const runId = `run-${runs.size + 1}`; runs.set(runId, { ...input }); return runId; },
    finish(runId, result) { runs.get(runId)!.result = result; },
    progress(runId, result) { runs.get(runId)!.result = { status: "completed", ...result }; },
    interrupt() {},
  };
  const enforcer = new NativeExecutablePolicyEnforcer(catalog, { workspaceRoot, delivery: { requestDelivery: async () => undefined }, executions });
  await enforcer.openWorkspace("mixed");
  /** One log-only repository scan: each file's `policy/check` findings, the IDs of all findings, and the failures. */
  const scan = async () => {
    const [runId] = await enforcer.scan("mixed", { scope: "repository", mode: "log" });
    const result = runs.get(runId!)?.result;
    const reported: Record<string, string[]> = {};
    for (const finding of result?.violations ?? []) (reported[finding.path ?? ""] ??= []).push(`${finding.ruleId}/${finding.clauseId}`);
    for (const path of Object.keys(reported)) reported[path]!.sort();
    return {
      status: result?.status, reported,
      ids: Object.fromEntries((result?.violations ?? []).map(finding => [`${finding.path}:${finding.clauseId}:${finding.span?.startByte}`, finding.id])),
      failures: (result?.failures ?? []).map(failure => `${failure.path ?? ""} ${failure.code}`),
    };
  };
  return { enforcer, scan, close: async () => { await enforcer.close(); catalog.close(); } };
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
  ])("reports each language's findings on its own files only, through %s, and follows a PHP edit across languages", async (_name, git) => {
    const root = await mixedRepository(git);
    const { enforcer, scan, close } = await enforcerFor(root);
    try {
      const before = await scan();
      // vendor/ calls the helper too, but the host never evaluates it.
      expect({ status: before.status, reported: before.reported, failures: before.failures }).toEqual({ status: "completed", reported: EXPECTED, failures: [] });

      // Renaming the PHP helper changes what the PHP and Blade callers in other files reach, and nothing else.
      const helpers = "<?php\n\nfunction escape_html_renamed(string $value): string\n{\n    return htmlspecialchars($value, ENT_QUOTES);\n}\n";
      await writeFile(join(root, "app/Support/helpers.php"), helpers);
      await enforcer.evaluateGeneratedEdits("mixed", {
        revision: 1, checkpoints: ["edit"], edits: [{ path: "app/Support/helpers.php", language: "php", generatedText: helpers }],
      });
      await enforcer.settle();
      const after = await scan();
      const { "app/Http/Controller.php": _controller, ...rest } = EXPECTED;
      expect({ status: after.status, reported: after.reported, failures: after.failures }).toEqual({
        status: "completed", failures: [],
        reported: { ...rest, "resources/views/page.blade.php": ["mixed:blade/raw-echo"] },
      });
      // The other languages' findings are the same findings, not re-derived ones.
      for (const [key, id] of Object.entries(after.ids)) expect(before.ids[key]).toBe(id);
    } finally {
      await close();
    }
  });
});
