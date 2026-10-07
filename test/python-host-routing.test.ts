import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ExecutablePolicyBridge, generatedEditPaths } from "../src/executable-policy-bridge.js";
import type { ExecutablePolicyEditBatch, ExecutablePolicyEvaluator } from "../src/executable-policy.js";
import { languageFor, NativeExecutablePolicyEnforcer } from "../src/executable-policy-enforcer.js";
import type { PolicyExecutionRecorder, PolicyRunInput, PolicyRunResult } from "../src/policy-execution.js";
import { bootstrapPolicyDslCatalog, bundledPolicyDslDirectory } from "../src/policy-dsl-catalog.js";
import { policyDialectForPath } from "../src/policy-languages.js";
import { policyRuleSchema } from "../src/policy-schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

async function directory(prefix: string): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), prefix)));
  roots.push(root);
  return root;
}

/**
 * Every Python routing case (the conformance standard's T5 routing tier): the dialect the path's file is evaluated as,
 * or undefined when the host evaluates no file there. Stubs (`.pyi`) are Python; the suffix is matched without regard
 * to case; a bare suffix, a compiled or Windows-script suffix and a backup name are not source.
 */
const ROUTES: ReadonlyArray<readonly [string, "python" | undefined]> = [
  ["main.py", "python"],
  ["api/app/services/deep/module.py", "python"],
  ["pkg/__init__.py", "python"],
  ["stubs/typing_ext.pyi", "python"],
  ["legacy/UPPER.PY", "python"],
  ["legacy/Mixed.Py", "python"],
  ["stubs/Types.PYI", "python"],
  [".py", undefined],
  ["pkg/.pyi", undefined],
  ["tools/script.pyw", undefined],
  ["pkg/module.pyc", undefined],
  ["notes/draft.py.bak", undefined],
  ["notebooks/analysis.ipynb", undefined],
];

/** The files a scan must leave alone: Python files under ignored directories, at the root and nested. */
const IGNORED_FILES = [
  ".venv/lib/python3.13/site-packages/pkg/core.py",
  "pkg/__pycache__/module.py",
  "services/api/.venv/bin/activate_this.py",
  "node_modules/pkg/gyp/pylib.py",
  "build/lib/pkg/module.py",
];

/** A program in which a `python` query finds `routed`. */
const SOURCE = "routed = 1\n";

function routedPolicy() {
  return policyRuleSchema.parse({
    format: "rerail.policy/v1", id: "routing:python", revision: 1, mode: "steer",
    scope: { include: ["**/*"], exclude: [] },
    clauses: [{
      id: "routed",
      violation: { code: "ROUTED", anchor: "witness.span", message: "routed as python" },
      forbid: { ast: { language: "python", rule: { kind: "identifier", regex: "^routed$" } } },
    }],
  });
}

/** One log-only repository scan: the policies reporting on each path, and its failures. */
async function scanRouting(workspaceRoot: string) {
  const home = await directory("rerail-py-routing-catalog-");
  const capabilitiesPath = join(home, "capabilities.json");
  await writeFile(capabilitiesPath, await readFile(join(bundledPolicyDslDirectory(), "capabilities.json")));
  const databasePath = join(home, "policies.sqlite");
  const store = new SqliteRuleStore(databasePath);
  store.upsertPolicy(routedPolicy());
  store.close();
  const catalog = bootstrapPolicyDslCatalog({ capabilitiesPath, databasePath });
  const runs = new Map<string, PolicyRunInput & { result?: PolicyRunResult }>();
  const recorder: PolicyExecutionRecorder = {
    start(input) { const runId = `run-${runs.size + 1}`; runs.set(runId, { ...input }); return runId; },
    finish(runId, result) { runs.get(runId)!.result = result; },
    progress(runId, result) { runs.get(runId)!.result = { status: "completed", ...result }; },
    interrupt() {},
  };
  const enforcer = new NativeExecutablePolicyEnforcer(catalog, { workspaceRoot, delivery: { requestDelivery: async () => undefined }, executions: recorder });
  try {
    await enforcer.openWorkspace("routing");
    const [runId] = await enforcer.scan("routing", { scope: "repository", mode: "log" });
    const result = runs.get(runId!)?.result;
    const routed: Record<string, string[]> = {};
    for (const finding of result?.violations ?? []) (routed[finding.path ?? ""] ??= []).push(finding.ruleId);
    return { status: result?.status, routed, failures: (result?.failures ?? []).map(failure => `${failure.path ?? ""} ${failure.code}`) };
  } finally {
    await enforcer.close();
    catalog.close();
  }
}

async function routingTree(git: boolean): Promise<string> {
  const root = await directory("rerail-py-routing-");
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

describe("Python host routing", () => {
  it.each(ROUTES)("routes %s to %s through every host table", (path, dialect) => {
    expect(languageFor(path)).toBe(dialect);
    expect(policyDialectForPath(path)).toBe(dialect);
    expect(generatedEditPaths("write", { path })).toEqual(dialect === undefined ? [] : [path]);
  });

  it.each([
    ["the native repository index (Git checkout)", true],
    ["the host's directory walk", false],
  ])("scans each file as Python through %s, never under ignored directories", async (_name, git) => {
    const root = await routingTree(git);
    const expected = Object.fromEntries(ROUTES.flatMap(([path, dialect]) => dialect === undefined ? [] : [[path, ["routing:python"]]]));
    await expect(scanRouting(root)).resolves.toEqual({ status: "completed", routed: expected, failures: [] });
  });

  it("labels the files a command writes as Python, skipping ignored directories and other names", async () => {
    const root = await directory("rerail-py-routing-bridge-");
    const evaluateGeneratedEdits = vi.fn(async (_sessionId: string, batch: ExecutablePolicyEditBatch) => ({
      revision: batch.revision, disposition: "continue" as const, violations: [], failures: [],
    }));
    const evaluator: ExecutablePolicyEvaluator = {
      openWorkspace: vi.fn(async () => undefined), acceptsPath: () => true, evaluateGeneratedEdits, close: vi.fn(async () => undefined),
    };
    const bridge = new ExecutablePolicyBridge(root, evaluator);
    try {
      await bridge.openWorkspace("session-1");
      await bridge.begin("shell-1", "shell", { command: "python -m pip install -e ." });
      for (const path of [...ROUTES.map(([path]) => path), ...IGNORED_FILES]) {
        await mkdir(dirname(join(root, path)), { recursive: true });
        await writeFile(join(root, path), SOURCE);
      }
      await bridge.finish("shell-1");
      const edits = evaluateGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits.map(edit => [edit.path, edit.language]));
      expect(Object.fromEntries(edits)).toEqual(Object.fromEntries(ROUTES.filter(([, dialect]) => dialect !== undefined)));
    } finally {
      await bridge.close();
    }
  });
});
