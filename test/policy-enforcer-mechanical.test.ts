import { execFileSync, fork } from "node:child_process";
import type * as ChildProcess from "node:child_process";
import Database from "better-sqlite3";
import { EventEmitter } from "node:events";
import { existsSync } from "node:fs";
import * as fsPromises from "node:fs/promises";
import type * as FsPromises from "node:fs/promises";
import { chmod, mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { setImmediate as queueHostTurn } from "node:timers";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { z } from "zod";
import type { PolicyExecutionRecorder, PolicyRunInput, PolicyRunResult } from "../src/policy-execution.js";
import { defaultRerailDatabasePath } from "../src/database.js";
import { SqlitePolicyExecutionStore } from "../src/policy-execution-store.js";
import type { PolicyDeliveryMetadata } from "../src/executable-policy.js";
import { NativeExecutablePolicyEnforcer, pythonModulePath, SOURCE_LANGUAGES } from "../src/executable-policy-enforcer.js";
import { bootstrapPolicyDslCatalog, bundledPolicyDslDirectory } from "../src/policy-dsl-catalog.js";
import { policyRuleSchema } from "../src/policy-schema.js";
import type { RuleMode } from "../src/rule-mode.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { rerailOraclesDirectory } from "./oracles.js";
import { gateStorageAttachments } from "./storage-attach-gate.js";
import * as native from "../src/native.js";
import type { NativeBinding, NativePolicyDslWorkspace, NativeRepositoryIndexOptions } from "../src/native.js";


vi.mock("node:fs/promises", { spy: true });
vi.mock("node:child_process", { spy: true });
vi.mock("node:fs", { spy: true });



const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

function needlePolicy(revision: number, pattern = "NEEDLE", code = "NEEDLE") {
  return policyRuleSchema.parse({
    format: "rerail.policy/v1", id: "test:needle", revision, mode: "block",
    scope: { include: ["api/**/*.py"], exclude: [] },
    clauses: [{ id: "needle", violation: { code, anchor: "witness.span", message: `No ${pattern}` }, forbid: { regex: { pattern, source: "file.content" } } }],
  });
}

function modePolicy(id: string, pattern: string, mode: RuleMode) {
  return policyRuleSchema.parse({
    format: "rerail.policy/v1", id, revision: 1, mode,
    scope: { include: ["api/**/*.py"], exclude: [] },
    clauses: [{ id: "match", violation: { code: pattern, anchor: "witness.span", message: `No ${pattern}` }, forbid: { regex: { pattern, source: "file.content" } } }],
  });
}

function recordingRuns(): { recorder: PolicyExecutionRecorder; runs: Map<string, PolicyRunInput & { result?: PolicyRunResult }> } {
  const runs = new Map<string, PolicyRunInput & { result?: PolicyRunResult }>();
  let sequence = 0;
  return {
    recorder: {
      start(input) {
        const runId = `run-${++sequence}`;
        runs.set(runId, { ...input, ruleRevisions: [...input.ruleRevisions], paths: [...input.paths] });
        return runId;
      },
      finish(runId, result) { runs.get(runId)!.result = result; },
      progress(runId, result) { runs.get(runId)!.result = { status: "completed", ...result }; },
      interrupt() {},
    },
    runs,
  };
}

function runForPath(runs: ReadonlyMap<string, PolicyRunInput & { result?: PolicyRunResult }>, path: string) {
  return [...runs.values()].find(run => run.paths.includes(path));
}


function consolePolicy(revision: number) {
  return policyRuleSchema.parse({
    format: "rerail.policy/v1", id: "test:console", revision, mode: "block",
    scope: { include: ["web/**/*.ts", "web/**/*.tsx"], exclude: [] },
    clauses: [{
      id: "console", violation: { code: "CONSOLE", anchor: "witness.span", message: "No console output" },
      forbid: { ast: { language: "typescript", rule: { pattern: "console.log($ARG)" } } },
    }],
  });
}

async function fixture({ policies = [needlePolicy(1)], ...options }: {
  scope?: "repository" | "touched-files" | "ai-output";
  mode?: "steer" | "log";
  executions?: PolicyExecutionRecorder;
  policies?: readonly ReturnType<typeof policyRuleSchema.parse>[];
} = {}) {
  const root = await mkdtemp(join(tmpdir(), "rerail-enforcer-"));
  roots.push(root);
  await mkdir(join(root, "api"));
  const capabilitiesPath = join(root, "capabilities.json");
  await writeFile(capabilitiesPath, await readFile(join(bundledPolicyDslDirectory(), "capabilities.json")));
  const databasePath = join(root, ".rerail", "policies.sqlite");
  const store = new SqliteRuleStore(databasePath);
  for (const policy of policies) store.upsertPolicy(policy);
  store.close();
  const catalog = bootstrapPolicyDslCatalog({ capabilitiesPath, databasePath });
  const requestDelivery = vi.fn(async (_sessionId: string, _delivery: "steer", _content: string, _metadata?: PolicyDeliveryMetadata) => undefined);
  const enforcer = new NativeExecutablePolicyEnforcer(catalog, {
    workspaceRoot: root,
    delivery: { requestDelivery },
    ...options,
  });
  return { root, catalog, enforcer, requestDelivery, databasePath };
}


async function settled(enforcer: NativeExecutablePolicyEnforcer) {
  await expect.poll(() => enforcer.snapshotStatus().pending, { timeout: 15_000, interval: 10 }).toBe(false);
  return enforcer.snapshotStatus();
}

const gitCommit = ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"];

/**
 * A tree whose `api/` sources Git, `.rerailignore` and Rerail's built-in ignores would each hide from a Git listing: a
 * tracked file under `tmp/`, a gitignored untracked file, a file ignored by a nested repository, a `.rerailignore`d
 * tracked file. `build/` is a host-ignored directory. `tmp/ws` and `out/ws` (gitignored) are nested workspace roots.
 */
async function writeScanTree(root: string, git: boolean): Promise<void> {
  const files: Record<string, string> = {
    ".gitignore": "api/gen/\nout/\n",
    ".rerailignore": "api/skip/\n",
    "api/a.py": "NEEDLE\n",
    "api/tmp/b.py": "NEEDLE\n",
    "api/gen/c.py": "NEEDLE\n",
    "api/nested/.gitignore": "e.py\n",
    "api/nested/d.py": "NEEDLE\n",
    "api/nested/e.py": "NEEDLE\n",
    "api/skip/f.py": "NEEDLE\n",
    "api/build/g.py": "NEEDLE\n",
    "tmp/ws/api/a.py": "NEEDLE\n",
    "out/ws/api/a.py": "NEEDLE\n",
  };
  for (const [path, source] of Object.entries(files)) {
    await mkdir(dirname(join(root, path)), { recursive: true });
    await writeFile(join(root, path), source);
  }
  if (!git) return;
  const nested = join(root, "api", "nested");
  execFileSync("git", ["init", "--quiet"], { cwd: nested });
  execFileSync("git", ["add", "."], { cwd: nested });
  execFileSync("git", gitCommit, { cwd: nested });
  execFileSync("git", ["init", "--quiet"], { cwd: root });
  execFileSync("git", ["add", "."], { cwd: root });
  execFileSync("git", gitCommit, { cwd: root });
}

/** One log-only repository scan of `workspaceRoot`, whose catalog lives elsewhere: its status, sorted finding paths and sorted `path code` failures. */
async function repositoryScan(workspaceRoot: string): Promise<{ status: string | undefined; paths: string[]; failures: string[] }> {
  const home = await mkdtemp(join(tmpdir(), "rerail-scan-catalog-"));
  roots.push(home);
  const capabilitiesPath = join(home, "capabilities.json");
  await writeFile(capabilitiesPath, await readFile(join(bundledPolicyDslDirectory(), "capabilities.json")));
  const databasePath = join(home, "policies.sqlite");
  const store = new SqliteRuleStore(databasePath);
  store.upsertPolicy(needlePolicy(1));
  store.close();
  const catalog = bootstrapPolicyDslCatalog({ capabilitiesPath, databasePath });
  const { recorder, runs } = recordingRuns();
  const enforcer = new NativeExecutablePolicyEnforcer(catalog, {
    workspaceRoot,
    delivery: { requestDelivery: async () => undefined },
    executions: recorder,
  });
  try {
    await enforcer.openWorkspace("scan-parity");
    const [runId] = await enforcer.scan("scan-parity", { scope: "repository", mode: "log" });
    const result = runs.get(runId!)?.result;
    return {
      status: result?.status,
      paths: (result?.violations ?? []).map(finding => finding.path ?? "").toSorted(),
      failures: (result?.failures ?? []).map(failure => `${failure.path ?? ""} ${failure.code}`).toSorted(),
    };
  } finally {
    await enforcer.close();
    catalog.close();
  }
}

/** The sorted finding paths of one repository scan of `workspaceRoot` that must complete without failures. */
async function repositoryScanPaths(workspaceRoot: string): Promise<string[]> {
  const result = await repositoryScan(workspaceRoot);
  expect(result).toMatchObject({ status: "completed", failures: [] });
  return result.paths;
}

/** Make `directory` (created with one scoped source) unlistable; returns the undo `afterEach` cleanup needs. */
async function unreadableDirectory(directory: string): Promise<() => Promise<void>> {
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, "x.py"), "NEEDLE\n");
  await chmod(directory, 0o000);
  return () => chmod(directory, 0o755);
}

/** Clean directories the shared content store records as complete. */
function completedDirectories(): number {
  const store = new Database(process.env.RERAIL_CONTENT_STORE_PATH!, { readonly: true });
  try {
    const count: unknown = store.prepare("SELECT count(*) FROM completed_directory_reuse").pluck().get();
    return typeof count === "number" ? count : Number.NaN;
  } finally { store.close(); }
}

/** Another process that takes the content store's write lock (`lock`) and gives it back (`release`) on request. */
interface StoreWriterHolder {
  request(command: "lock" | "release"): Promise<void>;
  close(): void;
}

async function storeWriterHolder(storePath: string): Promise<StoreWriterHolder> {
  const { fork: forkProcess } = await vi.importActual<typeof ChildProcess>("node:child_process");
  const directory = await mkdtemp(join(tmpdir(), "rerail-store-holder-"));
  roots.push(directory);
  const script = join(directory, "holder.cjs");
  await writeFile(script, [
    "const Database = require(process.argv[2]);",
    "const store = new Database(process.argv[3]);",
    "process.on('message', (command) => {",
    "  if (command === 'lock') store.exec('BEGIN IMMEDIATE');",
    "  if (command === 'release' && store.inTransaction) store.exec('COMMIT');",
    "  process.send(command);",
    "});",
    "process.send('ready');",
  ].join("\n"));
  const child = forkProcess(script, [createRequire(import.meta.url).resolve("better-sqlite3"), storePath], {
    stdio: ["ignore", "inherit", "inherit", "ipc"],
  });
  const reply = (command: string) => new Promise<void>((resolve, reject) => {
    const onMessage = (message: unknown) => {
      if (message !== command) return;
      child.off("message", onMessage);
      child.off("exit", reject);
      resolve();
    };
    child.on("message", onMessage);
    child.once("exit", reject);
  });
  await reply("ready");
  return {
    async request(command) {
      const replied = reply(command);
      child.send(command);
      await replied;
    },
    close: () => child.kill(),
  };
}

/**
 * Real native workspaces whose repository index takes `holder`'s store write lock once the index is done, so the
 * scan's reuse-cache write meets a busy store. `onCompletion` runs right after the enforcer starts that write.
 */
function holdStoreWriterForCompletion(holder: StoreWriterHolder, onCompletion: () => void) {
  const binding = native.loadNativeBinding();
  class Workspace {
    constructor() {
      const inner = new binding.PolicyDslWorkspace();
      return new Proxy(inner, {
        get(target, key) {
          const value: unknown = Reflect.get(target, key, target);
          if (typeof value !== "function") return value;
          if (key === "indexRepositoryAsync")
            return async (options: NativeRepositoryIndexOptions) => {
              const chunk = await target.indexRepositoryAsync(options);
              if (repositoryIndexDone.parse(JSON.parse(chunk)).done) await holder.request("lock");
              return chunk;
            };
          if (key === "completeStorageRevision" || key === "completeStorageRevisionAsync")
            return (...args: unknown[]): unknown => {
              try {
                return Reflect.apply(value, target, args);
              } finally {
                onCompletion();
              }
            };
          return value.bind(target);
        },
      });
    }
  }
  return vi
    .spyOn(native, "loadNativeBinding")
    .mockReturnValue(Object.assign(Object.create(binding) as NativeBinding, { PolicyDslWorkspace: Workspace }));
}

const repositoryIndexDone = z.object({ done: z.boolean() });

describe("Python module paths", () => {
  it("walks package __init__.py files the way Ruff does", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-modules-"));
    roots.push(root);
    await mkdir(join(root, "src/pkg/sub"), { recursive: true });
    await mkdir(join(root, "src/pkg/loose"), { recursive: true });
    await mkdir(join(root, "fn"));
    for (const init of ["src/pkg/__init__.py", "src/pkg/sub/__init__.py"]) await writeFile(join(root, init), "");
    expect(pythonModulePath(join(root, "src/pkg/sub/mod.py"))).toBe("pkg.sub.mod");
    expect(pythonModulePath(join(root, "src/pkg/__init__.py"))).toBe("pkg.__init__");
    expect(pythonModulePath(join(root, "src/pkg/sub/types.pyi"))).toBe("pkg.sub.types");
    // A directory without __init__.py ends the package: the file is outside any package.
    expect(pythonModulePath(join(root, "src/pkg/loose/mod.py"))).toBeUndefined();
    expect(pythonModulePath(join(root, "fn/contextlib.py"))).toBeUndefined();
  });

  it("sends names as on disk, as a list when one keeps a dot", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-modules-"));
    roots.push(root);
    await mkdir(join(root, "json.d"), { recursive: true });
    await mkdir(join(root, "good"), { recursive: true });
    for (const init of ["json.d/__init__.py", "good/__init__.py"]) await writeFile(join(root, init), "");
    // Ruff's N999 reads the package directory `json.d` and the stem `my.mod`; the kernel derives `to_module_path`'s
    // stems (`json`) itself for qualified names.
    expect(pythonModulePath(join(root, "json.d/__init__.py"))).toEqual(["json.d", "__init__"]);
    expect(pythonModulePath(join(root, "json.d/m.py"))).toEqual(["json.d", "m"]);
    expect(pythonModulePath(join(root, "good/my.mod.py"))).toEqual(["good", "my.mod"]);
    expect(pythonModulePath(join(root, "good/fine.py"))).toBe("good.fine");
    expect(pythonModulePath(join(root, "my.mod.py"))).toBeUndefined();
  });

  it("agrees with the module paths of the kernel's Ruff rules oracle", async () => {
    const oracle = new URL("python/scope/", rerailOraclesDirectory());
    const expected = JSON.parse(await readFile(new URL("ruff-rules.expected.json", oracle), "utf8")) as {
      cases: Array<{ files: Array<{ path: string; module: string | string[] | null }> }>;
    };
    let checked = 0;
    for (const { files } of expected.cases) {
      const root = await mkdtemp(join(tmpdir(), "rerail-modules-"));
      roots.push(root);
      for (const file of files) {
        await mkdir(dirname(join(root, file.path)), { recursive: true });
        await writeFile(join(root, file.path), "");
      }
      for (const file of files) {
        expect(pythonModulePath(join(root, file.path)) ?? null, file.path).toEqual(file.module);
        checked += 1;
      }
    }
    expect(checked).toBeGreaterThan(40);
  });

  it("agrees with the module paths of the kernel's Ruff oracle fixtures", async () => {
    const oracle = new URL("python/scope/", rerailOraclesDirectory());
    const expected = JSON.parse(await readFile(new URL("modules.expected.json", oracle), "utf8")) as { files: Array<{ path: string; module: string | null }> };
    for (const file of expected.files) {
      expect(pythonModulePath(new URL(`modules/${file.path}`, oracle).pathname) ?? null, file.path).toBe(file.module);
    }
  });
});

describe("mechanical executable policy regressions", () => {
  it("reads each repository source once without spawning an initial index worker", async () => {
    const { root, enforcer, catalog } = await fixture();
    // The walk reads below the resolved root (the temporary directory may sit behind a symlink).
    const source = join(await realpath(root), "api", "once.py");
    await writeFile(source, "safe()\n");
    vi.mocked(existsSync).mockReturnValue(true);
    vi.mocked(fork).mockImplementation(() => {
      const child = new EventEmitter();
      Object.assign(child, {
        kill: vi.fn(),
        send: vi.fn(() => queueMicrotask(() => child.emit("message", { type: "complete", peakRssBytes: 0 }))),
      });
      return child as never;
    });
    try {
      await enforcer.openWorkspace("one-read-repository-scan");
      vi.mocked(readFile).mockClear();
      await enforcer.scan("one-read-repository-scan", { scope: "repository", mode: "log" });
      expect(vi.mocked(readFile).mock.calls.filter(([path]) => path === source)).toHaveLength(1);
      expect(fork).not.toHaveBeenCalled();
    } finally {
      vi.mocked(fork).mockReset();
      vi.mocked(existsSync).mockReset();
      await enforcer.close();
      catalog.close();
    }
  });

  it("qualifies Python definitions by the module path of the edited file", async () => {
    const { recorder, runs } = recordingRuns();
    const policy = policyRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:module", revision: 1,
      mode: "steer",
      scope: { include: ["api/**/*.py"], exclude: [] },
      clauses: [{
        id: "suppress", violation: { code: "SUPPRESS", anchor: "witness.span", message: "suppress()" },
        forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { import: ["api.pkg.contextlib.suppress"] } } } },
      }],
    });
    const { root, catalog, enforcer } = await fixture({ executions: recorder, policies: [policy] });
    try {
      await mkdir(join(root, "api/pkg"));
      await writeFile(join(root, "api/__init__.py"), "");
      await writeFile(join(root, "api/pkg/__init__.py"), "");
      await enforcer.openWorkspace("module-path");
      await settled(enforcer);
      await enforcer.evaluateGeneratedEdits("module-path", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/pkg/contextlib.py", language: "python", generatedText: "def suppress(): pass\nwith suppress(): pass\n" }],
      });
      await settled(enforcer);
      expect(runForPath(runs, "api/pkg/contextlib.py")?.result).toMatchObject({
        status: "completed",
        violations: [expect.objectContaining({ code: "SUPPRESS", span: expect.objectContaining({ startLine: 1 }) })],
        failures: [],
      });
    } finally { await enforcer.close(); catalog.close(); }
  });

  it("retains interruption when closure overlaps writing accepted source to disk", async () => {
    const { root, enforcer, catalog } = await fixture();
    const written = Promise.withResolvers<void>();
    const release = Promise.withResolvers<void>();
    const actual = await vi.importActual<typeof FsPromises>("node:fs/promises");
    let accepted: Promise<unknown> | undefined;
    let closing: Promise<void> | undefined;
    try {
      await enforcer.openWorkspace("closing-during-storage");
      await settled(enforcer);
      vi.mocked(writeFile).mockImplementationOnce(async (...args) => {
        await actual.writeFile(...args);
        written.resolve();
        await release.promise;
      });
      accepted = enforcer.evaluateGeneratedEdits("closing-during-storage", {
        revision: 1, checkpoints: ["edit"],
        edits: [{ path: "api/staging.py", language: "python", generatedText: "x".repeat(9 * 1024 * 1024) }],
      });
      await written.promise;
      closing = enforcer.close();
      release.resolve();
      await Promise.all([accepted, closing]);
      const executions = new SqlitePolicyExecutionStore(defaultRerailDatabasePath());
      try {
        expect(executions.list({ sessionId: "closing-during-storage" }).audits).toEqual(expect.arrayContaining([
          expect.objectContaining({ paths: ["api/staging.py"], status: "interrupted", catalogRevision: null, ruleRevisions: [] }),
        ]));
      } finally { executions.close(); }
    } finally {
      release.resolve();
      await accepted;
      await closing;
      await enforcer.close();
      catalog.close();
      vi.mocked(writeFile).mockReset();
    }
  });

  it("records a failed run when retained source cannot be loaded", async () => {
    const { recorder, runs } = recordingRuns();
    const { enforcer, catalog } = await fixture({ executions: recorder });
    try {
      await enforcer.openWorkspace("lost-retained-source");
      await settled(enforcer);
      vi.mocked(readFile).mockRejectedValueOnce(new Error("Retained source is unavailable"));
      await enforcer.evaluateGeneratedEdits("lost-retained-source", {
        revision: 1, checkpoints: ["edit"],
        edits: [{ path: "api/retained.py", language: "python", generatedText: "x".repeat(9 * 1024 * 1024) }],
      });
      await settled(enforcer);
      expect(runForPath(runs, "api/retained.py")).toMatchObject({
        result: { status: "failed", failures: [expect.objectContaining({ message: "Retained source is unavailable" })] },
      });
    } finally {
      vi.mocked(readFile).mockReset();
      await enforcer.close();
      catalog.close();
    }
  });

  it("coalesces successive generated edits to the latest source without diagnostic interference", async () => {
    const { root, catalog, enforcer } = await fixture();
    try {
      await enforcer.openWorkspace("coalesce");
      await enforcer.evaluateGeneratedEdits("coalesce", { revision: 1, checkpoints: ["edit"], edits: [{ path: "api/a.py", language: "python", generatedText: "safe()\n" }] });
      await enforcer.evaluateGeneratedEdits("coalesce", { revision: 2, checkpoints: ["edit"], edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n" }] });
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      expect((await settled(enforcer)).violations).toEqual([expect.objectContaining({ path: "api/a.py", code: "NEEDLE" })]);
    } finally { await enforcer.close(); catalog.close(); }
  });

  it("delivers findings only for the source scope that changed", async () => {
    const { root, catalog, enforcer, requestDelivery } = await fixture();
    try {
      await writeFile(join(root, "api", "a.py"), "safe()\n");
      await writeFile(join(root, "api", "b.py"), "safe()\n");
      await enforcer.openWorkspace("scoped-delivery");

      await enforcer.evaluateGeneratedEdits("scoped-delivery", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n" }],
      });
      await settled(enforcer);
      await enforcer.evaluateGeneratedEdits("scoped-delivery", {
        revision: 2,
        checkpoints: ["edit"],
        edits: [{ path: "api/b.py", language: "python", generatedText: "safe(2)\n" }],
      });
      await settled(enforcer);
      await enforcer.evaluateGeneratedEdits("scoped-delivery", {
        revision: 3,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n# touched\n" }],
      });
      await settled(enforcer);

      expect(requestDelivery).toHaveBeenCalledTimes(2);
      expect(requestDelivery.mock.calls.map(([, , content]) => content)).toEqual([
        expect.stringContaining("api/a.py"),
        expect.stringContaining("api/a.py"),
      ]);
    } finally { await enforcer.close(); catalog.close(); }
  });

  it("credits a steer to the distinct rules whose steer and block findings it rendered", async () => {
    const { root, catalog, enforcer, requestDelivery } = await fixture({
      policies: [
        modePolicy("test:alpha", "ALPHA", "block"),
        modePolicy("test:beta", "BETA", "steer"),
        modePolicy("test:gamma", "GAMMA", "log"),
      ],
    });
    try {
      await writeFile(join(root, "api", "a.py"), "ALPHA\nBETA\nALPHA\nGAMMA\n");
      await enforcer.openWorkspace("rule-attribution");
      await enforcer.evaluateGeneratedEdits("rule-attribution", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "ALPHA\nBETA\nALPHA\nGAMMA\n" }],
      });
      const status = await settled(enforcer);

      expect(new Set(status.violations.map(finding => `${finding.ruleId}:${finding.mode}`)))
        .toEqual(new Set(["test:alpha:block", "test:beta:steer", "test:gamma:log"]));
      expect(status.disposition).toBe("steer");
      expect(requestDelivery).toHaveBeenCalledTimes(1);
      const [, , content, metadata] = requestDelivery.mock.calls[0]!;
      expect(metadata?.ruleIds).toEqual(["test:alpha", "test:beta"]);
      expect(content).toContain("ALPHA");
      expect(content).toContain("BETA");
      expect(content).not.toContain("GAMMA");
    } finally { await enforcer.close(); catalog.close(); }
  });

  it("records a log rule's findings without steering", async () => {
    const { root, catalog, enforcer, requestDelivery } = await fixture({ policies: [modePolicy("test:gamma", "GAMMA", "log")] });
    try {
      await writeFile(join(root, "api", "a.py"), "GAMMA\n");
      await enforcer.openWorkspace("log-only");
      await enforcer.evaluateGeneratedEdits("log-only", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "GAMMA\n" }],
      });
      const status = await settled(enforcer);

      expect(status.violations).toEqual([expect.objectContaining({ ruleId: "test:gamma", mode: "log" })]);
      expect(status.disposition).toBe("continue");
      expect(requestDelivery).not.toHaveBeenCalled();
    } finally { await enforcer.close(); catalog.close(); }
  });

  it("limits AI-output reporting to disjoint changed ranges with UTF-8 byte offsets", async () => {
    const { catalog, enforcer, requestDelivery } = await fixture({ scope: "ai-output" });
    try {
      await enforcer.openWorkspace("ranges");
      await enforcer.evaluateGeneratedEdits("ranges", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{
          path: "api/a.py",
          language: "python",
          previousText: "# π\nsafe_one()\nNEEDLE_old\nsafe_two()\n",
          generatedText: "# π\nNEEDLE_new_one\nNEEDLE_old\nNEEDLE_new_two\n",
        }],
      });
      expect((await settled(enforcer)).violations.map(finding => finding.span?.startLine)).toEqual([1, 3]);
      expect(requestDelivery).toHaveBeenCalledTimes(1);
    } finally { await enforcer.close(); catalog.close(); }
  });

  it("settles a repository-scope scan triggered by a generated edit", async () => {
    const { root, catalog, enforcer, requestDelivery } = await fixture({ scope: "repository", mode: "steer" });
    try {
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      await enforcer.openWorkspace("repository-edit");
      await enforcer.evaluateGeneratedEdits("repository-edit", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n" }],
      });
      await enforcer.settleGeneratedEdits();
      expect(requestDelivery).toHaveBeenCalledWith(
        "repository-edit",
        "steer",
        expect.stringContaining("NEEDLE"),
        expect.objectContaining({ revision: 1, generatedEdit: true }),
      );
    } finally { await enforcer.close(); catalog.close(); }
  });

  it("scans a Git checkout through the native repository index and records its clean directories for reuse", async () => {
    const { recorder, runs } = recordingRuns();
    const { root, catalog, enforcer } = await fixture({ executions: recorder });
    try {
      await writeFile(join(root, ".gitignore"), ".rerail/\ncapabilities.json\napi/ignored.py\n");
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      await writeFile(join(root, "api", "binary.py"), "NEEDLE\0\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "."], { cwd: root });
      execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
      await writeFile(join(root, "api", "ignored.py"), "NEEDLE\n");
      await enforcer.openWorkspace("git-repository-scan");
      const before = completedDirectories();
      const [runId] = await enforcer.scan("git-repository-scan", { scope: "repository", mode: "log" });
      expect(runs.get(runId!)?.result).toMatchObject({
        status: "failed",
        violations: [
          expect.objectContaining({ path: "api/a.py", code: "NEEDLE" }),
          expect.objectContaining({ path: "api/ignored.py", code: "NEEDLE" }),
        ],
        failures: [expect.objectContaining({ path: "api/binary.py", code: "source-incomplete", stage: "read" })],
      });
      expect(completedDirectories()).toBeGreaterThan(before);
    } finally { await enforcer.close(); catalog.close(); }
  });

  it("indexes a Git checkout's files of every source language a scan evaluates", async () => {
    const { recorder, runs } = recordingRuns();
    const anywhere = policyRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:anywhere", revision: 1,
      mode: "steer",
      scope: { include: ["src/**/*"], exclude: [] },
      clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
    });
    const { root, catalog, enforcer } = await fixture({ executions: recorder, policies: [anywhere] });
    const paths = [...SOURCE_LANGUAGES.keys()].map((extension) => `src/file${extension}`).toSorted();
    try {
      await mkdir(join(root, "src"));
      for (const path of paths) await writeFile(join(root, path), "NEEDLE\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "src"], { cwd: root });
      execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
      await enforcer.openWorkspace("all-languages");
      const [runId] = await enforcer.scan("all-languages", { scope: "repository", mode: "log" });
      expect(runs.get(runId!)?.result?.violations?.map((finding) => finding.path).toSorted()).toEqual(paths);
    } finally { await enforcer.close(); catalog.close(); }
  });

  it("records each scan whose native repository index throws as a failed run", async () => {
    const { recorder, runs } = recordingRuns();
    const real = native.loadNativeBinding();
    class Workspace {
      constructor() {
        const inner = new real.PolicyDslWorkspace();
        return new Proxy(inner, {
          get(target, key) {
            const value: unknown = Reflect.get(target, key, target);
            if (typeof value !== "function") return value;
            if (key === "indexRepositoryAsync") return async () => { throw new Error("injected index failure"); };
            return value.bind(target);
          },
        });
      }
    }
    const binding = vi
      .spyOn(native, "loadNativeBinding")
      .mockReturnValue(Object.assign(Object.create(real) as NativeBinding, { PolicyDslWorkspace: Workspace }));
    const { root, catalog, enforcer } = await fixture({ executions: recorder });
    try {
      await writeFile(join(root, ".gitignore"), ".rerail/\ncapabilities.json\n");
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "."], { cwd: root });
      execFileSync("git", gitCommit, { cwd: root });
      await enforcer.openWorkspace("failing-index");
      for (let scan = 0; scan < 2; scan += 1) {
        const [runId] = await enforcer.scan("failing-index", { scope: "repository", mode: "log" });
        expect(runId).toBeDefined();
        expect(runs.get(runId!)).toMatchObject({
          scope: "repository",
          result: {
            status: "failed",
            violations: [],
            failures: [expect.objectContaining({ clauseId: "scan", code: "adapter-error", stage: "scan", message: "injected index failure" })],
          },
        });
      }
      expect(runs.size).toBe(2);
    } finally {
      binding.mockRestore();
      await enforcer.close();
      catalog.close();
    }
  });

  it("spools oversized pending batches and reclaims the spool directory", async () => {
    const { catalog, enforcer } = await fixture();
    try {
      await enforcer.openWorkspace("spool");
      const source = `NEEDLE\n${"x".repeat(850_000)}`;
      const edits = Array.from({ length: 12 }, (_, index) => ({ path: `api/generated-${index}.py`, language: "python" as const, generatedText: source }));
      await enforcer.evaluateGeneratedEdits("spool", { revision: 1, checkpoints: ["edit"], edits });
      await settled(enforcer);
      expect((await settled(enforcer)).violations).toHaveLength(12);
    } finally { await enforcer.close(); catalog.close(); }
  }, 60_000);

  it("evaluates TypeScript AST rules against React files with the JSX grammar", async () => {
    const { recorder, runs } = recordingRuns();
    const { catalog, enforcer } = await fixture({ executions: recorder, policies: [consolePolicy(1)] });
    try {
      await enforcer.openWorkspace("jsx-grammar");
      await settled(enforcer);
      await enforcer.evaluateGeneratedEdits("jsx-grammar", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{
          path: "web/App.tsx", language: "tsx",
          generatedText: "export function App() {\n  console.log('render');\n  return <main className=\"app\" />;\n}\n",
        }],
      });
      await settled(enforcer);

      expect(runForPath(runs, "web/App.tsx")?.result).toMatchObject({
        status: "completed",
        violations: [expect.objectContaining({ code: "CONSOLE", path: "web/App.tsx" })],
        failures: [],
      });
    } finally { await enforcer.close(); catalog.close(); }
  });

  it("steers with the anchored location, interpolated message, and suggested fix", async () => {
    const { recorder, runs } = recordingRuns();
    const fixing = policyRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:logger", revision: 1, mode: "block",
      scope: { include: ["web/**/*.tsx"], exclude: [] },
      clauses: [{
        id: "logger",
        violation: { code: "LOGGER", anchor: "capture.ARG", message: "Log $ARG through the logger", fix: "logger.info($ARG)" },
        forbid: { ast: { language: "typescript", rule: { pattern: "console.log($ARG)" } } },
      }],
    });
    const { root, catalog, enforcer, requestDelivery } = await fixture({ executions: recorder, policies: [fixing] });
    try {
      const source = "export function App() {\n  console.log(user);\n  return <main />;\n}\n";
      await mkdir(join(root, "web"));
      await writeFile(join(root, "web", "App.tsx"), source);
      await enforcer.openWorkspace("fix-steer");
      await settled(enforcer);
      await enforcer.evaluateGeneratedEdits("fix-steer", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "web/App.tsx", language: "tsx", generatedText: source }],
      });
      await settled(enforcer);

      expect(requestDelivery).toHaveBeenCalledTimes(1);
      expect(requestDelivery.mock.calls[0]![2]).toBe([
        "LOGGER (web/App.tsx:2:15): Log user through the logger",
        "  Suggested fix: replace 2:3-2:20 with `logger.info(user)`",
      ].join("\n"));
      expect(runForPath(runs, "web/App.tsx")?.result).toMatchObject({
        violations: [expect.objectContaining({ fix: { span: expect.objectContaining({ startLine: 1, startColumn: 2 }), text: "logger.info(user)" } })],
      });
      // The fix is a suggestion only: the source on disk is untouched.
      expect(await readFile(join(root, "web", "App.tsx"), "utf8")).toBe(source);
    } finally { await enforcer.close(); catalog.close(); }
  });

  it("records the replacement catalog that evaluates a queued edit", async () => {
    const { recorder, runs } = recordingRuns();
    const { catalog, databasePath, enforcer } = await fixture({ executions: recorder });
    try {
      await enforcer.openWorkspace("catalog-provenance");
      await settled(enforcer);

      await enforcer.evaluateGeneratedEdits("catalog-provenance", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/queued.py", language: "python", generatedText: "FRESH\n" }],
      });
      const writer = new SqliteRuleStore(databasePath);
      writer.upsertPolicy(needlePolicy(2, "FRESH", "FRESH"));
      writer.close();
      await enforcer.evaluateGeneratedEdits("catalog-provenance", {
        revision: 2,
        checkpoints: ["edit"],
        edits: [{ path: "api/refresh.py", language: "python", generatedText: "safe()\n" }],
      });
      await settled(enforcer);

      const run = runForPath(runs, "api/queued.py");
      expect(run).toMatchObject({
        catalogRevision: catalog.status.version,
        ruleRevisions: [{ ruleId: "test:needle", revision: 2 }],
        result: { status: "completed", violations: [expect.objectContaining({ code: "FRESH", path: "api/queued.py" })] },
      });
    } finally { await enforcer.close(); catalog.close(); }
  });

  it("installs the newest catalog when an older replacement finishes attaching storage last", async () => {
    const { recorder, runs } = recordingRuns();
    const { catalog, databasePath, enforcer } = await fixture({ executions: recorder });
    const attachments = gateStorageAttachments();
    try {
      attachments.gated = false;
      await enforcer.openWorkspace("newest-catalog");
      await settled(enforcer);
      attachments.gated = true;
      const writer = new SqliteRuleStore(databasePath);
      writer.upsertPolicy(needlePolicy(2, "TWO", "TWO"));
      const older = enforcer.refreshPolicies();
      writer.upsertPolicy(needlePolicy(3, "THREE", "THREE"));
      const newer = enforcer.refreshPolicies();
      writer.close();
      const [, olderWorkspace, newerWorkspace] = attachments.workspaces;
      newerWorkspace!.release();
      await newerWorkspace!.attached;
      await newer;
      olderWorkspace!.release();
      await olderWorkspace!.attached;
      await older;

      await enforcer.evaluateGeneratedEdits("newest-catalog", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/both.py", language: "python", generatedText: "TWO\nTHREE\n" }],
      });
      await settled(enforcer);
      expect(runForPath(runs, "api/both.py")).toMatchObject({
        catalogRevision: catalog.status.version,
        ruleRevisions: [{ ruleId: "test:needle", revision: 3 }],
        result: { status: "completed", violations: [expect.objectContaining({ code: "THREE", path: "api/both.py" })] },
      });
    } finally {
      for (const workspace of attachments.workspaces) workspace.release();
      attachments.restore();
      await enforcer.close();
      catalog.close();
    }
  });

  it("closes a workspace still attaching storage when the session closes during open", async () => {
    const { catalog, enforcer } = await fixture();
    const attachments = gateStorageAttachments();
    try {
      const opening = enforcer.openWorkspace("close-during-open");
      await enforcer.close();
      for (const workspace of attachments.workspaces) workspace.release();
      await Promise.all(attachments.workspaces.map(workspace => workspace.attached));
      await opening;
      expect(attachments.workspaces.map(workspace => workspace.native.acceptsPath("api/a.py"))).toEqual([false]);
    } finally {
      for (const workspace of attachments.workspaces) workspace.release();
      attachments.restore();
      await enforcer.close();
      catalog.close();
    }
  });

  it("closes a replacement still attaching storage when the session closes during a policy refresh", async () => {
    const { catalog, databasePath, enforcer } = await fixture();
    const attachments = gateStorageAttachments();
    try {
      attachments.gated = false;
      await enforcer.openWorkspace("close-during-refresh");
      attachments.gated = true;
      const writer = new SqliteRuleStore(databasePath);
      writer.upsertPolicy(needlePolicy(2, "FRESH", "FRESH"));
      writer.close();
      const refreshing = enforcer.refreshPolicies();
      await enforcer.close();
      for (const workspace of attachments.workspaces) workspace.release();
      await Promise.all(attachments.workspaces.map(workspace => workspace.attached));
      await refreshing;
      expect(attachments.workspaces.map(workspace => workspace.native.acceptsPath("api/a.py"))).toEqual([false, false]);
    } finally {
      for (const workspace of attachments.workspaces) workspace.release();
      attachments.restore();
      await enforcer.close();
      catalog.close();
    }
  });

  it("reads no Git checkout source in JavaScript during repository diagnostics", async () => {
    const { recorder, runs } = recordingRuns();
    const { root, catalog, enforcer } = await fixture({ executions: recorder });
    const source = join(await realpath(root), "api", "once.py");
    await writeFile(source, "NEEDLE\n");
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    execFileSync("git", ["add", "."], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
    const reads = vi.spyOn(fsPromises, "readFile");
    try {
      await enforcer.openWorkspace("single-read");
      const [runId] = await enforcer.scan("single-read", { scope: "repository", mode: "log" });
      expect(runs.get(runId!)?.result?.violations).toEqual([expect.objectContaining({ path: "api/once.py", code: "NEEDLE" })]);
      expect(reads.mock.calls.filter(([path]) => path === source)).toHaveLength(0);
    } finally {
      reads.mockRestore();
      await enforcer.close();
      catalog.close();
    }
  });

  it("evaluates the files the walk selects whether or not the workspace is a Git checkout", async () => {
    const plain = await mkdtemp(join(tmpdir(), "rerail-scan-plain-"));
    const checkout = await mkdtemp(join(tmpdir(), "rerail-scan-git-"));
    roots.push(plain, checkout);
    await writeScanTree(plain, false);
    await writeScanTree(checkout, true);
    const walked = await repositoryScanPaths(plain);
    expect(walked).toEqual(["api/a.py", "api/gen/c.py", "api/nested/d.py", "api/nested/e.py", "api/skip/f.py", "api/tmp/b.py"]);
    expect(await repositoryScanPaths(checkout)).toEqual(walked);
    for (const workspace of ["tmp/ws", "out/ws"]) {
      expect(await repositoryScanPaths(join(plain, workspace))).toEqual(["api/a.py"]);
      expect(await repositoryScanPaths(join(checkout, workspace))).toEqual(["api/a.py"]);
    }
  }, 60_000);

  it("scans a workspace root reached through a symlink as its target, with or without Git", async () => {
    const plain = await mkdtemp(join(tmpdir(), "rerail-scan-plain-"));
    const checkout = await mkdtemp(join(tmpdir(), "rerail-scan-git-"));
    const links = await mkdtemp(join(tmpdir(), "rerail-scan-links-"));
    roots.push(plain, checkout, links);
    await writeScanTree(plain, false);
    await writeScanTree(checkout, true);
    await symlink(plain, join(links, "plain"));
    await symlink(checkout, join(links, "checkout"));
    const walked = await repositoryScanPaths(plain);
    expect(walked).toContain("api/a.py");
    expect(await repositoryScanPaths(join(links, "plain"))).toEqual(walked);
    expect(await repositoryScanPaths(join(links, "checkout"))).toEqual(walked);
  }, 60_000);

  it("scans a Git checkout holding unreadable untracked directories as the walk does", async () => {
    const plain = await mkdtemp(join(tmpdir(), "rerail-scan-plain-"));
    const checkout = await mkdtemp(join(tmpdir(), "rerail-scan-git-"));
    roots.push(plain, checkout);
    const restores: (() => Promise<void>)[] = [];
    try {
      for (const root of [plain, checkout]) {
        await mkdir(join(root, "api"));
        await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
        if (root === checkout) {
          execFileSync("git", ["init", "--quiet"], { cwd: root });
          execFileSync("git", ["add", "."], { cwd: root });
          execFileSync("git", gitCommit, { cwd: root });
        }
        // Untracked and not gitignored; `tmp/` is also outside Rerail's Git status.
        for (const directory of ["new", "tmp"]) restores.push(await unreadableDirectory(join(root, "api", directory)));
      }
      const walked = await repositoryScan(plain);
      expect(walked).toEqual({
        status: "failed",
        paths: ["api/a.py"],
        failures: ["api/new source-incomplete", "api/tmp source-incomplete"],
      });
      expect(await repositoryScan(checkout)).toEqual(walked);
    } finally {
      for (const restore of restores) await restore();
    }
  }, 60_000);

  it("evaluates generated edits in a Git checkout holding an unreadable untracked directory", async () => {
    const { recorder, runs } = recordingRuns();
    const { root, catalog, enforcer } = await fixture({ executions: recorder });
    let restore: () => Promise<void> = async () => undefined;
    try {
      await writeFile(join(root, "api", "a.py"), "safe()\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "."], { cwd: root });
      execFileSync("git", gitCommit, { cwd: root });
      restore = await unreadableDirectory(join(root, "api", "new"));
      await enforcer.openWorkspace("unreadable-untracked");
      await enforcer.evaluateGeneratedEdits("unreadable-untracked", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n" }],
      });
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      expect((await settled(enforcer)).violations).toEqual([expect.objectContaining({ path: "api/a.py", code: "NEEDLE" })]);
      expect(runForPath(runs, "api/a.py")?.result).toMatchObject({ status: "completed", failures: [] });
    } finally {
      await restore();
      await enforcer.close();
      catalog.close();
    }
  });

  it("records a Git checkout's clean directories while another process holds the store writer", async () => {
    const { recorder, runs } = recordingRuns();
    const { root, catalog, enforcer } = await fixture({ executions: recorder });
    const holder = await storeWriterHolder(process.env.RERAIL_CONTENT_STORE_PATH!);
    // Only a JavaScript thread left free while the reuse-cache write waits can hand the lock back.
    const binding = holdStoreWriterForCompletion(holder, () =>
      queueHostTurn(() => void holder.request("release").catch(() => undefined)));
    try {
      await writeFile(join(root, ".gitignore"), ".rerail/\ncapabilities.json\n");
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "."], { cwd: root });
      execFileSync("git", gitCommit, { cwd: root });
      await enforcer.openWorkspace("busy-store-completion");
      const before = completedDirectories();
      const [runId] = await enforcer.scan("busy-store-completion", { scope: "repository", mode: "log" });
      expect(runs.get(runId!)?.result).toMatchObject({
        status: "completed",
        violations: [expect.objectContaining({ path: "api/a.py", code: "NEEDLE" })],
        failures: [],
      });
      expect(completedDirectories()).toBeGreaterThan(before);
    } finally {
      binding.mockRestore();
      holder.close();
      await enforcer.close();
      catalog.close();
    }
  }, 30_000);

  it("keeps a finished scan's findings when its reuse-cache write fails", async () => {
    const { recorder, runs } = recordingRuns();
    const { root, catalog, enforcer } = await fixture({ executions: recorder });
    const holder = await storeWriterHolder(process.env.RERAIL_CONTENT_STORE_PATH!);
    const binding = holdStoreWriterForCompletion(holder, () => undefined);
    try {
      await writeFile(join(root, ".gitignore"), ".rerail/\ncapabilities.json\n");
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "."], { cwd: root });
      execFileSync("git", gitCommit, { cwd: root });
      await enforcer.openWorkspace("failed-completion");
      const before = completedDirectories();
      const [runId] = await enforcer.scan("failed-completion", { scope: "repository", mode: "log" });
      await holder.request("release");
      expect(completedDirectories()).toBe(before);
      expect(runs.get(runId!)?.result).toMatchObject({
        status: "completed",
        violations: [expect.objectContaining({ path: "api/a.py", code: "NEEDLE" })],
        failures: [],
      });
    } finally {
      binding.mockRestore();
      holder.close();
      await enforcer.close();
      catalog.close();
    }
  }, 30_000);

  it("closes a workspace whose storage attach throws before it starts", async () => {
    const { catalog, enforcer } = await fixture();
    let closed = false;
    const workspace = Object.assign(pagingWorkspace(() => { throw new Error("the workspace is never paged"); }), {
      enableStorageAsync: () => { throw new Error("content store path is required"); },
      close: () => { closed = true; },
    });
    const binding = nativeWorkspaces(workspace);
    try {
      await enforcer.openWorkspace("throwing-attach");
      expect(closed).toBe(true);
    } finally {
      binding.mockRestore();
      await enforcer.close();
      catalog.close();
    }
  });

  it("reports once why repository reuse is disabled when the storage attach says why", async () => {
    const { recorder, runs } = recordingRuns();
    const { catalog, enforcer } = await fixture({ executions: recorder });
    const reason = "cannot read .rerailignore: Is a directory (os error 21)";
    const unattached = () => Object.assign(pagingWorkspace((cursor) => openPage(cursor, 1)), {
      enableStorageAsync: async () => ({ attached: false, reason }),
    });
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const binding = nativeWorkspaces(unattached(), unattached(), unattached());
    try {
      await enforcer.openWorkspace("unattached-reuse");
      for (let scan = 0; scan < 2; scan += 1) {
        const [runId] = await enforcer.scan("unattached-reuse", { scope: "repository", mode: "log" });
        expect(runs.get(runId!)?.result?.status).toBe("completed");
      }
      const reports = stderr.mock.calls.map(([chunk]) => String(chunk)).filter((line) => line.includes("repository reuse"));
      expect(reports).toEqual([`rerail: repository reuse is disabled: ${reason}\n`]);
    } finally {
      stderr.mockRestore();
      binding.mockRestore();
      await enforcer.close();
      catalog.close();
    }
  });

  it("never applies to a replacement that a newer catalog superseded while it replayed", async () => {
    const { catalog, databasePath, enforcer } = await fixture();
    const unpaged = () => { throw new Error("the workspace is never paged"); };
    let supersededClosed = false;
    let appliesAfterClose = 0;
    let superseding = false;
    const superseded = Object.assign(pagingWorkspace(unpaged), {
      applyBufferAsync: async () => {
        if (supersededClosed) appliesAfterClose += 1;
        else if (!superseding) {
          // The first replayed edit lands while a newer catalog arrives.
          superseding = true;
          const writer = new SqliteRuleStore(databasePath);
          writer.upsertPolicy(needlePolicy(3, "THREE", "THREE"));
          writer.close();
          enforcer.refreshPolicies();
        }
        return completedApply;
      },
      close: () => { supersededClosed = true; },
    });
    const binding = nativeWorkspaces(pagingWorkspace(unpaged), superseded, pagingWorkspace(unpaged));
    try {
      await enforcer.openWorkspace("superseded-replay");
      await enforcer.evaluateGeneratedEdits("superseded-replay", {
        revision: 1, checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "safe()\n" }],
      });
      await settled(enforcer);
      const writer = new SqliteRuleStore(databasePath);
      writer.upsertPolicy(needlePolicy(2, "TWO", "TWO"));
      writer.close();
      await enforcer.evaluateGeneratedEdits("superseded-replay", {
        revision: 2, checkpoints: ["edit"],
        edits: [{ path: "api/b.py", language: "python", generatedText: "safe()\n" }],
      });
      await settled(enforcer);
      expect(superseding).toBe(true);
      expect(supersededClosed).toBe(true);
      expect(appliesAfterClose).toBe(0);
    } finally {
      binding.mockRestore();
      await enforcer.close();
      catalog.close();
    }
  });
});

const completedApply = JSON.stringify({
  revision: 1,
  evaluation: { status: "completed", violations: [], failures: [], coverage: [], metrics: { files: 0, visitedNodes: 0, witnesses: 0 } },
});

/** A native workspace double that serves result pages from `page` and reports each apply to `onApply`. */
function pagingWorkspace(page: (cursor: number) => string, onApply: () => void = () => undefined): NativePolicyDslWorkspace {
  return {
    enableStorageAsync: async () => ({ attached: true }), addPolicy: () => undefined, acceptsPath: () => true, tryApply: () => null,
    applyAsync: async () => { onApply(); return completedApply; },
    applyBufferAsync: async () => { onApply(); return completedApply; },
    memoryProfile: () => JSON.stringify({}),
    resultPageAsync: async (cursor: number) => page(cursor),
    evictInactive: () => false, nodeReference: () => null,
    resolveNodeReference: () => JSON.stringify(null), setDocumentResidencyLimit: () => undefined, close: () => undefined,
  } as unknown as NativePolicyDslWorkspace;
}

/** Serves the given workspaces, in order, to each `new PolicyDslWorkspace()`; policies still compile natively. */
function nativeWorkspaces(...workspaces: NativePolicyDslWorkspace[]) {
  const binding = native.loadNativeBinding();
  return vi.spyOn(native, "loadNativeBinding").mockReturnValue(Object.assign(Object.create(binding) as NativeBinding, {
    pythonModulePath: () => null,
    PolicyDslWorkspace: class {
      constructor() {
        const next = workspaces.shift();
        if (next === undefined) throw new Error("unexpected native workspace");
        return next;
      }
    },
  }));
}

/** A result page that never finishes before `limit` pages. */
function openPage(cursor: number, limit: number, done = false): string {
  return JSON.stringify({ revision: 1, cursor: cursor + 1, done: done || cursor + 1 >= limit, violations: [], failures: [], coverage: [], bytes: 0 });
}

describe("result paging", () => {
  it("applies session edits while a scan pages its private workspace", async () => {
    const { recorder, runs } = recordingRuns();
    const { root, catalog, enforcer } = await fixture({ executions: recorder });
    const limit = 50;
    const pages: number[] = [];
    let requested = false;
    let sessionApplyPage: number | undefined;
    let edit: Promise<unknown> | undefined;
    const session = pagingWorkspace(
      () => { throw new Error("the session workspace is never paged"); },
      () => { if (requested) sessionApplyPage ??= pages.length; },
    );
    const scan = pagingWorkspace((cursor) => {
      pages.push(cursor);
      if (cursor === 0)
        queueHostTurn(() => {
          requested = true;
          edit = enforcer.evaluateGeneratedEdits("page-session", {
            revision: 1, checkpoints: ["edit"],
            edits: [{ path: "api/apply.py", language: "python", generatedText: "safe()\n" }],
          });
        });
      // The scan keeps paging until the session edit reaches its workspace (or the page limit).
      return openPage(cursor, limit, sessionApplyPage !== undefined);
    });
    const binding = nativeWorkspaces(session, scan);
    try {
      await writeFile(join(root, "api", "page.py"), "safe()\n");
      await enforcer.openWorkspace("page-session");
      await enforcer.scan("page-session", { scope: "touched-files", mode: "log", paths: ["api/page.py"] });
      await edit;
      await enforcer.settleGeneratedEdits();
      expect(sessionApplyPage).toBeDefined();
      expect(sessionApplyPage!).toBeLessThan(limit);
      expect(pages.length).toBeLessThan(limit);
      expect(runs.get("run-1")?.result?.status).toBe("completed");
    } finally {
      binding.mockRestore();
      await enforcer.close();
      catalog.close();
    }
  });

  it("stops paging once the enforcer is interrupted", async () => {
    const { root, catalog, enforcer } = await fixture({ executions: recordingRuns().recorder });
    const pages: number[] = [];
    const session = pagingWorkspace(() => { throw new Error("the session workspace is never paged"); });
    const scan = pagingWorkspace((cursor) => {
      pages.push(cursor);
      if (cursor === 0) queueHostTurn(() => enforcer.interrupt());
      return openPage(cursor, 50);
    });
    const binding = nativeWorkspaces(session, scan);
    try {
      await writeFile(join(root, "api", "page.py"), "safe()\n");
      await enforcer.openWorkspace("page-interrupt");
      await enforcer.scan("page-interrupt", { scope: "touched-files", mode: "log", paths: ["api/page.py"] });
      expect(pages).toEqual([0]);
    } finally {
      binding.mockRestore();
      await enforcer.close();
      catalog.close();
    }
  });

  it("yields to the host before fetching the next result page", async () => {
    const { recorder, runs } = recordingRuns();
    const { root, catalog, enforcer } = await fixture({ executions: recorder });
    const pages: number[] = [];
    let hostTurn = false;
    const session = pagingWorkspace(() => { throw new Error("the session workspace is never paged"); });
    const scan = pagingWorkspace((cursor) => {
      pages.push(cursor);
      if (cursor === 0) queueHostTurn(() => { hostTurn = true; });
      else expect(hostTurn).toBe(true);
      return openPage(cursor, 2);
    });
    const binding = nativeWorkspaces(session, scan);
    try {
      await writeFile(join(root, "api", "page.py"), "safe()\n");
      await enforcer.openWorkspace("page-yield");
      await enforcer.scan("page-yield", { scope: "touched-files", mode: "log", paths: ["api/page.py"] });
      expect(pages).toEqual([0, 1]);
      expect(runs.get("run-1")?.result?.status).toBe("completed");
    } finally {
      binding.mockRestore();
      await enforcer.close();
      catalog.close();
    }
  });

  it("rejects a scan whose later result page holds a malformed violation", async () => {
    const { recorder, runs } = recordingRuns();
    const { root, catalog, enforcer } = await fixture({ executions: recorder });
    const finding = {
      id: "finding-1", ruleId: "test:needle", clauseId: "needle", code: "NEEDLE", mode: "steer",
      message: "needle", path: "api/page.py", captures: {}, variables: {},
    };
    const session = pagingWorkspace(() => { throw new Error("the session workspace is never paged"); });
    const scan = pagingWorkspace((cursor) => JSON.stringify({
      revision: 1, cursor: cursor + 1, done: cursor === 1, failures: [], coverage: [], bytes: 0,
      // The first page is valid; the second's violation has a mode the contract does not allow.
      violations: [cursor === 0 ? finding : { ...finding, id: "finding-2", mode: "fatal" }],
    }));
    const binding = nativeWorkspaces(session, scan);
    try {
      await writeFile(join(root, "api", "page.py"), "safe()\n");
      await enforcer.openWorkspace("page-malformed");
      await enforcer.scan("page-malformed", { scope: "touched-files", mode: "log", paths: ["api/page.py"] });
      expect(runs.get("run-1")?.result).toEqual({
        status: "failed",
        violations: [],
        failures: [expect.objectContaining({ clauseId: "scan", code: "adapter-error", stage: "scan" })],
      });
    } finally {
      binding.mockRestore();
      await enforcer.close();
      catalog.close();
    }
  });
});
