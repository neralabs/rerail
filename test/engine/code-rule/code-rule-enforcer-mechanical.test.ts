import { execFileSync, fork } from "node:child_process";
import type * as ChildProcess from "node:child_process";
import { EventEmitter, once } from "node:events";
import { existsSync, opendirSync, type Dir, type Dirent } from "node:fs";
import * as fsPromises from "node:fs/promises";
import type * as FsPromises from "node:fs/promises";
import { chmod, link, mkdir, mkdtemp, opendir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { setImmediate as queueHostTurn } from "node:timers";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { z } from "zod";
import { isSourceGap, type AuditRecorder, type AuditInput, type AuditResult } from "../../../src/engine/audit/audit.js";
import { defaultRerailDatabasePath } from "../../../src/engine/storage/database.js";
import { SqliteAuditStore } from "../../../src/engine/audit/audit-store.js";
import type { CodeRuleEdit, SteerMetadata } from "../../../src/engine/code-rule/code-rule.js";
import type { CodeRuleDslCatalog, RerailRuleEntry } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { AUDIT_MAX_SOURCE_BYTES, MAX_SOURCE_BYTES, modulePath, NativeCodeRuleEnforcer } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import { SOURCE_LANGUAGES } from "../../../src/engine/code-rule/code-rule-languages.js";
import { bundledCodeRuleDslDirectory } from "../../../src/engine/code-rule/code-rule-compile.js";
import { type CodeRule, codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import type { RuleMode } from "../../../src/engine/catalog/rule-mode.js";
import type {
  NativeBinding,
  NativeCodeRuleDslWorkspace,
  NativeRepositoryIndexOptions,
} from "../../../src/engine/native/native.js";
import { linkFolder } from "../../support/platform/links.js";
import { fixturesPath } from "../../support/oracles.js";
import { testCodeRuleCatalog } from "../../support/repository-rules.js";
import { gateStorageAttachments } from "../support/storage-attach-gate.js";
import { SENSITIVE_FILE_NAMES, SENSITIVE_FOLDER_NAMES } from "../../../src/engine/path-boundary.js";
import * as native from "../../../src/engine/native/native.js";
import { holdRemovals } from "../../support/platform/held-removal.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { queryContentStore } from "../../support/engine/content-store-query.js";
import { GIT_OPEN_AGAIN_MS, nativeGitAccess } from "../../../src/engine/native/git-access.js";


vi.mock("node:fs/promises", { spy: true });
vi.mock("node:child_process", { spy: true });
vi.mock("node:fs", { spy: true });



const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => removeFolder(root))); });

function needleCodeRule(revision: number, pattern = "NEEDLE", code = "NEEDLE") {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1", id: "test:needle", revision, mode: "block",
    scope: { include: ["api/**/*.py"], exclude: [] },
    clauses: [{ id: "needle", violation: { code, anchor: "witness.span", message: `No ${pattern}` }, forbid: { regex: { pattern, source: "file.content" } } }],
  });
}

function modeCodeRule(id: string, pattern: string, mode: RuleMode) {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1", id, revision: 1, mode,
    scope: { include: ["api/**/*.py"], exclude: [] },
    clauses: [{ id: "match", violation: { code: pattern, anchor: "witness.span", message: `No ${pattern}` }, forbid: { regex: { pattern, source: "file.content" } } }],
  });
}

function recordingAudits(): { recorder: AuditRecorder; audits: Map<string, AuditInput & { result?: AuditResult }>; finished: string[] } {
  const audits = new Map<string, AuditInput & { result?: AuditResult }>();
  const finished: string[] = [];
  let sequence = 0;
  return {
    recorder: {
      start(input) {
        const auditId = `run-${++sequence}`;
        audits.set(auditId, { ...input, ruleRevisions: [...input.ruleRevisions], paths: [...input.paths] });
        return auditId;
      },
      finish(auditId, result) {
        audits.get(auditId)!.result = result;
        finished.push(audits.get(auditId)!.paths[0] ?? "");
      },
      progress(auditId, result) { audits.get(auditId)!.result = { status: "completed", ...result }; },
      interrupt() {},
    },
    audits,
    finished,
  };
}

/** `recordingAudits` where an audit keeps its first result, as the audit store does: a later finish changes nothing. */
function firstResultAudits(): ReturnType<typeof recordingAudits> {
  const recording = recordingAudits();
  const finish = recording.recorder.finish.bind(recording.recorder);
  const keepFirst = (auditId: string, result: AuditResult) => {
    if (recording.audits.get(auditId)?.result === undefined) finish(auditId, result);
  };
  return { ...recording, recorder: { ...recording.recorder, finish: keepFirst } };
}

/** At most one NEEDLE among the Python sources under api/: a count across files. */
function needleBudgetRule() {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1", id: "test:needle-budget", revision: 1, mode: "block",
    scope: { include: ["api/**/*.py"], exclude: [] },
    clauses: [{
      id: "budget", violation: { code: "BUDGET", anchor: "witness.span", message: "One NEEDLE at most" },
      budget: {
        query: { evidence: { as: "n", query: { ast: { language: "python", rule: { kind: "identifier", regex: "^NEEDLE$" } } }, project: { path: "file.path" } } },
        count: { lte: 1 },
      },
    }],
  });
}

/**
 * A walk's `log` edits of `logged` (revision 1), then the session's own edits of `own` (revision 2), each file written
 * with its text first. Returns once both batches are queued.
 */
async function logThenOwn(
  enforcer: NativeCodeRuleEnforcer,
  root: string,
  { session, logged, own, loggedText = "safe()\n", ownText = "NEEDLE\n" }: { session: string; logged: readonly string[]; own: readonly string[]; loggedText?: string; ownText?: string },
): Promise<void> {
  for (const path of logged) await writeFile(join(root, path), loggedText);
  for (const path of own) await writeFile(join(root, path), ownText);
  await enforcer.openWorkspace(session);
  await enforcer.auditGeneratedEdits(session, {
    revision: 1, checkpoints: ["edit"], mode: "log", edits: logged.map(path => ({ path, language: "python" as const, generatedText: loggedText })),
  });
  await enforcer.auditGeneratedEdits(session, {
    revision: 2, checkpoints: ["edit"], edits: own.map(path => ({ path, language: "python" as const, generatedText: ownText })),
  });
}

const LOGGED = ["l0", "l1", "l2", "l3", "l4"].map(name => `api/${name}.py`);

function auditForPath(audits: ReadonlyMap<string, AuditInput & { result?: AuditResult }>, path: string) {
  return [...audits.values()].find(audit => audit.paths.includes(path));
}


function consoleCodeRule(revision: number) {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1", id: "test:console", revision, mode: "block",
    scope: { include: ["web/**/*.ts", "web/**/*.tsx"], exclude: [] },
    clauses: [{
      id: "console", violation: { code: "CONSOLE", anchor: "witness.span", message: "No console output" },
      forbid: { ast: { language: "typescript", rule: { pattern: "console.log($ARG)" } } },
    }],
  });
}

/** The code rules a test catalog serves; `upsert` replaces a code rule by id, as a library edit would. */
function editableCodeRules(initial: readonly CodeRule[]) {
  let current = [...initial];
  return {
    list: (): readonly CodeRule[] => current,
    upsert(codeRule: CodeRule) { current = [...current.filter(existing => existing.id !== codeRule.id), codeRule]; },
  };
}

async function fixture({ codeRules = [needleCodeRule(1)], undelivered, revised, ...options }: {
  scope?: "repository" | "touched-files" | "ai-output";
  mode?: "steer" | "log";
  audits?: AuditRecorder;
  codeRules?: readonly CodeRule[];
  timingLog?: (line: string) => void;
  steerShell?: "posix" | "git-bash" | "powershell";
  /** The runtime's answer to whether a path's steer was dropped before delivery (`RuleSteerDelivery.undelivered`). */
  undelivered?: (sessionId: string, path: string) => boolean;
  /** The runtime's side of a revised source's steers (`RuleSteerDelivery.revised`). */
  revised?: (sessionId: string, revision: number, paths: readonly string[]) => readonly string[];
} = {}) {
  const root = await mkdtemp(join(tmpdir(), "rerail-enforcer-"));
  roots.push(root);
  await mkdir(join(root, "api"));
  const capabilitiesPath = join(root, "capabilities.json");
  await writeFile(capabilitiesPath, await readFile(join(bundledCodeRuleDslDirectory(), "capabilities.json")));
  const rules = editableCodeRules(codeRules);
  const catalog = testCodeRuleCatalog(rules.list, capabilitiesPath);
  const requestSteer = vi.fn(async (_sessionId: string, _delivery: "steer", _content: string, _metadata?: SteerMetadata) => undefined);
  const enforcer = new NativeCodeRuleEnforcer(catalog, {
    workspaceRoot: root,
    steerDelivery: { requestSteer, ...(undelivered === undefined ? {} : { undelivered }), ...(revised === undefined ? {} : { revised }) },
    ...options,
  });
  return { root, catalog, enforcer, requestSteer, rules };
}


async function settled(enforcer: NativeCodeRuleEnforcer) {
  await expect.poll(() => enforcer.snapshotStatus().pending, { timeout: 15_000, interval: 10 }).toBe(false);
  return enforcer.snapshotStatus();
}

const gitCommit = ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"];

/**
 * A tree whose `api/` sources Git, `.rerailignore` and Rerail's built-in ignores would each hide from a Git listing: a
 * tracked file under `tmp/`, a gitignored untracked file, a file ignored by a nested repository, a `.rerailignore`d
 * tracked file. `build/` is a host-ignored directory. `tmp/ws` and `out/ws` (gitignored) are nested workspace roots.
 */
async function writeAuditTree(root: string, git: boolean): Promise<void> {
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
  execFileSync("git", ["add", "--", ".gitignore", "d.py"], { cwd: nested });
  execFileSync("git", gitCommit, { cwd: nested });
  execFileSync("git", ["init", "--quiet"], { cwd: root });
  execFileSync("git", ["add", "--", ".gitignore", ".rerailignore", "api/a.py", "api/tmp/b.py", "api/nested", "api/skip/f.py", "api/build/g.py", "tmp/ws/api/a.py"], { cwd: root });
  execFileSync("git", gitCommit, { cwd: root });
}

/**
 * One log-only repository audit of `workspaceRoot`, whose catalog lives elsewhere, following `.gitignore` or not: its
 * status, sorted violation paths and sorted `path code` failures.
 */
async function repositoryAudit(workspaceRoot: string, followGitignore = false): Promise<{ status: string | undefined; paths: string[]; failures: string[] }> {
  const home = await mkdtemp(join(tmpdir(), "rerail-scan-catalog-"));
  roots.push(home);
  const capabilitiesPath = join(home, "capabilities.json");
  await writeFile(capabilitiesPath, await readFile(join(bundledCodeRuleDslDirectory(), "capabilities.json")));
  const catalog = testCodeRuleCatalog([needleCodeRule(1)], capabilitiesPath);
  const { recorder, audits } = recordingAudits();
  const enforcer = new NativeCodeRuleEnforcer(catalog, {
    workspaceRoot,
    steerDelivery: { requestSteer: async () => undefined },
    audits: recorder,
    followGitignore,
  });
  try {
    await enforcer.openWorkspace("audit-parity");
    const [auditId] = await enforcer.audit("audit-parity", { scope: "repository", mode: "log" });
    const result = audits.get(auditId!)?.result;
    return {
      status: result?.status,
      paths: (result?.violations ?? []).map(violation => violation.path ?? "").toSorted(),
      failures: (result?.failures ?? []).map(failure => `${failure.path ?? ""} ${failure.code}`).toSorted(),
    };
  } finally {
    await enforcer.close();
  }
}

/** The sorted violation paths of one repository audit of `workspaceRoot` that must complete without failures. */
async function repositoryAuditPaths(workspaceRoot: string, followGitignore = false): Promise<string[]> {
  const result = await repositoryAudit(workspaceRoot, followGitignore);
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

/**
 * `directory`, whose listing fails with EIO once it has given `after` entries, as a listing on a network or FUSE
 * filesystem can; the names it gave go to `listed`.
 */
function failingListing(directory: Dir, { after, listed }: { after: number; listed: string[] }): Dir {
  async function* entries(): AsyncGenerator<Dirent> {
    try {
      for (let entry = await directory.read(); entry !== null; entry = await directory.read()) {
        if (listed.length === after) throw Object.assign(new Error("EIO: i/o error, readdir"), { code: "EIO" });
        listed.push(entry.name);
        yield entry;
      }
    } finally {
      await directory.close();
    }
  }
  return Object.defineProperty(directory, Symbol.asyncIterator, { value: entries });
}

/** `directory`, whose listing gives the entries `names` names in that order, as a filesystem that lists so would. */
function listingInOrder(directory: Dir, names: readonly string[]): Dir {
  async function* entries(): AsyncGenerator<Dirent> {
    try {
      const listed = new Map<string, Dirent>();
      for (let entry = await directory.read(); entry !== null; entry = await directory.read()) listed.set(entry.name, entry);
      for (const name of names) yield listed.get(name)!;
    } finally {
      await directory.close();
    }
  }
  return Object.defineProperty(directory, Symbol.asyncIterator, { value: entries });
}

/** Clean directories the shared content store records as complete, read in another process (`queryContentStore`). */
function completedDirectories(): number {
  const [row] = queryContentStore(process.env.RERAIL_CONTENT_STORE_PATH!, "SELECT count(*) AS n FROM completed_directory_reuse");
  return typeof row?.n === "number" ? row.n : Number.NaN;
}

/** Another process that takes the content store's write lock (`lock`) and gives it back (`release`) on request. */
interface StoreWriterHolder {
  request(command: "lock" | "release"): Promise<void>;
  /** Ends the process and settles once it has exited, so it holds neither the store nor its script's folder. */
  close(): Promise<void>;
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
    async close() {
      if (child.exitCode !== null || child.signalCode !== null) return;
      const exited = once(child, "exit");
      child.kill();
      await exited;
    },
  };
}

/**
 * Real native workspaces whose repository index takes `holder`'s store write lock once the index is done, so the
 * audit's reuse-cache write meets a busy store. `onCompletion` runs right after the enforcer starts that write.
 */
function holdStoreWriterForCompletion(holder: StoreWriterHolder, onCompletion: () => void) {
  const binding = native.loadNativeBinding();
  class Workspace {
    constructor() {
      const inner = new binding.CodeRuleDslWorkspace();
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
    .mockReturnValue(Object.assign(Object.create(binding) as NativeBinding, { CodeRuleDslWorkspace: Workspace }));
}

const repositoryIndexDone = z.object({ done: z.boolean() });

/** Real native workspaces whose inline apply of a request naming `path` throws, as an engine error would. */
function failingApply(path: string) {
  const binding = native.loadNativeBinding();
  class Workspace {
    constructor() {
      const inner = new binding.CodeRuleDslWorkspace();
      return new Proxy(inner, {
        get(target, key) {
          const value: unknown = Reflect.get(target, key, target);
          if (typeof value !== "function") return value;
          if (key !== "tryApply") return value.bind(target);
          return (request: string): unknown => {
            if (request.includes(JSON.stringify(path))) throw new Error(`injected evaluation failure for ${path}`);
            return Reflect.apply(value, target, [request]);
          };
        },
      });
    }
  }
  return vi
    .spyOn(native, "loadNativeBinding")
    .mockReturnValue(Object.assign(Object.create(binding) as NativeBinding, { CodeRuleDslWorkspace: Workspace }));
}

describe("Python module paths", () => {
  it("walks package __init__.py files the way Ruff does", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-modules-"));
    roots.push(root);
    await mkdir(join(root, "src/pkg/sub"), { recursive: true });
    await mkdir(join(root, "src/pkg/loose"), { recursive: true });
    await mkdir(join(root, "fn"));
    for (const init of ["src/pkg/__init__.py", "src/pkg/sub/__init__.py"]) await writeFile(join(root, init), "");
    expect(modulePath("python", join(root, "src/pkg/sub/mod.py"))).toBe("pkg.sub.mod");
    expect(modulePath("python", join(root, "src/pkg/__init__.py"))).toBe("pkg.__init__");
    expect(modulePath("python", join(root, "src/pkg/sub/types.pyi"))).toBe("pkg.sub.types");
    // A directory without __init__.py ends the package: the file is outside any package.
    expect(modulePath("python", join(root, "src/pkg/loose/mod.py"))).toBeUndefined();
    expect(modulePath("python", join(root, "fn/contextlib.py"))).toBeUndefined();
    // Only Python's family names modules: other dialects' files carry none, inside a package too.
    for (const [label, file] of [["typescript", "mod.ts"], ["tsx", "mod.tsx"], ["javascript", "mod.js"], ["php", "mod.php"], ["blade", "mod.blade.php"]] as const) {
      expect(modulePath(label, join(root, "src/pkg/sub", file)), label).toBeUndefined();
    }
  });

  it("sends names as on disk, as a list when one keeps a dot", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-modules-"));
    roots.push(root);
    await mkdir(join(root, "json.d"), { recursive: true });
    await mkdir(join(root, "good"), { recursive: true });
    for (const init of ["json.d/__init__.py", "good/__init__.py"]) await writeFile(join(root, init), "");
    // Ruff's N999 reads the package directory `json.d` and the stem `my.mod`; the engine derives `to_module_path`'s
    // stems (`json`) itself for qualified names.
    expect(modulePath("python", join(root, "json.d/__init__.py"))).toEqual(["json.d", "__init__"]);
    expect(modulePath("python", join(root, "json.d/m.py"))).toEqual(["json.d", "m"]);
    expect(modulePath("python", join(root, "good/my.mod.py"))).toEqual(["good", "my.mod"]);
    expect(modulePath("python", join(root, "good/fine.py"))).toBe("good.fine");
    expect(modulePath("python", join(root, "my.mod.py"))).toBeUndefined();
  });

  it("agrees with the module paths of the engine's Ruff rules oracle", async () => {
    const expected = JSON.parse(await readFile(fixturesPath("python", "scope", "ruff-rules.expected.json"), "utf8")) as {
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
        expect(modulePath("python", join(root, file.path)) ?? null, file.path).toEqual(file.module);
        checked += 1;
      }
    }
    expect(checked).toBeGreaterThan(40);
  });

  it("agrees with the module paths of the engine's Ruff oracle fixtures", async () => {
    const expected = JSON.parse(await readFile(fixturesPath("python", "scope", "modules.expected.json"), "utf8")) as { files: Array<{ path: string; module: string | null }> };
    for (const file of expected.files) {
      expect(modulePath("python", fixturesPath("python", "scope", "modules", file.path)) ?? null, file.path).toBe(file.module);
    }
  });
});

describe("mechanical code rule regressions", () => {
  it("reads each repository source once without spawning an initial index worker", async () => {
    const { root, enforcer } = await fixture();
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
      await enforcer.openWorkspace("one-read-repository-audit");
      vi.mocked(readFile).mockClear();
      await enforcer.audit("one-read-repository-audit", { scope: "repository", mode: "log" });
      expect(vi.mocked(readFile).mock.calls.filter(([path]) => path === source)).toHaveLength(1);
      expect(fork).not.toHaveBeenCalled();
    } finally {
      vi.mocked(fork).mockReset();
      vi.mocked(existsSync).mockReset();
      await enforcer.close();
    }
  });

  it("qualifies Python definitions by the module path of the edited file", async () => {
    const { recorder, audits } = recordingAudits();
    const codeRule = codeRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:module", revision: 1,
      mode: "steer",
      scope: { include: ["api/**/*.py"], exclude: [] },
      clauses: [{
        id: "suppress", violation: { code: "SUPPRESS", anchor: "witness.span", message: "suppress()" },
        forbid: { ast: { language: "python", rule: { kind: "identifier", resolves: { import: ["api.pkg.contextlib.suppress"] } } } },
      }],
    });
    const { root, enforcer } = await fixture({ audits: recorder, codeRules: [codeRule] });
    try {
      await mkdir(join(root, "api/pkg"));
      await writeFile(join(root, "api/__init__.py"), "");
      await writeFile(join(root, "api/pkg/__init__.py"), "");
      await enforcer.openWorkspace("module-path");
      await settled(enforcer);
      await enforcer.auditGeneratedEdits("module-path", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/pkg/contextlib.py", language: "python", generatedText: "def suppress(): pass\nwith suppress(): pass\n" }],
      });
      await settled(enforcer);
      expect(auditForPath(audits, "api/pkg/contextlib.py")?.result).toMatchObject({
        status: "completed",
        violations: [expect.objectContaining({ code: "SUPPRESS", span: expect.objectContaining({ startLine: 1 }) })],
        failures: [],
      });
    } finally { await enforcer.close(); }
  });

  it("retains interruption when closure overlaps writing accepted source to disk", async () => {
    const { root, enforcer } = await fixture();
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
      accepted = enforcer.auditGeneratedEdits("closing-during-storage", {
        revision: 1, checkpoints: ["edit"],
        edits: [{ path: "api/staging.py", language: "python", generatedText: "x".repeat(9 * 1024 * 1024) }],
      });
      await written.promise;
      closing = enforcer.close();
      release.resolve();
      await Promise.all([accepted, closing]);
      const audits = new SqliteAuditStore(defaultRerailDatabasePath());
      try {
        expect(audits.list({ sessionId: "closing-during-storage" }).audits).toEqual(expect.arrayContaining([
          expect.objectContaining({ paths: ["api/staging.py"], status: "interrupted", catalogRevision: null, ruleRevisions: [] }),
        ]));
      } finally { audits.close(); }
    } finally {
      release.resolve();
      await accepted;
      await closing;
      await enforcer.close();
      vi.mocked(writeFile).mockReset();
    }
  });

  it("records a failed audit when retained source cannot be loaded", async () => {
    const { recorder, audits } = recordingAudits();
    const { enforcer } = await fixture({ audits: recorder });
    try {
      await enforcer.openWorkspace("lost-retained-source");
      await settled(enforcer);
      vi.mocked(readFile).mockRejectedValueOnce(new Error("Retained source is unavailable"));
      await enforcer.auditGeneratedEdits("lost-retained-source", {
        revision: 1, checkpoints: ["edit"],
        edits: [{ path: "api/retained.py", language: "python", generatedText: "x".repeat(9 * 1024 * 1024) }],
      });
      await settled(enforcer);
      expect(auditForPath(audits, "api/retained.py")).toMatchObject({
        result: { status: "failed", failures: [expect.objectContaining({ message: "Retained source is unavailable" })] },
      });
    } finally {
      vi.mocked(readFile).mockReset();
      await enforcer.close();
    }
  });

  it("coalesces successive generated edits to the latest source without diagnostic interference", async () => {
    const { root, enforcer } = await fixture();
    try {
      await enforcer.openWorkspace("coalesce");
      await enforcer.auditGeneratedEdits("coalesce", { revision: 1, checkpoints: ["edit"], edits: [{ path: "api/a.py", language: "python", generatedText: "safe()\n" }] });
      await enforcer.auditGeneratedEdits("coalesce", { revision: 2, checkpoints: ["edit"], edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n" }] });
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      expect((await settled(enforcer)).violations).toEqual([expect.objectContaining({ path: "api/a.py", code: "NEEDLE" })]);
    } finally { await enforcer.close(); }
  });

  it("delivers violations only for the source scope that changed", async () => {
    const { root, enforcer, requestSteer } = await fixture();
    try {
      await writeFile(join(root, "api", "a.py"), "safe()\n");
      await writeFile(join(root, "api", "b.py"), "safe()\n");
      await enforcer.openWorkspace("scoped-delivery");

      await enforcer.auditGeneratedEdits("scoped-delivery", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n" }],
      });
      await settled(enforcer);
      await enforcer.auditGeneratedEdits("scoped-delivery", {
        revision: 2,
        checkpoints: ["edit"],
        edits: [{ path: "api/b.py", language: "python", generatedText: "safe(2)\n" }],
      });
      await settled(enforcer);
      await enforcer.auditGeneratedEdits("scoped-delivery", {
        revision: 3,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n# touched\n" }],
      });
      await settled(enforcer);

      expect(requestSteer).toHaveBeenCalledTimes(2);
      expect(requestSteer.mock.calls.map(([, , content]) => content)).toEqual([
        expect.stringContaining("api/a.py"),
        expect.stringContaining("api/a.py"),
      ]);
    } finally { await enforcer.close(); }
  });

  describe("a steer is asked for as its audit is recorded", () => {
    /**
     * An enforcer whose audit records and steer requests go, in the order they happen, to `order`. An audit keeps its
     * first result, as the audit store does.
     */
    async function ordered() {
      const order: string[] = [];
      const { recorder, audits } = recordingAudits();
      const made = await fixture({
        audits: {
          ...recorder,
          finish(auditId, result) {
            if (audits.get(auditId)!.result !== undefined) return;
            recorder.finish(auditId, result);
            order.push(`recorded ${audits.get(auditId)!.paths.join(",")}`);
          },
        },
      });
      made.requestSteer.mockImplementation(async (_sessionId, _delivery, _content, metadata) => {
        order.push(`steered ${metadata?.scope?.join(",") ?? ""}`);
        return undefined;
      });
      return { ...made, order, audits };
    }

    it("while another tool call's edit still waits for its check", async () => {
      const { root, enforcer, order } = await ordered();
      try {
        await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
        await writeFile(join(root, "api", "b.py"), "NEEDLE\n");
        await enforcer.openWorkspace("recorded-then-steered");
        // Both are queued before the evaluator takes the first, so the second waits behind the first's check.
        await Promise.all(["a", "b"].map((name, index) => enforcer.auditGeneratedEdits("recorded-then-steered", {
          revision: index + 1,
          checkpoints: ["edit"],
          edits: [{ path: `api/${name}.py`, language: "python", generatedText: "NEEDLE\n" }],
        })));
        await settled(enforcer);

        expect(order).toEqual(["recorded api/a.py", "steered api/a.py", "recorded api/b.py", "steered api/b.py"]);
      } finally { await enforcer.close(); }
    });

    /** A tool call of a.py and b.py, then another call's c.py, all queued before the evaluator takes a.py on the host's next turn. */
    async function queueTwoCalls(enforcer: NativeCodeRuleEnforcer, sessionId: string, b: CodeRuleEdit) {
      await enforcer.openWorkspace(sessionId);
      await enforcer.auditGeneratedEdits(sessionId, {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n" }, b],
      });
      await enforcer.auditGeneratedEdits(sessionId, {
        revision: 2,
        checkpoints: ["edit"],
        edits: [{ path: "api/c.py", language: "python", generatedText: "NEEDLE\n" }],
      });
    }

    async function twoCalls(enforcer: NativeCodeRuleEnforcer, sessionId: string, b: CodeRuleEdit) {
      await queueTwoCalls(enforcer, sessionId, b);
      await settled(enforcer);
    }

    it("when the tool call's other edit ends in a read failure, before another call's queued edit is recorded", async () => {
      const { root, enforcer, order } = await ordered();
      try {
        for (const name of ["a", "b", "c"]) await writeFile(join(root, "api", `${name}.py`), "NEEDLE\n");
        await twoCalls(enforcer, "read-failure-then-steered", { path: "api/b.py", language: "python", readError: "unreadable" });

        expect(order).toEqual(["recorded api/a.py", "recorded api/b.py", "steered api/a.py", "recorded api/c.py", "steered api/c.py"]);
      } finally { await enforcer.close(); }
    });

    it("when the tool call's other edit's evaluation fails, before another call's queued edit is recorded", async () => {
      const { root, enforcer, order } = await ordered();
      const binding = failingApply("api/b.py");
      try {
        for (const name of ["a", "b", "c"]) await writeFile(join(root, "api", `${name}.py`), "NEEDLE\n");
        await twoCalls(enforcer, "evaluation-error-then-steered", { path: "api/b.py", language: "python", generatedText: "NEEDLE\n" });

        expect(order).toEqual(["recorded api/a.py", "recorded api/b.py", "steered api/a.py", "recorded api/c.py", "steered api/c.py"]);
      } finally {
        binding.mockRestore();
        await enforcer.close();
      }
    });

    /** How b.py's audit ends in the two cases below, and the status it is recorded with. */
    const ends = [
      { id: "check", end: "its check", b: { path: "api/b.py", language: "python", generatedText: "NEEDLE\n" }, failing: false, status: "completed" },
      { id: "read-failure", end: "a read failure", b: { path: "api/b.py", language: "python", readError: "unreadable" }, failing: false, status: "failed" },
      { id: "evaluation-error", end: "an evaluation error", b: { path: "api/b.py", language: "python", generatedText: "NEEDLE\n" }, failing: true, status: "failed" },
    ] as const;

    it.each(ends)("records b.py's audit, ended by $end, before the steer it releases is answered", async ({ id, b, failing, status }) => {
      const { root, enforcer, order, requestSteer } = await ordered();
      const asked = Promise.withResolvers<void>();
      const answered = Promise.withResolvers<void>();
      // The runtime holds the first steer asked for, a.py's, until the test answers it.
      requestSteer.mockImplementationOnce(async (_sessionId, _delivery, _content, metadata) => {
        order.push(`steered ${metadata?.scope?.join(",") ?? ""}`);
        asked.resolve();
        await answered.promise;
        return undefined;
      });
      const binding = failing ? failingApply("api/b.py") : undefined;
      try {
        for (const name of ["a", "b", "c"]) await writeFile(join(root, "api", `${name}.py`), "NEEDLE\n");
        await queueTwoCalls(enforcer, `held-steer-${id}`, b);
        await asked.promise;
        const atRequest = { order: [...order], failed: enforcer.snapshotStatus().failures.some(item => item.path === "api/b.py") };
        answered.resolve();
        await settled(enforcer);

        expect(atRequest).toEqual({
          order: ["recorded api/a.py", "recorded api/b.py", expect.stringMatching(/^steered api\/a\.py/u)],
          failed: status === "failed",
        });
      } finally {
        answered.resolve();
        binding?.mockRestore();
        await enforcer.close();
      }
    });

    it.each(ends)("finishes b.py's audit, ended by $end, and still checks and steers c.py when the steer it releases fails", async ({ id, b, failing, status }) => {
      const { root, enforcer, order, requestSteer, audits } = await ordered();
      requestSteer.mockRejectedValueOnce(new Error("injected steer failure"));
      const binding = failing ? failingApply("api/b.py") : undefined;
      try {
        for (const name of ["a", "b", "c"]) await writeFile(join(root, "api", `${name}.py`), "NEEDLE\n");
        await queueTwoCalls(enforcer, `failed-steer-${id}`, b);
        const { failures } = await settled(enforcer);

        // The failed request is listed among the answer's failures, and the next queued edit is checked and steered.
        expect({
          b: auditForPath(audits, "api/b.py")?.result?.status,
          c: auditForPath(audits, "api/c.py")?.result?.status,
          steeredC: order.includes("steered api/c.py"),
          listed: failures.some(item => item.message.includes("injected steer failure")),
        }).toEqual({ b: status, c: "completed", steeredC: true, listed: true });
      } finally {
        binding?.mockRestore();
        await enforcer.close();
      }
    });

    it("once for a tool call's edits, after the last of them is recorded", async () => {
      const { root, enforcer, order } = await ordered();
      try {
        await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
        await writeFile(join(root, "api", "b.py"), "NEEDLE\n");
        await enforcer.openWorkspace("one-call-two-files");
        await enforcer.auditGeneratedEdits("one-call-two-files", {
          revision: 1,
          checkpoints: ["edit"],
          edits: ["a", "b"].map(name => ({ path: `api/${name}.py`, language: "python" as const, generatedText: "NEEDLE\n" })),
        });
        await settled(enforcer);

        expect(order).toEqual(["recorded api/a.py", "recorded api/b.py", "steered api/a.py,api/b.py"]);
      } finally { await enforcer.close(); }
    });

    it("and the evaluator still answers with the newest tool call's findings when an older call's go out after it", async () => {
      const { root, enforcer, order } = await ordered();
      try {
        for (const name of ["a", "b", "c"]) await writeFile(join(root, "api", `${name}.py`), "NEEDLE\n");
        await enforcer.openWorkspace("newest-answer");
        // The first call's second file is queued behind the second call's file, so the second call's steer goes first.
        await Promise.all([
          enforcer.auditGeneratedEdits("newest-answer", {
            revision: 1,
            checkpoints: ["edit"],
            edits: ["a", "b"].map(name => ({ path: `api/${name}.py`, language: "python" as const, generatedText: "NEEDLE\n" })),
          }),
          enforcer.auditGeneratedEdits("newest-answer", {
            revision: 2,
            checkpoints: ["edit"],
            edits: [{ path: "api/c.py", language: "python", generatedText: "NEEDLE\n" }],
          }),
        ]);
        const status = await settled(enforcer);

        expect(order.filter(entry => entry.startsWith("steered"))).toEqual(["steered api/c.py", "steered api/a.py,api/b.py"]);
        expect(status.violations.map(violation => violation.path)).toEqual(["api/c.py"]);
      } finally { await enforcer.close(); }
    });

    it("and keeps a repository audit's answer when a tool call queued before the scope changed goes out after it", async () => {
      const { root, enforcer } = await fixture({ scope: "touched-files", mode: "steer" });
      const actual = await vi.importActual<typeof FsPromises>("node:fs/promises");
      const spooling = Promise.withResolvers<void>();
      const stored = Promise.withResolvers<void>();
      let first: Promise<unknown> | undefined;
      try {
        for (const name of ["a", "r"]) await writeFile(join(root, "api", `${name}.py`), "NEEDLE\n");
        // The first call's large file is held as it is stored to disk, so the call's edits are queued only once the
        // repository audit of the next call has answered.
        vi.mocked(writeFile).mockImplementation(async (...args) => {
          if (String(args[0]).includes("rerail-policy-")) {
            spooling.resolve();
            await stored.promise;
          }
          return actual.writeFile(...args);
        });
        await enforcer.openWorkspace("repository-answer-kept");
        first = enforcer.auditGeneratedEdits("repository-answer-kept", {
          revision: 1,
          checkpoints: ["edit"],
          edits: [
            { path: "api/large.py", language: "python", generatedText: "x".repeat(9 * 1024 * 1024), readError: "unreadable" },
            { path: "api/a.py", language: "python", generatedText: "NEEDLE\n" },
          ],
        });
        await spooling.promise;
        enforcer.updateSettings({ scope: "repository", mode: "steer" });
        await enforcer.auditGeneratedEdits("repository-answer-kept", {
          revision: 2,
          checkpoints: ["edit"],
          edits: [{ path: "api/r.py", language: "python", generatedText: "NEEDLE\n" }],
        });
        await enforcer.settleGeneratedEdits();
        const repository = enforcer.snapshotStatus().violations.map(violation => violation.path).sort();
        stored.resolve();
        await first;
        const status = await settled(enforcer);

        expect(repository).toEqual(["api/a.py", "api/r.py"]);
        expect(status.violations.map(violation => violation.path).sort()).toEqual(repository);
      } finally {
        stored.resolve();
        await first;
        vi.mocked(writeFile).mockReset();
        await enforcer.close();
      }
    }, 60_000);

    it("and keeps the tool calls' answer when a scan of the repository ends outside repository scope", async () => {
      const { root, enforcer } = await fixture({ scope: "touched-files", mode: "steer" });
      try {
        for (const name of ["a", "r"]) await writeFile(join(root, "api", `${name}.py`), "NEEDLE\n");
        await enforcer.openWorkspace("scan-keeps-answer");
        await enforcer.auditGeneratedEdits("scan-keeps-answer", {
          revision: 1,
          checkpoints: ["edit"],
          edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n" }],
        });
        await settled(enforcer);
        await enforcer.audit("scan-keeps-answer", { scope: "repository", mode: "log" });
        const status = await settled(enforcer);

        expect(status.violations.map(violation => violation.path)).toEqual(["api/a.py"]);
      } finally { await enforcer.close(); }
    });
  });

  it("credits a steer to the distinct rules whose steer and block violations it rendered", async () => {
    const { root, enforcer, requestSteer } = await fixture({
      codeRules: [
        modeCodeRule("test:alpha", "ALPHA", "block"),
        modeCodeRule("test:beta", "BETA", "steer"),
        modeCodeRule("test:gamma", "GAMMA", "log"),
      ],
    });
    try {
      await writeFile(join(root, "api", "a.py"), "ALPHA\nBETA\nALPHA\nGAMMA\n");
      await enforcer.openWorkspace("rule-attribution");
      await enforcer.auditGeneratedEdits("rule-attribution", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "ALPHA\nBETA\nALPHA\nGAMMA\n" }],
      });
      const status = await settled(enforcer);

      expect(new Set(status.violations.map(violation => `${violation.ruleId}:${violation.mode}`)))
        .toEqual(new Set(["test:alpha:block", "test:beta:steer", "test:gamma:log"]));
      expect(status.disposition).toBe("steer");
      expect(requestSteer).toHaveBeenCalledTimes(1);
      const [, , content, metadata] = requestSteer.mock.calls[0]!;
      expect(metadata?.ruleIds).toEqual(["test:alpha", "test:beta"]);
      expect(content).toContain("ALPHA");
      expect(content).toContain("BETA");
      expect(content).not.toContain("GAMMA");
    } finally { await enforcer.close(); }
  });

  it("records a log rule's violations without a steer", async () => {
    const { root, enforcer, requestSteer } = await fixture({ codeRules: [modeCodeRule("test:gamma", "GAMMA", "log")] });
    try {
      await writeFile(join(root, "api", "a.py"), "GAMMA\n");
      await enforcer.openWorkspace("log-only");
      await enforcer.auditGeneratedEdits("log-only", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "GAMMA\n" }],
      });
      const status = await settled(enforcer);

      expect(status.violations).toEqual([expect.objectContaining({ ruleId: "test:gamma", mode: "log" })]);
      expect(status.disposition).toBe("continue");
      expect(requestSteer).not.toHaveBeenCalled();
    } finally { await enforcer.close(); }
  });

  it("evaluates a tool call's own edit before the log edits of a walk queued ahead of it", async () => {
    const { recorder, audits, finished } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    try {
      // What a walk of the other worktrees found, which steers no session, then the session's own edit.
      await logThenOwn(enforcer, root, { session: "steer-first", logged: LOGGED, own: ["api/own.py"] });
      await settled(enforcer);
      // The order each edit's check ended in: the own edit's at most behind the one log edit already under way.
      expect(finished.toSorted()).toEqual([...LOGGED, "api/own.py"].toSorted());
      expect(finished.indexOf("api/own.py")).toBeLessThan(2);
      // Each one checked, the log ones too.
      expect([...audits.values()].map(audit => audit.result?.status)).toEqual(finished.map(() => "completed"));
    } finally { await enforcer.close(); }
  });

  it("publishes a tool call's own edit once checked, before the log edits queued around it are", async () => {
    const { recorder, finished } = recordingAudits();
    const { root, enforcer, requestSteer } = await fixture({ audits: recorder });
    const loggedDoneAtSteer: number[] = [];
    requestSteer.mockImplementation(async () => { loggedDoneAtSteer.push(finished.filter(path => LOGGED.includes(path)).length); });
    try {
      await logThenOwn(enforcer, root, { session: "publish-own", logged: LOGGED, own: ["api/own.py"] });
      await settled(enforcer);
      // At most the one log edit under way as the own edit came was checked before its steer went out.
      expect(loggedDoneAtSteer).toHaveLength(1);
      expect(loggedDoneAtSteer[0]).toBeLessThan(2);
    } finally { await enforcer.close(); }
  });

  it("lets a tool call's result wait for its own edit's steer, not for the log edits queued around it", async () => {
    const { recorder, finished } = recordingAudits();
    const { root, enforcer, requestSteer } = await fixture({ audits: recorder });
    try {
      await logThenOwn(enforcer, root, { session: "settle-own", logged: LOGGED, own: ["api/own.py"] });
      await enforcer.settleGeneratedEdits("steering");
      expect(requestSteer).toHaveBeenCalledTimes(1);
      expect(finished.filter(path => LOGGED.includes(path)).length).toBeLessThan(2);
      // Without `steering` it waits for every edit, as before.
      await enforcer.settleGeneratedEdits();
      expect(finished.filter(path => LOGGED.includes(path))).toHaveLength(LOGGED.length);
    } finally { await enforcer.close(); }
  });

  it("answers with the newest revision's findings when an older log revision publishes after it", async () => {
    const { root, enforcer } = await fixture();
    try {
      // The log edits find NEEDLE too: published last, their revision would leave the answer at `continue`.
      await logThenOwn(enforcer, root, { session: "newest", logged: LOGGED, own: ["api/own.py"], loggedText: "NEEDLE\n" });
      const status = await settled(enforcer);
      expect(status.disposition).toBe("steer");
      expect(status.violations.map(violation => violation.path)).toEqual(["api/own.py"]);
    } finally { await enforcer.close(); }
  });

  it("publishes a revision's findings once and whole when log edits are checked between its files", async () => {
    const { root, enforcer, requestSteer } = await fixture();
    try {
      await logThenOwn(enforcer, root, { session: "whole", logged: LOGGED, own: ["api/own.py", "api/own2.py"] });
      await settled(enforcer);
      const steers = requestSteer.mock.calls.map(([, , , metadata]) => metadata);
      expect(steers.map(metadata => [metadata?.revision, metadata?.scope])).toEqual([[2, ["api/own.py", "api/own2.py"]]]);
    } finally { await enforcer.close(); }
  });

  it("begins the audits of the log edits a tool call's edit passes before its own, in the order they came", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    try {
      await logThenOwn(enforcer, root, { session: "log-order", logged: LOGGED, own: ["api/own.py"] });
      await settled(enforcer);
      // The Log lists audits by when they began: in the order the edits came, as when they were checked in that order.
      expect([...audits.values()].map(audit => audit.paths[0])).toEqual([...LOGGED, "api/own.py"]);
    } finally { await enforcer.close(); }
  });

  it("checks a tool call's edit with the log edits ahead of it, before their findings, where a rule counts matches across files", async () => {
    const { recorder, audits, finished } = recordingAudits();
    const { root, enforcer, requestSteer } = await fixture({ audits: recorder, codeRules: [needleBudgetRule()] });
    const loggedDoneAtSteer: number[] = [];
    requestSteer.mockImplementation(async () => { loggedDoneAtSteer.push(finished.filter(path => path !== "api/a_own.py").length); });
    const logged = [...LOGGED, "api/z_other.py"];
    const text = (path: string) => (path === "api/z_other.py" ? "NEEDLE\n" : "safe()\n");
    try {
      for (const path of logged) await writeFile(join(root, path), text(path));
      await writeFile(join(root, "api/a_own.py"), "NEEDLE\n");
      await enforcer.openWorkspace("cross-file");
      // A walk's log edits, the last another writer's NEEDLE, then the session's own: the count passes one only once both are held.
      await enforcer.auditGeneratedEdits("cross-file", {
        revision: 1, checkpoints: ["edit"], mode: "log", edits: logged.map(path => ({ path, language: "python" as const, generatedText: text(path) })),
      });
      await enforcer.auditGeneratedEdits("cross-file", {
        revision: 2, checkpoints: ["edit"], edits: [{ path: "api/a_own.py", language: "python", generatedText: "NEEDLE\n" }],
      });
      await settled(enforcer);
      // Checked with every log edit's text held, as in the order they came: the count's finding is its own.
      const steered = requestSteer.mock.calls.flatMap(([, , , metadata]) => metadata?.generatedEdit?.violations ?? []);
      expect(steered.map(violation => `${violation.code}@${violation.path}`)).toContain("BUDGET@api/a_own.py");
      // Its steer goes out before the log edits' checks end, but for the one under way as it came.
      expect(loggedDoneAtSteer).toHaveLength(1);
      expect(loggedDoneAtSteer[0]).toBeLessThan(2);
      // Every edit is checked, and the Log lists them in the order they came.
      expect([...audits.values()].map(audit => audit.paths[0])).toEqual([...logged, "api/a_own.py"]);
      expect([...audits.values()].map(audit => audit.result?.status)).toEqual([...logged, "own"].map(() => "completed"));
    } finally { await enforcer.close(); }
  });

  /** A NEEDLE reported where some file under api/ holds a witness of `exists`: the workspace's other files decide it. */
  function needleWhenRule(exists: Record<string, unknown>) {
    return codeRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:needle-when", revision: 1, mode: "block",
      scope: { include: ["api/**/*.py"], exclude: [] },
      clauses: [{
        id: "when", violation: { code: "WHEN", anchor: "witness.span", message: "No NEEDLE here" },
        forbid: { select_when: { select: { ast: { language: "python", rule: { kind: "identifier", regex: "^NEEDLE$" } } }, exists } },
      }],
    });
  }

  /**
   * The steers a check of `logged` (a walk's log edits) then the session's `api/a_own.py` asked for, as `code@path`;
   * `earlier`, a walk's log edits checked before both are queued.
   */
  async function steeredAfterLog(codeRule: CodeRule, session: string, edits: { earlier?: Record<string, string>; logged: Record<string, string>; own: string }): Promise<string[]> {
    const { root, enforcer, requestSteer } = await fixture({ codeRules: [codeRule] });
    const batch = (texts: Record<string, string>) => Object.entries(texts).map(([path, text]) => ({ path, language: "python" as const, generatedText: text }));
    try {
      for (const [path, text] of Object.entries(edits.logged)) await writeFile(join(root, path), text);
      await writeFile(join(root, "api/a_own.py"), edits.own);
      await enforcer.openWorkspace(session);
      await enforcer.auditGeneratedEdits(session, { revision: 1, checkpoints: ["edit"], mode: "log", edits: batch(edits.earlier ?? {}) });
      await settled(enforcer);
      await enforcer.auditGeneratedEdits(session, { revision: 2, checkpoints: ["edit"], mode: "log", edits: batch(edits.logged) });
      await enforcer.auditGeneratedEdits(session, { revision: 3, checkpoints: ["edit"], edits: [{ path: "api/a_own.py", language: "python", generatedText: edits.own }] });
      await settled(enforcer);
      return requestSteer.mock.calls.flatMap(([, , , metadata]) => metadata?.generatedEdit?.violations ?? []).map(violation => `${violation.code}@${violation.path}`);
    } finally { await enforcer.close(); }
  }

  // A `select_when` reads the whole workspace: the tool call's edit is checked with the log edits queued ahead of it held,
  // so it is steered as when the edits were checked in the order they came.
  it("steers a tool call's edit as in the order the edits came where a select_when's other side sits in a log edit ahead of it", async () => {
    const anchor = { ast: { language: "python", rule: { kind: "identifier", regex: "^ANCHOR$" } } };
    const logged = { ...Object.fromEntries(LOGGED.map(path => [path, "safe()\n"])), "api/z_other.py": "ANCHOR\n" };
    expect(await steeredAfterLog(needleWhenRule(anchor), "select-when-exists", { logged, own: "NEEDLE\n" })).toEqual(["WHEN@api/a_own.py"]);
  });

  it("steers no tool call's edit where a log edit ahead of it took away a select_when's last witness", async () => {
    // A witness is a file with no ANCHOR: z_other.py was one, and the log edit gives it an ANCHOR, as the own file has.
    const noAnchor = { ast: { language: "python", rule: { kind: "module", not: { has: { kind: "identifier", regex: "^ANCHOR$", stopBy: "end" } } } } };
    const edits = { earlier: { "api/z_other.py": "other = 1\n" }, logged: { "api/z_other.py": "ANCHOR\n" }, own: "NEEDLE\nANCHOR\n" };
    expect(await steeredAfterLog(needleWhenRule(noAnchor), "select-when-absent", edits)).toEqual([]);
  });

  it("lets a tool call's result wait for its own edit's steer, not for the log edits checked with it in one engine call", async () => {
    const { recorder, finished } = recordingAudits();
    const { root, enforcer, requestSteer } = await fixture({ audits: recorder, codeRules: [needleBudgetRule()] });
    const logged = [...LOGGED, "api/z_other.py"];
    const text = (path: string) => (path === "api/z_other.py" ? "NEEDLE\n" : "safe()\n");
    try {
      for (const path of logged) await writeFile(join(root, path), text(path));
      await writeFile(join(root, "api/a_own.py"), "NEEDLE\n");
      await enforcer.openWorkspace("settle-together");
      await enforcer.auditGeneratedEdits("settle-together", {
        revision: 1, checkpoints: ["edit"], mode: "log", edits: logged.map(path => ({ path, language: "python" as const, generatedText: text(path) })),
      });
      await enforcer.auditGeneratedEdits("settle-together", {
        revision: 2, checkpoints: ["edit"], edits: [{ path: "api/a_own.py", language: "python", generatedText: "NEEDLE\n" }],
      });
      await enforcer.settleGeneratedEdits("steering");
      expect(requestSteer).toHaveBeenCalledTimes(1);
      expect(finished.filter(path => logged.includes(path)).length).toBeLessThan(logged.length);
      await enforcer.settleGeneratedEdits();
      expect(finished.filter(path => logged.includes(path))).toHaveLength(logged.length);
    } finally { await enforcer.close(); }
  });

  // The one state where a steer can still come later than on main: another session's checks hold the engine past the
  // result's one-second wait. What may not get worse: the tool call's own finding is in the Log as soon as it is checked,
  // before its steer goes out and while the log edits queued ahead of it are still being checked.
  it("records a tool call's finding before its steer goes out, while the log edits queued ahead of it are still being checked", async () => {
    const { recorder, audits, finished } = recordingAudits();
    const { root, enforcer, requestSteer } = await fixture({ audits: recorder, codeRules: [needleBudgetRule()] });
    const atSteer: Array<{ own: string[]; loggedDone: number }> = [];
    requestSteer.mockImplementation(async () => {
      const own = auditForPath(audits, "api/a_own.py")?.result?.violations ?? [];
      atSteer.push({ own: own.map(violation => `${violation.code}@${violation.path}`), loggedDone: finished.filter(path => path !== "api/a_own.py").length });
    });
    const logged = [...LOGGED, "api/z_other.py"];
    const text = (path: string) => (path === "api/z_other.py" ? "NEEDLE\n" : "safe()\n");
    try {
      for (const path of logged) await writeFile(join(root, path), text(path));
      await writeFile(join(root, "api/a_own.py"), "NEEDLE\n");
      await enforcer.openWorkspace("recorded-first");
      await enforcer.auditGeneratedEdits("recorded-first", {
        revision: 1, checkpoints: ["edit"], mode: "log", edits: logged.map(path => ({ path, language: "python" as const, generatedText: text(path) })),
      });
      await enforcer.auditGeneratedEdits("recorded-first", {
        revision: 2, checkpoints: ["edit"], edits: [{ path: "api/a_own.py", language: "python", generatedText: "NEEDLE\n" }],
      });
      await settled(enforcer);
      expect(atSteer).toHaveLength(1);
      expect(atSteer[0]!.own).toContain("BUDGET@api/a_own.py");
      expect(atSteer[0]!.loggedDone).toBeLessThan(logged.length);
    } finally { await enforcer.close(); }
  });

  // A check of several whose tool-call edit cannot be read: the last log edit is read first in its place, and the
  // log revision's other edits after it. The session is answered with that revision once all of it is checked.
  it("answers a log revision whole when the tool call's edit checked with it in one engine call cannot be read", async () => {
    const { root, enforcer } = await fixture({ codeRules: [needleBudgetRule(), modeCodeRule("test:gamma", "GAMMA", "log")] });
    const logged = [...LOGGED, "api/z_other.py"];
    try {
      for (const path of logged) await writeFile(join(root, path), "GAMMA\n");
      await enforcer.openWorkspace("unreadable-own");
      await enforcer.auditGeneratedEdits("unreadable-own", {
        revision: 1, checkpoints: ["edit"], mode: "log", edits: logged.map(path => ({ path, language: "python" as const, generatedText: "GAMMA\n" })),
      });
      await enforcer.auditGeneratedEdits("unreadable-own", {
        revision: 2, checkpoints: ["edit"], edits: [{ path: "api/a_own.py", language: "python", readError: "EACCES: permission denied" }],
      });
      const status = await settled(enforcer);
      expect(status.failures.map(failure => failure.path)).toContain("api/a_own.py");
      expect(status.violations.map(violation => violation.path).toSorted()).toEqual(logged.toSorted());
    } finally { await enforcer.close(); }
  });

  // The same check of several, where reading one log edit's findings throws: that edit alone is recorded failed, the
  // edits after it are finished as with no throw, and the revision goes out once the last is recorded.
  it("fails only the log edit whose findings could not be read in a check of several, and answers with every other", async () => {
    const undelivered = (_sessionId: string, path: string): boolean => {
      if (path === "api/l2.py") throw new Error("the runtime could not answer");
      return false;
    };
    const { root, enforcer } = await fixture({ codeRules: [needleBudgetRule(), modeCodeRule("test:gamma", "GAMMA", "log")], undelivered });
    const logged = [...LOGGED, "api/z_other.py"];
    try {
      for (const path of logged) await writeFile(join(root, path), "GAMMA\n");
      await enforcer.openWorkspace("fails-partway");
      await enforcer.auditGeneratedEdits("fails-partway", {
        revision: 1, checkpoints: ["edit"], mode: "log", edits: logged.map(path => ({ path, language: "python" as const, generatedText: "GAMMA\n" })),
      });
      await enforcer.auditGeneratedEdits("fails-partway", {
        revision: 2, checkpoints: ["edit"], edits: [{ path: "api/a_own.py", language: "python", readError: "EACCES: permission denied" }],
      });
      const status = await settled(enforcer);
      // The last log edit is finished first in the unreadable edit's place, then the others in the order they came.
      expect(status.failures.map(failure => failure.path).toSorted()).toEqual(["api/a_own.py", "api/l2.py"]);
      expect(status.violations.map(violation => violation.path).toSorted()).toEqual(["api/l0.py", "api/l1.py", "api/l3.py", "api/l4.py", "api/z_other.py"]);
    } finally { await enforcer.close(); }
  });

  // A check of several where the runtime refuses the tool call's steer: as `#check` does for one edit, the failure is
  // listed on that edit, whose finding stands, and the log edits checked with it keep theirs.
  it("fails only the tool call's edit whose steer could not be published in a check of several, the log edits keeping their findings", async () => {
    const { recorder, audits } = firstResultAudits();
    const codeRules = [needleCodeRule(1), needleBudgetRule(), modeCodeRule("test:gamma", "GAMMA", "log")];
    const { root, enforcer, requestSteer } = await fixture({ audits: recorder, codeRules });
    requestSteer.mockRejectedValueOnce(new Error("injected steer failure"));
    const logged = [...LOGGED, "api/z_other.py"];
    try {
      for (const path of logged) await writeFile(join(root, path), "GAMMA\n");
      await writeFile(join(root, "api/a_own.py"), "NEEDLE\n");
      await enforcer.openWorkspace("steer-refused");
      await enforcer.auditGeneratedEdits("steer-refused", {
        revision: 1, checkpoints: ["edit"], mode: "log", edits: logged.map(path => ({ path, language: "python" as const, generatedText: "GAMMA\n" })),
      });
      await enforcer.auditGeneratedEdits("steer-refused", { revision: 2, checkpoints: ["edit"], edits: [{ path: "api/a_own.py", language: "python", generatedText: "NEEDLE\n" }] });
      const { failures } = await settled(enforcer);
      expect([...logged, "api/a_own.py"].map(path => auditForPath(audits, path)?.result?.status)).toEqual([...logged, "own"].map(() => "completed"));
      expect(auditForPath(audits, "api/l0.py")?.result?.violations?.map(violation => violation.code)).toEqual(["GAMMA"]);
      expect(failures.filter(item => item.message.includes("injected steer failure")).map(item => item.path)).toEqual(["api/a_own.py"]);
      // The evaluator goes on: the next tool call's edit is checked and steered.
      await enforcer.auditGeneratedEdits("steer-refused", { revision: 3, checkpoints: ["edit"], edits: [{ path: "api/b_next.py", language: "python", generatedText: "NEEDLE\n" }] });
      await settled(enforcer);
      expect(requestSteer.mock.calls.map(([, , , metadata]) => metadata?.revision)).toEqual([2, 3]);
    } finally { await enforcer.close(); }
  });

  // A check of several where one log edit's stored text cannot be loaded: that edit alone fails, as `#check` fails it,
  // and the others are checked in the same engine call.
  it("fails only the log edit whose stored text could not be loaded in a check of several", async () => {
    const { recorder, audits } = firstResultAudits();
    const { enforcer } = await fixture({ audits: recorder, codeRules: [needleCodeRule(1), needleBudgetRule()] });
    const logged = [...LOGGED, "api/z_big.py"];
    // Past the bytes the evaluator keeps in memory, so it is stored on disk, and read back as it is checked.
    const text = (path: string) => (path === "api/z_big.py" ? "x".repeat(9 * 1024 * 1024) : "safe()\n");
    try {
      await enforcer.openWorkspace("lost-in-several");
      await settled(enforcer);
      vi.mocked(readFile).mockRejectedValueOnce(new Error("Retained source is unavailable"));
      await enforcer.auditGeneratedEdits("lost-in-several", {
        revision: 1, checkpoints: ["edit"], mode: "log", edits: logged.map(path => ({ path, language: "python" as const, generatedText: text(path) })),
      });
      await enforcer.auditGeneratedEdits("lost-in-several", { revision: 2, checkpoints: ["edit"], edits: [{ path: "api/a_own.py", language: "python", generatedText: "NEEDLE\n" }] });
      await settled(enforcer);
      expect([...LOGGED, "api/a_own.py"].map(path => auditForPath(audits, path)?.result?.status)).toEqual([...LOGGED, "own"].map(() => "completed"));
      expect(auditForPath(audits, "api/z_big.py")).toMatchObject({
        result: { status: "failed", failures: [expect.objectContaining({ message: "Retained source is unavailable" })] },
      });
    } finally {
      vi.mocked(readFile).mockReset();
      await enforcer.close();
    }
  });

  // A check of several none of whose texts could be read ends the tool call's revision, whose first edit was checked
  // before: what that revision's publish throws is listed on the tool call's edit, as `#check` lists it.
  it("lists on the tool call's edit what its revision's publish throws when no edit of a check of several could be read", async () => {
    const { recorder, audits } = firstResultAudits();
    const { root, enforcer, requestSteer } = await fixture({ audits: recorder, codeRules: [needleCodeRule(1), needleBudgetRule()] });
    requestSteer.mockRejectedValueOnce(new Error("injected steer failure"));
    try {
      await writeFile(join(root, "api/own1.py"), "NEEDLE\n");
      await enforcer.openWorkspace("none-readable");
      // The tool call's two edits and a walk's log edit come at once: each is queued as it is stored, the log edit
      // between the two, so the second is checked with it, after the first.
      await Promise.all([
        enforcer.auditGeneratedEdits("none-readable", { revision: 1, checkpoints: ["edit"], edits: [
          { path: "api/own1.py", language: "python", generatedText: "NEEDLE\n" },
          { path: "api/own2.py", language: "python", readError: "EACCES: permission denied" },
        ] }),
        enforcer.auditGeneratedEdits("none-readable", {
          revision: 2, checkpoints: ["edit"], mode: "log", edits: [{ path: "api/l0.py", language: "python", readError: "EACCES: permission denied" }],
        }),
      ]);
      const { failures } = await settled(enforcer);
      expect([...audits.values()].map(audit => [audit.paths[0], audit.result?.status])).toEqual([
        ["api/own1.py", "completed"], ["api/l0.py", "failed"], ["api/own2.py", "failed"],
      ]);
      expect(failures.filter(item => item.message.includes("injected steer failure")).map(item => item.path)).toEqual(["api/own2.py"]);
    } finally { await enforcer.close(); }
  });

  // A publish that sends two revisions, the first refused by the runtime: the second goes out with the publish that
  // follows the failure's record, so the session is answered with the newest revision, as with the edits checked in order.
  it("answers with the newest revision when the runtime refuses an older one published in the same pass", async () => {
    const { root, enforcer, requestSteer } = await fixture({ codeRules: [needleCodeRule(1), needleBudgetRule(), modeCodeRule("test:gamma", "GAMMA", "log")] });
    requestSteer.mockRejectedValueOnce(new Error("injected steer failure"));
    try {
      await writeFile(join(root, "api/own1.py"), "NEEDLE\n");
      await writeFile(join(root, "api/l0.py"), "GAMMA\n");
      await enforcer.openWorkspace("two-revisions");
      // As above: the log edit is queued between the tool call's two, and checked with the second, which cannot be read.
      await Promise.all([
        enforcer.auditGeneratedEdits("two-revisions", { revision: 1, checkpoints: ["edit"], edits: [
          { path: "api/own1.py", language: "python", generatedText: "NEEDLE\n" },
          { path: "api/own2.py", language: "python", readError: "EACCES: permission denied" },
        ] }),
        enforcer.auditGeneratedEdits("two-revisions", { revision: 2, checkpoints: ["edit"], mode: "log", edits: [{ path: "api/l0.py", language: "python", generatedText: "GAMMA\n" }] }),
      ]);
      const status = await settled(enforcer);
      expect(requestSteer).toHaveBeenCalledTimes(1);
      expect(status.violations.map(violation => `${violation.code}@${violation.path}`)).toEqual(["GAMMA@api/l0.py"]);
    } finally { await enforcer.close(); }
  });

  // The last edit of a check of several, a walk's manifest edit: what it publishes for the source it refreshed throws
  // after its audit is recorded. The failure is listed on it, as on any edit, though no edit of the check is left.
  it("lists the failure of the last edit of a check of several when what it publishes throws after its audit is recorded", async () => {
    const { recorder, audits } = firstResultAudits();
    const requireRule = codeRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:require", revision: 1, mode: "steer", scope: { include: ["**/*.js"], exclude: [] },
      clauses: [
        { id: "require", violation: { code: "REQUIRE", anchor: "witness.span", message: "CommonJS require" }, forbid: { ast: { language: "typescript", languageOptions: {}, rule: { kind: "identifier", resolves: { global: ["require"] } } } } },
        { id: "budget", violation: { code: "BUDGET", anchor: "witness.span", message: "Too many files" }, budget: { query: { evidence: { as: "n", query: { ast: { language: "typescript", rule: { kind: "program" } } }, project: { path: "file.path" } } }, count: { lte: 100 } } },
      ],
    });
    const revised = () => { throw new Error("the runtime could not revise"); };
    const { root, enforcer } = await fixture({ audits: recorder, scope: "touched-files", codeRules: [requireRule], revised });
    try {
      await mkdir(join(root, "lib"));
      await writeFile(join(root, "package.json"), '{ "type": "commonjs" }\n');
      await writeFile(join(root, "lib/plain.js"), "module.exports = require('x');\n");
      await writeFile(join(root, "lib/own.js"), "export const own = 1;\n");
      await enforcer.openWorkspace("last-publish-throws");
      await enforcer.auditGeneratedEdits("last-publish-throws", { revision: 1, checkpoints: ["edit"], edits: [{ path: "lib/plain.js", language: "javascript", generatedText: "module.exports = require('x');\n" }] });
      await settled(enforcer);
      // A walk's manifest edit makes lib/plain.js a module: finished last, it reports that source again.
      await writeFile(join(root, "package.json"), '{ "type": "module" }\n');
      await enforcer.auditGeneratedEdits("last-publish-throws", { revision: 2, checkpoints: ["edit"], mode: "log", edits: [{ path: "package.json", language: "json", generatedText: '{ "type": "module" }\n' }] });
      await enforcer.auditGeneratedEdits("last-publish-throws", { revision: 3, checkpoints: ["edit"], edits: [{ path: "lib/own.js", language: "javascript", generatedText: "export const own = 1;\n" }] });
      const { failures } = await settled(enforcer);
      expect(["package.json", "lib/own.js"].map(path => auditForPath(audits, path)?.result?.status)).toEqual(["completed", "completed"]);
      expect(failures.filter(item => item.message.includes("could not revise")).map(item => item.path)).toEqual(["package.json"]);
    } finally { await enforcer.close(); }
  });

  // A tool call's edit is checked before the log edits queued ahead of it, whose audits begin first. Rules replaced
  // before their check: each audit names the rules that checked it.
  it("records the rules that check a log edit a tool call's edit passed over, when they were replaced before its check", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer, requestSteer, rules } = await fixture({ audits: recorder });
    // The tool call's steer goes out while the log edits it passed over still wait: the rules change then.
    requestSteer.mockImplementationOnce(async () => {
      rules.upsert(needleCodeRule(2, "FRESH", "FRESH"));
      enforcer.refreshCodeRules();
    });
    try {
      await logThenOwn(enforcer, root, { session: "passed-over-replaced", logged: LOGGED, own: ["api/own.py"], loggedText: "FRESH\n" });
      await settled(enforcer);
      const logAudits = [...audits.values()].filter(audit => LOGGED.includes(audit.paths[0]!));
      const checked = logAudits.filter(audit => audit.result?.status === "completed");
      // Each log edit is checked once; the one under way as the tool call came, if any, under the first rules.
      expect(checked.map(audit => audit.paths[0]).toSorted()).toEqual(LOGGED);
      expect(checked.filter(audit => audit.ruleRevisions[0]?.revision === 2).length).toBeGreaterThanOrEqual(LOGGED.length - 1);
      // The rules an audit names are the ones that found what it lists: revision 2 finds FRESH, revision 1 nothing here.
      expect(checked.map(audit => [audit.ruleRevisions[0]?.revision, audit.result?.violations?.map(violation => violation.code)]))
        .toEqual(checked.map(audit => (audit.ruleRevisions[0]?.revision === 2 ? [2, ["FRESH"]] : [1, []])));
      // An audit begun under the replaced rules and never checked by them ends as superseded.
      expect(logAudits.filter(audit => audit.result?.status !== "completed").map(audit => audit.result?.status)).toEqual(
        logAudits.filter(audit => audit.result?.status !== "completed").map(() => "superseded"));
    } finally { await enforcer.close(); }
  });

  it("limits AI-output reporting to disjoint changed ranges with UTF-8 byte offsets", async () => {
    const { enforcer, requestSteer } = await fixture({ scope: "ai-output" });
    try {
      await enforcer.openWorkspace("ranges");
      await enforcer.auditGeneratedEdits("ranges", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{
          path: "api/a.py",
          language: "python",
          previousText: "# π\nsafe_one()\nNEEDLE_old\nsafe_two()\n",
          generatedText: "# π\nNEEDLE_new_one\nNEEDLE_old\nNEEDLE_new_two\n",
        }],
      });
      expect((await settled(enforcer)).violations.map(violation => violation.span?.startLine)).toEqual([1, 3]);
      expect(requestSteer).toHaveBeenCalledTimes(1);
    } finally { await enforcer.close(); }
  });

  it("settles a repository-scope audit triggered by a generated edit", async () => {
    const { root, enforcer, requestSteer } = await fixture({ scope: "repository", mode: "steer" });
    try {
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      await enforcer.openWorkspace("repository-edit");
      await enforcer.auditGeneratedEdits("repository-edit", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n" }],
      });
      await enforcer.settleGeneratedEdits();
      expect(requestSteer).toHaveBeenCalledWith(
        "repository-edit",
        "steer",
        expect.stringContaining("NEEDLE"),
        expect.objectContaining({
          revision: 1,
          // The findings travel with the text, for the yield digest should the steer miss its step.
          generatedEdit: expect.objectContaining({ violations: [expect.objectContaining({ path: "api/a.py", ruleId: expect.any(String) })] }),
        }),
      );
    } finally { await enforcer.close(); }
  });

  // The dashboard counts the session's own audits, never what someone's `rerail scan` found in the checkout.
  it.each(["touched-files", "repository"] as const)("records the audits of edits as ongoing and those asked for as scans (%s scope)", async (scope) => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder, scope, mode: "steer" });
    try {
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      await enforcer.openWorkspace("audit-origin");
      await enforcer.auditGeneratedEdits("audit-origin", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n" }],
      });
      await enforcer.settleGeneratedEdits();
      const ongoing = [...audits.keys()];
      const [repositoryScan] = await enforcer.audit("audit-origin", { scope: "repository", mode: "log" });
      const [pathScan] = await enforcer.audit("audit-origin", { scope: "touched-files", mode: "log", paths: ["api/a.py"] });
      expect(ongoing.length).toBeGreaterThan(0);
      expect(ongoing.map(auditId => audits.get(auditId)?.origin)).toEqual(ongoing.map(() => "ongoing"));
      expect([repositoryScan, pathScan].map(auditId => audits.get(auditId!)?.origin)).toEqual(["scan", "scan"]);
      expect(audits.get(pathScan!)?.result?.violations).toEqual([expect.objectContaining({ path: "api/a.py", code: "NEEDLE" })]);
    } finally { await enforcer.close(); }
  });

  it("starts no repository audit for a write only a .rerail rule takes, in repository scope", async () => {
    const { recorder, audits } = recordingAudits();
    const root = await mkdtemp(join(tmpdir(), "rerail-enforcer-"));
    roots.push(root);
    const capabilitiesPath = join(root, "capabilities.json");
    await writeFile(capabilitiesPath, await readFile(join(bundledCodeRuleDslDirectory(), "capabilities.json")));
    const json = testCodeRuleCatalog([needleCodeRule(1)], capabilitiesPath);
    const rerailRule: RerailRuleEntry = {
      id: "rr-lib-rule", path: join(root, "rr-lib-rule.rerail"), placement: [{ folder: "", mode: "steer" }], files: [],
      scope: { include: ["lib/**/*.py"], exclude: [] }, language: "python",
      provenance: { ruleId: "rr-lib-rule", revision: 1, payloadHash: "hash", source: { kind: "pack", id: "rr", packId: "rr" }, repositoryKey: "repository", checkoutPath: root, placement: [] },
    };
    // The JSON rule reads `api/**`; the `.rerail` rule alone takes `lib/**`.
    const catalog: CodeRuleDslCatalog = {
      get status() { return json.status; },
      snapshot: () => ({ ...json.snapshot(), rerailRules: [rerailRule] }),
      provenance: () => json.provenance(),
      quarantined: () => json.quarantined(),
      refresh: () => json.refresh(),
    };
    const enforcer = new NativeCodeRuleEnforcer(catalog, {
      workspaceRoot: root, steerDelivery: { requestSteer: async () => undefined }, audits: recorder, scope: "repository", mode: "steer",
    });
    try {
      await mkdir(join(root, "lib"));
      await writeFile(join(root, "lib", "report.py"), "assert True\n");
      await enforcer.openWorkspace("rerail-only-write");
      expect(enforcer.acceptsPath("lib/report.py")).toBe(true);
      await enforcer.auditGeneratedEdits("rerail-only-write", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "lib/report.py", language: "python", generatedText: "assert True\n" }],
      });
      await enforcer.settleGeneratedEdits();
      expect([...audits.values()].filter(audit => audit.scope === "repository" && audit.origin === "ongoing")).toEqual([]);
    } finally { await enforcer.close(); }
  });

  it("audits a Git checkout through the native repository index and records its clean directories for reuse", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    try {
      await writeFile(join(root, ".gitignore"), ".rerail/\ncapabilities.json\napi/ignored.py\n");
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      await writeFile(join(root, "api", "binary.py"), "NEEDLE\0\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "--", ".gitignore", "api/a.py", "api/binary.py"], { cwd: root });
      execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
      await writeFile(join(root, "api", "ignored.py"), "NEEDLE\n");
      await enforcer.openWorkspace("git-repository-audit");
      const before = completedDirectories();
      const [auditId] = await enforcer.audit("git-repository-audit", { scope: "repository", mode: "log" });
      expect(audits.get(auditId!)?.result).toMatchObject({
        status: "completed",
        violations: [
          expect.objectContaining({ path: "api/a.py", code: "NEEDLE" }),
          expect.objectContaining({ path: "api/ignored.py", code: "NEEDLE" }),
        ],
        failures: [expect.objectContaining({ path: "api/binary.py", code: "source-incomplete", stage: "read" })],
      });
      expect(completedDirectories()).toBeGreaterThan(before);
    } finally { await enforcer.close(); }
  });

  it("completes a repository audit whose only failures are files with syntax errors or binary content, and reads every file past them", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder, codeRules: [consoleCodeRule(1)] });
    try {
      await mkdir(join(root, "web"));
      for (let index = 0; index < 25; index += 1) {
        await writeFile(join(root, "web", `broken${String(index).padStart(2, "0")}.ts`), "const x: = ;\nconsole.log(x);\n");
      }
      await writeFile(join(root, "web", "binary.ts"), "console.log(1)\0\n");
      await writeFile(join(root, "web", "zz-clean.ts"), "console.log(1);\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "web"], { cwd: root });
      execFileSync("git", gitCommit, { cwd: root });
      await enforcer.openWorkspace("syntax-only-failures");
      const [auditId] = await enforcer.audit("syntax-only-failures", { scope: "repository", mode: "log" });
      const result = audits.get(auditId!)?.result;
      expect(result?.status).toBe("completed");
      expect(result?.violations?.map((violation) => violation.path)).toContain("web/zz-clean.ts");
      expect(result?.failures?.map((failure) => failure.message)).not.toContainEqual(expect.stringContaining("stopped"));
      expect(result?.failures).toContainEqual(expect.objectContaining({ path: "web/binary.ts", code: "source-incomplete" }));
      expect(result?.failures).toContainEqual(expect.objectContaining({ path: "web/broken00.ts", code: "source-partial" }));
      // 26 files were read in part and 20 are listed: the rest are counted, which does not fail a completed audit.
      expect(result?.failures).toHaveLength(20);
      expect(result?.unlistedFailureCount).toBe(6);
    } finally { await enforcer.close(); }
  });

  it("fails a touched-files scan of a file with syntax errors, as it does not for a repository audit", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder, codeRules: [consoleCodeRule(1)] });
    try {
      await mkdir(join(root, "web"));
      await writeFile(join(root, "web", "broken.ts"), "const x: = ;\nconsole.log(x);\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "web"], { cwd: root });
      execFileSync("git", gitCommit, { cwd: root });
      await enforcer.openWorkspace("touched-syntax-errors");
      const [touchedId] = await enforcer.audit("touched-syntax-errors", { scope: "touched-files", mode: "log", paths: ["web/broken.ts"] });
      expect(audits.get(touchedId!)?.result?.failures).toContainEqual(expect.objectContaining({ path: "web/broken.ts", code: "source-partial" }));
      expect(audits.get(touchedId!)?.result?.status).toBe("failed");
      const [repositoryId] = await enforcer.audit("touched-syntax-errors", { scope: "repository", mode: "log" });
      expect(audits.get(repositoryId!)?.result?.failures).toContainEqual(expect.objectContaining({ path: "web/broken.ts", code: "source-partial" }));
      expect(audits.get(repositoryId!)?.result?.status).toBe("completed");
    } finally { await enforcer.close(); }
  });

  describe("what the native engine reports for a file it read only in part is a source gap", () => {
    /** A rule of the clause kind that cannot run around syntax errors: it resolves a name, which needs the whole file's scope. */
    function resolvesRule(id: string, language: "python" | "typescript", include: string, name: string) {
      return codeRuleSchema.parse({
        format: "rerail.policy/v1", id, revision: 1, mode: "block",
        scope: { include: [include], exclude: [] },
        clauses: [{
          id: "resolves", violation: { code: "RESOLVES", anchor: "witness.span", message: "Resolved" },
          forbid: { ast: { language, rule: { kind: "identifier", resolves: { global: [name] } } } },
        }],
      });
    }
    async function repositoryAuditFailures(files: Record<string, string | Buffer>, codeRules: readonly CodeRule[]) {
      const { recorder, audits } = recordingAudits();
      const { root, enforcer } = await fixture({ audits: recorder, codeRules });
      try {
        await mkdir(join(root, "web"));
        for (const [path, content] of Object.entries(files)) await writeFile(join(root, path), content);
        execFileSync("git", ["init", "--quiet"], { cwd: root });
        execFileSync("git", ["add", "--", "capabilities.json", ...Object.keys(files)], { cwd: root });
        execFileSync("git", gitCommit, { cwd: root });
        await enforcer.openWorkspace("source-gap-contract");
        const [auditId] = await enforcer.audit("source-gap-contract", { scope: "repository", mode: "log" });
        return audits.get(auditId!)?.result;
      } finally { await enforcer.close(); }
    }

    it("for a rule that needs the whole file, of a TypeScript or Python file with syntax errors", async () => {
      const result = await repositoryAuditFailures(
        { "api/broken.py": "print(1)\ndef (:\n", "web/broken.ts": "console.log(1);\nconst x: = ;\n" },
        [resolvesRule("test:python-resolves", "python", "api/**/*.py", "print"), resolvesRule("test:ts-resolves", "typescript", "web/**/*.ts", "console")],
      );
      for (const path of ["api/broken.py", "web/broken.ts"]) {
        const failure = result?.failures?.find(item => item.path === path);
        expect(failure, path).toMatchObject({ code: "source-incomplete", stage: "evaluate" });
        expect(isSourceGap(failure!), `${path}: ${failure?.message}`).toBe(true);
      }
      expect(result?.status).toBe("completed");
    });

    it("for a binary file the native repository index skips", async () => {
      const result = await repositoryAuditFailures({ "api/binary.py": "print(1)\0\n" }, [needleCodeRule(1)]);
      const failure = result?.failures?.find(item => item.path === "api/binary.py");
      expect(failure).toMatchObject({ code: "source-incomplete", stage: "read" });
      expect(isSourceGap(failure!), failure?.message).toBe(true);
      expect(result?.status).toBe("completed");
    });
  });

  it("indexes a Git checkout's files of every source language an audit evaluates", async () => {
    const { recorder, audits } = recordingAudits();
    const anywhere = codeRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:anywhere", revision: 1,
      mode: "steer",
      scope: { include: ["src/**/*"], exclude: [] },
      clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
    });
    const { root, enforcer } = await fixture({ audits: recorder, codeRules: [anywhere] });
    const paths = [...SOURCE_LANGUAGES.keys()].map((extension) => `src/file${extension}`).toSorted();
    try {
      await mkdir(join(root, "src"));
      for (const path of paths) await writeFile(join(root, path), "NEEDLE\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "src"], { cwd: root });
      execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
      await enforcer.openWorkspace("all-languages");
      const [auditId] = await enforcer.audit("all-languages", { scope: "repository", mode: "log" });
      expect(audits.get(auditId!)?.result?.violations?.map((violation) => violation.path).toSorted()).toEqual(paths);
    } finally { await enforcer.close(); }
  });

  it("records as failed each audit whose native repository index throws", async () => {
    const { recorder, audits } = recordingAudits();
    const real = native.loadNativeBinding();
    class Workspace {
      constructor() {
        const inner = new real.CodeRuleDslWorkspace();
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
      .mockReturnValue(Object.assign(Object.create(real) as NativeBinding, { CodeRuleDslWorkspace: Workspace }));
    const { root, enforcer } = await fixture({ audits: recorder });
    try {
      await writeFile(join(root, ".gitignore"), ".rerail/\ncapabilities.json\n");
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "--", ".gitignore", "api/a.py"], { cwd: root });
      execFileSync("git", gitCommit, { cwd: root });
      await enforcer.openWorkspace("failing-index");
      for (let scan = 0; scan < 2; scan += 1) {
        const [auditId] = await enforcer.audit("failing-index", { scope: "repository", mode: "log" });
        expect(auditId).toBeDefined();
        expect(audits.get(auditId!)).toMatchObject({
          scope: "repository",
          result: {
            status: "failed",
            violations: [],
            failures: [expect.objectContaining({ clauseId: "scan", code: "adapter-error", stage: "scan", message: "injected index failure" })],
          },
        });
      }
      expect(audits.size).toBe(2);
    } finally {
      binding.mockRestore();
      await enforcer.close();
    }
  });

  it("fails a repository audit when a file's source was released by a busy store: that is no file read in part", async () => {
    const { recorder, audits } = recordingAudits();
    const real = native.loadNativeBinding();
    const released = {
      ruleId: "test:needle", clauseId: "needle", code: "source-incomplete", path: "api/a.py",
      message: "source released while the content store was busy and no longer available",
    };
    class Workspace {
      constructor() {
        const inner = new real.CodeRuleDslWorkspace();
        return new Proxy(inner, {
          get(target, key) {
            const value: unknown = Reflect.get(target, key, target);
            if (typeof value !== "function") return value;
            if (key === "resultPageAsync")
              return async (cursor: number, bytes: number) => {
                const page = JSON.parse(await target.resultPageAsync(cursor, bytes)) as { failures: unknown[] };
                return JSON.stringify({ ...page, failures: [...page.failures, released] });
              };
            return value.bind(target);
          },
        });
      }
    }
    const binding = vi
      .spyOn(native, "loadNativeBinding")
      .mockReturnValue(Object.assign(Object.create(real) as NativeBinding, { CodeRuleDslWorkspace: Workspace }));
    const { root, enforcer } = await fixture({ audits: recorder });
    try {
      await writeFile(join(root, ".gitignore"), ".rerail/\ncapabilities.json\n");
      await writeFile(join(root, "api", "a.py"), "safe()\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "--", ".gitignore", "api/a.py"], { cwd: root });
      execFileSync("git", gitCommit, { cwd: root });
      await enforcer.openWorkspace("released-source");
      const [auditId] = await enforcer.audit("released-source", { scope: "repository", mode: "log" });
      expect(audits.get(auditId!)?.result).toMatchObject({
        status: "failed",
        failures: [expect.objectContaining({ path: "api/a.py", code: "source-incomplete", message: released.message })],
      });
    } finally {
      binding.mockRestore();
      await enforcer.close();
    }
  });

  it("lists a guardrail that stopped the scan after more read failures than the list holds", async () => {
    const { recorder, audits } = recordingAudits();
    const real = native.loadNativeBinding();
    // The index reports 25 unreadable files, then that the scan reached its file or byte budget.
    const unreadable = Array.from({ length: 25 }, (_, index) => ({
      path: `api/unreadable-${index}.py`, code: "budget-exceeded", message: "Source exceeds the per-file byte budget",
    }));
    class Workspace {
      constructor() {
        const inner = new real.CodeRuleDslWorkspace();
        return new Proxy(inner, {
          get(target, key) {
            const value: unknown = Reflect.get(target, key, target);
            if (typeof value !== "function") return value;
            if (key === "indexRepositoryAsync")
              return async (options: NativeRepositoryIndexOptions) => {
                const chunk = JSON.parse(await target.indexRepositoryAsync(options)) as Record<string, unknown>;
                return JSON.stringify({ ...chunk, done: true, overBudget: true, failures: unreadable });
              };
            return value.bind(target);
          },
        });
      }
    }
    const binding = vi
      .spyOn(native, "loadNativeBinding")
      .mockReturnValue(Object.assign(Object.create(real) as NativeBinding, { CodeRuleDslWorkspace: Workspace }));
    const { root, enforcer } = await fixture({ audits: recorder });
    try {
      await writeFile(join(root, ".gitignore"), ".rerail/\ncapabilities.json\n");
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "--", ".gitignore", "api/a.py"], { cwd: root });
      execFileSync("git", gitCommit, { cwd: root });
      await enforcer.openWorkspace("stopped-index");
      const [auditId] = await enforcer.audit("stopped-index", { scope: "repository", mode: "log" });
      expect(audits.get(auditId!)?.result).toMatchObject({
        status: "failed",
        failures: [
          ...Array.from({ length: 20 }, () => expect.objectContaining({ clauseId: "source", stage: "read" })),
          expect.objectContaining({
            clauseId: "scan", stage: "scan", message: "Repository scan exceeded its 100,000-file or 512 MiB source budget",
          }),
        ],
      });
    } finally {
      binding.mockRestore();
      await enforcer.close();
    }
  });

  it("spools oversized pending batches and reclaims the spool directory", async () => {
    const { enforcer } = await fixture();
    try {
      await enforcer.openWorkspace("spool");
      const source = `NEEDLE\n${"x".repeat(850_000)}`;
      const edits = Array.from({ length: 12 }, (_, index) => ({ path: `api/generated-${index}.py`, language: "python" as const, generatedText: source }));
      await enforcer.auditGeneratedEdits("spool", { revision: 1, checkpoints: ["edit"], edits });
      await settled(enforcer);
      expect((await settled(enforcer)).violations).toHaveLength(12);
    } finally { await enforcer.close(); }
  }, 60_000);

  it("removes its spool folder on close while Windows holds a file in it for a moment", async () => {
    const { enforcer } = await fixture();
    const actual = await vi.importActual<typeof FsPromises>("node:fs/promises");
    // The spool folder, not a spooled file in it.
    const refused = holdRemovals(vi.mocked(rm), actual.rm, 2, path => /rerail-policy-[^\\/]+$/u.test(path));
    try {
      await enforcer.openWorkspace("held-spool");
      const source = `NEEDLE\n${"x".repeat(850_000)}`;
      const edits = Array.from({ length: 12 }, (_, index) => ({ path: `api/generated-${index}.py`, language: "python" as const, generatedText: source }));
      await enforcer.auditGeneratedEdits("held-spool", { revision: 1, checkpoints: ["edit"], edits });
      await settled(enforcer);

      await enforcer.close();

      expect(refused).toHaveLength(2);
      expect(existsSync(refused[0]!)).toBe(false);
    } finally {
      vi.mocked(rm).mockReset();
      await enforcer.close();
    }
  }, 60_000);

  it("never evaluates a spooled edit a newer edit of its file superseded while the pump was running", async () => {
    const { recorder, audits } = recordingAudits();
    const { enforcer } = await fixture({ scope: "ai-output", audits: recorder });
    // Spool reads are told apart by the file their edit changes. The pump stops at its read of api/ahead.py, queued
    // before api/late.py, until the newer edit of api/late.py has looked for the older one in the queue (the queue's
    // read of the older edit back from the spool, or the newer edit's batch ending), so the pump cannot have taken the
    // older edit first. The queue's read then waits until the pump has read the older edit too (it is evaluating it)
    // or has gone idle without it. Each wait ends on an event the other side reaches, never on a state that may last.
    const looked = Promise.withResolvers<void>();
    const pumpReadOlder = Promise.withResolvers<void>();
    let pumpStopped = true;
    let older: string | undefined;
    const lookedForOlder = () => {
      pumpStopped = false;
      looked.resolve();
    };
    const hold = async (spool: string, path: string): Promise<void> => {
      if (path === "api/ahead.py") await looked.promise;
      if (path !== "api/late.py") return;
      if (spool === older) pumpReadOlder.resolve();
      else if (pumpStopped) {
        older = spool;
        lookedForOlder();
        await Promise.race([pumpReadOlder.promise, enforcer.settleGeneratedEdits()]);
      }
    };
    const reads = vi.spyOn(fsPromises, "readFile").mockImplementation(async (...args: Parameters<typeof fsPromises.readFile>) => {
      const path = String(args[0]);
      const handle = await fsPromises.open(path, "r");
      let text: Awaited<ReturnType<typeof handle.readFile>>;
      try {
        text = await handle.readFile(args[1] as Parameters<typeof handle.readFile>[0]);
      } finally {
        await handle.close();
      }
      if (path.includes("rerail-policy-")) await hold(path, z.object({ path: z.string() }).parse(JSON.parse(String(text))).path);
      return text;
    });
    try {
      await enforcer.openWorkspace("superseded-spool");
      // Eight megabytes evaluated and kept in memory, so each later edit of over 388 KB goes to the spool.
      const held = Array.from({ length: 8 }, (_, index) => ({ path: `api/held-${index}.py`, language: "python" as const, generatedText: "x".repeat(1_000_000) }));
      await enforcer.auditGeneratedEdits("superseded-spool", { revision: 1, checkpoints: ["edit"], edits: held });
      await settled(enforcer);
      const first = `NEEDLE\n${"x".repeat(400_000)}\n`;
      await enforcer.auditGeneratedEdits("superseded-spool", {
        revision: 2, checkpoints: ["edit"], edits: [
          { path: "api/ahead.py", language: "python", generatedText: "x".repeat(400_000) },
          { path: "api/late.py", language: "python", generatedText: first },
        ],
      });
      await enforcer.auditGeneratedEdits("superseded-spool", {
        revision: 3, checkpoints: ["edit"], edits: [{ path: "api/late.py", language: "python", previousText: first, generatedText: `${first}NEEDLE\n` }],
      }).finally(lookedForOlder);
      await settled(enforcer);
      const late = [...audits.values()].filter(audit => audit.paths.includes("api/late.py"));
      // The older edit's audit stays superseded; the newer reports both edits' lines, since the older was never reported.
      expect(late.map(audit => [audit.revision, audit.result?.status, audit.result?.violations?.length])).toEqual([[2, "superseded", 0], [3, "completed", 2]]);
    } finally {
      lookedForOlder();
      pumpReadOlder.resolve();
      reads.mockRestore();
      await enforcer.close();
    }
  }, 60_000);

  it("reports the whole file when the edit it overtook had no known earlier text", async () => {
    const { recorder, audits } = recordingAudits();
    const { enforcer } = await fixture({ scope: "ai-output", audits: recorder });
    try {
      await enforcer.openWorkspace("unknown-overtaken");
      // A shell write whose earlier text is unknown, superseded before it was checked by an edit that knows its own.
      const written = "NEEDLE\nsafe()\n";
      await enforcer.auditGeneratedEdits("unknown-overtaken", {
        revision: 1, checkpoints: ["edit"], edits: [{ path: "api/shell.py", language: "python", previousReadError: "Source changed outside a tool", generatedText: written }],
      });
      await enforcer.auditGeneratedEdits("unknown-overtaken", {
        revision: 2, checkpoints: ["edit"], edits: [{ path: "api/shell.py", language: "python", previousText: written, generatedText: `${written}ok()\n` }],
      });
      await settled(enforcer);
      const shell = [...audits.values()].filter(audit => audit.paths.includes("api/shell.py"));
      expect(shell.map(audit => [audit.revision, audit.result?.status, audit.result?.violations?.length])).toEqual([[1, "superseded", 0], [2, "completed", 1]]);
    } finally {
      await enforcer.close();
    }
  });

  it("evaluates TypeScript AST rules against React files with the JSX grammar", async () => {
    const { recorder, audits } = recordingAudits();
    const { enforcer } = await fixture({ audits: recorder, codeRules: [consoleCodeRule(1)] });
    try {
      await enforcer.openWorkspace("jsx-grammar");
      await settled(enforcer);
      await enforcer.auditGeneratedEdits("jsx-grammar", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{
          path: "web/App.tsx", language: "tsx",
          generatedText: "export function App() {\n  console.log('render');\n  return <main className=\"app\" />;\n}\n",
        }],
      });
      await settled(enforcer);

      expect(auditForPath(audits, "web/App.tsx")?.result).toMatchObject({
        status: "completed",
        violations: [expect.objectContaining({ code: "CONSOLE", path: "web/App.tsx" })],
        failures: [],
      });
    } finally { await enforcer.close(); }
  });

  it("steers with the anchored location, interpolated message, and suggested fix", async () => {
    const { recorder, audits } = recordingAudits();
    const fixing = codeRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:logger", revision: 1, mode: "block", title: "Log through the logger",
      scope: { include: ["web/**/*.tsx"], exclude: [] },
      clauses: [{
        id: "logger",
        violation: { code: "LOGGER", anchor: "capture.ARG", message: "Log $ARG through the logger", fix: "logger.info($ARG)" },
        forbid: { ast: { language: "typescript", rule: { pattern: "console.log($ARG)" } } },
      }],
    });
    // The steer's command is written for sh, as for an agent off Windows, on every machine.
    const { root, enforcer, requestSteer } = await fixture({ audits: recorder, codeRules: [fixing], steerShell: "posix" });
    try {
      const source = "export function App() {\n  console.log(user);\n  return <main />;\n}\n";
      await mkdir(join(root, "web"));
      await writeFile(join(root, "web", "App.tsx"), source);
      await enforcer.openWorkspace("fix-steer");
      await settled(enforcer);
      await enforcer.auditGeneratedEdits("fix-steer", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "web/App.tsx", language: "tsx", generatedText: source }],
      });
      await settled(enforcer);

      expect(requestSteer).toHaveBeenCalledTimes(1);
      expect(requestSteer.mock.calls[0]![2]).toBe([
        "Rerail found 1 problem in your last edit:",
        "- web/App.tsx:2:15 — Log user through the logger [test:logger]",
        "  Suggested fix: replace 2:3–2:20 with `logger.info(user)`",
        "Fix it, or tell the user in one line why you won't and run `rerail feedback test:logger '<that line>'`.",
      ].join("\n"));
      // Where the rule comes from, and its title, are for the person's line, never the agent's text.
      expect(requestSteer.mock.calls[0]![3]).toMatchObject({
        ruleIds: ["test:logger"],
        rules: [{ id: "test:logger", title: "Log through the logger", state: "steered", origin: "From your rule library." }],
      });
      expect(auditForPath(audits, "web/App.tsx")?.result).toMatchObject({
        violations: [expect.objectContaining({ fix: { span: expect.objectContaining({ startLine: 1, startColumn: 2 }), text: "logger.info(user)" } })],
      });
      // The fix is a suggestion only: the source on disk is untouched.
      expect(await readFile(join(root, "web", "App.tsx"), "utf8")).toBe(source);
    } finally { await enforcer.close(); }
  });

  it("records the replacement catalog that evaluates a queued edit", async () => {
    const { recorder, audits } = recordingAudits();
    const { catalog, enforcer, rules } = await fixture({ audits: recorder });
    try {
      await enforcer.openWorkspace("catalog-provenance");
      await settled(enforcer);

      await enforcer.auditGeneratedEdits("catalog-provenance", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/queued.py", language: "python", generatedText: "FRESH\n" }],
      });
            rules.upsert(needleCodeRule(2, "FRESH", "FRESH"));
      await enforcer.auditGeneratedEdits("catalog-provenance", {
        revision: 2,
        checkpoints: ["edit"],
        edits: [{ path: "api/refresh.py", language: "python", generatedText: "safe()\n" }],
      });
      await settled(enforcer);

      const audit = auditForPath(audits, "api/queued.py");
      expect(audit).toMatchObject({
        catalogRevision: catalog.status.version,
        ruleRevisions: [{ ruleId: "test:needle", revision: 2 }],
        result: { status: "completed", violations: [expect.objectContaining({ code: "FRESH", path: "api/queued.py" })] },
      });
    } finally { await enforcer.close(); }
  });

  it("installs the newest catalog when an older replacement finishes attaching storage last", async () => {
    const { recorder, audits } = recordingAudits();
    const { catalog, enforcer, rules } = await fixture({ audits: recorder });
    const attachments = gateStorageAttachments();
    try {
      attachments.gated = false;
      await enforcer.openWorkspace("newest-catalog");
      await settled(enforcer);
      attachments.gated = true;
            rules.upsert(needleCodeRule(2, "TWO", "TWO"));
      const older = enforcer.refreshCodeRules();
      rules.upsert(needleCodeRule(3, "THREE", "THREE"));
      const newer = enforcer.refreshCodeRules();
      const [, olderWorkspace, newerWorkspace] = attachments.workspaces;
      newerWorkspace!.release();
      await newerWorkspace!.attached;
      await newer;
      olderWorkspace!.release();
      await olderWorkspace!.attached;
      await older;

      await enforcer.auditGeneratedEdits("newest-catalog", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/both.py", language: "python", generatedText: "TWO\nTHREE\n" }],
      });
      await settled(enforcer);
      expect(auditForPath(audits, "api/both.py")).toMatchObject({
        catalogRevision: catalog.status.version,
        ruleRevisions: [{ ruleId: "test:needle", revision: 3 }],
        result: { status: "completed", violations: [expect.objectContaining({ code: "THREE", path: "api/both.py" })] },
      });
    } finally {
      for (const workspace of attachments.workspaces) workspace.release();
      attachments.restore();
      await enforcer.close();
    }
  });

  it("closes a workspace still attaching storage when the session closes during open", async () => {
    const { enforcer } = await fixture();
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
    }
  });

  it("closes a replacement still attaching storage when the session closes during a code rule refresh", async () => {
    const { enforcer, rules } = await fixture();
    const attachments = gateStorageAttachments();
    try {
      attachments.gated = false;
      await enforcer.openWorkspace("close-during-refresh");
      attachments.gated = true;
            rules.upsert(needleCodeRule(2, "FRESH", "FRESH"));
      const refreshing = enforcer.refreshCodeRules();
      await enforcer.close();
      for (const workspace of attachments.workspaces) workspace.release();
      await Promise.all(attachments.workspaces.map(workspace => workspace.attached));
      await refreshing;
      expect(attachments.workspaces.map(workspace => workspace.native.acceptsPath("api/a.py"))).toEqual([false, false]);
    } finally {
      for (const workspace of attachments.workspaces) workspace.release();
      attachments.restore();
      await enforcer.close();
    }
  });

  it("reads no Git checkout source in JavaScript during repository audits", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    const source = join(await realpath(root), "api", "once.py");
    await writeFile(source, "NEEDLE\n");
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    execFileSync("git", ["add", "--", "capabilities.json", "api/once.py"], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
    const reads = vi.spyOn(fsPromises, "readFile");
    try {
      await enforcer.openWorkspace("single-read");
      const [auditId] = await enforcer.audit("single-read", { scope: "repository", mode: "log" });
      expect(audits.get(auditId!)?.result?.violations).toEqual([expect.objectContaining({ path: "api/once.py", code: "NEEDLE" })]);
      expect(reads.mock.calls.filter(([path]) => path === source)).toHaveLength(0);
    } finally {
      reads.mockRestore();
      await enforcer.close();
    }
  });

  it("evaluates the files the walk selects, leaving out what Git ignores only when following .gitignore", async () => {
    const plain = await mkdtemp(join(tmpdir(), "rerail-scan-plain-"));
    const checkout = await mkdtemp(join(tmpdir(), "rerail-scan-git-"));
    roots.push(plain, checkout);
    await writeAuditTree(plain, false);
    await writeAuditTree(checkout, true);
    const walked = await repositoryAuditPaths(plain);
    expect(walked).toEqual(["api/a.py", "api/gen/c.py", "api/nested/d.py", "api/nested/e.py", "api/skip/f.py", "api/tmp/b.py"]);
    // Outside Git there is nothing to follow.
    expect(await repositoryAuditPaths(plain, true)).toEqual(walked);
    // `api/nested` holds its own repository in the Git tree; only the lane's owner can make it a checkout apart.
    expect(await repositoryAuditPaths(checkout)).toEqual(walked);
    // Following .gitignore leaves out the gitignored `api/gen/` and the file the nested `.gitignore` names; the
    // tracked `api/tmp/b.py` and the `.rerailignore`d `api/skip/f.py` stay, as Git lists them.
    expect(await repositoryAuditPaths(checkout, true)).toEqual(["api/a.py", "api/nested/d.py", "api/skip/f.py", "api/tmp/b.py"]);
    for (const workspace of ["tmp/ws", "out/ws"]) {
      expect(await repositoryAuditPaths(join(plain, workspace))).toEqual(["api/a.py"]);
      expect(await repositoryAuditPaths(join(plain, workspace), true)).toEqual(["api/a.py"]);
      expect(await repositoryAuditPaths(join(checkout, workspace))).toEqual(["api/a.py"]);
    }
    // A workspace in a folder the checkout around it ignores is ignored whole.
    expect(await repositoryAuditPaths(join(checkout, "tmp/ws"), true)).toEqual(["api/a.py"]);
    expect(await repositoryAuditPaths(join(checkout, "out/ws"), true)).toEqual([]);
  }, 60_000);

  it("tries a refused Git ignore matcher once and logs it once, however many paths ask", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-refused-ignore-"));
    roots.push(root);
    const access = nativeGitAccess();
    const opens = vi.spyOn(access, "openGitIgnoreMatcher").mockImplementation(() => {
      throw new Error("detected dubious ownership");
    });
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const enforcer = new NativeCodeRuleEnforcer(testCodeRuleCatalog([needleCodeRule(1)]), {
      workspaceRoot: root,
      steerDelivery: { requestSteer: async () => undefined },
      followGitignore: true,
    });
    try {
      expect(["a.py", "b.log", "build"].map(path => enforcer.ignoresPath(path, path === "build"))).toEqual([false, false, false]);
      expect(opens).toHaveBeenCalledTimes(1);
      expect(stderr.mock.calls.filter(([text]) => String(text).includes("cannot read Git ignore rules"))).toHaveLength(1);
    } finally {
      opens.mockRestore();
      stderr.mockRestore();
      await enforcer.close();
    }
  });

  it("opens the Git ignore matcher again once GIT_OPEN_AGAIN_MS has passed after an open that timed out, not at every path", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-timed-out-ignore-"));
    roots.push(root);
    const access = nativeGitAccess();
    const timedOut = Object.assign(new Error("opening Git's ignore rules gave no answer within 10 s"), { code: "GitReadTimedOut" });
    const ignoresLogs = { ignored: (path: string) => path.endsWith(".log"), ignoredBy: () => null, sourcesUnchanged: () => true };
    const opens = vi.spyOn(access, "openGitIgnoreMatcher").mockImplementationOnce(() => { throw timedOut; }).mockImplementation(() => ignoresLogs);
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    let clock = 1_000_000;
    const now = vi.spyOn(Date, "now").mockImplementation(() => clock);
    const enforcer = new NativeCodeRuleEnforcer(testCodeRuleCatalog([needleCodeRule(1)]), {
      workspaceRoot: root,
      steerDelivery: { requestSteer: async () => undefined },
      followGitignore: true,
    });
    try {
      expect(["a.log", "b.log", "c.log"].map(path => enforcer.ignoresPath(path, false))).toEqual([false, false, false]);
      expect(opens).toHaveBeenCalledTimes(1);
      clock += GIT_OPEN_AGAIN_MS;
      expect(["d.log", "e.py"].map(path => enforcer.ignoresPath(path, false))).toEqual([true, false]);
      expect(opens).toHaveBeenCalledTimes(2);
      expect(stderr.mock.calls.filter(([text]) => String(text).includes("gave no answer within"))).toHaveLength(1);
    } finally {
      now.mockRestore();
      opens.mockRestore();
      stderr.mockRestore();
      await enforcer.close();
    }
  });

  it("logs each distinct failure of a Git ignore question once", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-failing-ignore-"));
    roots.push(root);
    const access = nativeGitAccess();
    const failing = {
      ignored: (path: string) => { throw new Error(path.startsWith("locked/") ? "cannot read Git excludes for locked: Permission denied" : "cannot read Git index"); },
      ignoredBy: () => null,
      sourcesUnchanged: () => true,
    };
    const opens = vi.spyOn(access, "openGitIgnoreMatcher").mockImplementation(() => failing);
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const enforcer = new NativeCodeRuleEnforcer(testCodeRuleCatalog([needleCodeRule(1)]), {
      workspaceRoot: root,
      steerDelivery: { requestSteer: async () => undefined },
      followGitignore: true,
    });
    try {
      for (const path of ["locked/a.py", "locked/b.py", "c.py", "d.py"]) expect(enforcer.ignoresPath(path, false)).toBe(false);
      const logged = stderr.mock.calls.map(([text]) => String(text)).filter(text => text.includes("cannot read Git ignore rules"));
      expect(logged).toEqual([expect.stringContaining("Permission denied"), expect.stringContaining("cannot read Git index")]);
      expect(opens).toHaveBeenCalledTimes(1);
    } finally {
      opens.mockRestore();
      stderr.mockRestore();
      await enforcer.close();
    }
  });

  it("audits a workspace root reached through a symlink as its target, with or without Git", async () => {
    const plain = await mkdtemp(join(tmpdir(), "rerail-scan-plain-"));
    const checkout = await mkdtemp(join(tmpdir(), "rerail-scan-git-"));
    const links = await mkdtemp(join(tmpdir(), "rerail-scan-links-"));
    roots.push(plain, checkout, links);
    await writeAuditTree(plain, false);
    await writeAuditTree(checkout, true);
    linkFolder(plain, join(links, "plain"));
    linkFolder(checkout, join(links, "checkout"));
    const walked = await repositoryAuditPaths(plain);
    expect(walked).toContain("api/a.py");
    expect(await repositoryAuditPaths(join(links, "plain"))).toEqual(walked);
    expect(await repositoryAuditPaths(join(links, "checkout"))).toEqual(walked);
  }, 60_000);

  it("audits a Git checkout holding unreadable untracked directories as the walk does", async ({ skip }) => {
    skip(process.platform === "win32", "Windows ignores a folder's mode, so chmod cannot make a folder unreadable");
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
          execFileSync("git", ["add", "--", "api/a.py"], { cwd: root });
          execFileSync("git", gitCommit, { cwd: root });
        }
        // Untracked and not gitignored; `tmp/` is also outside Rerail's Git status.
        for (const directory of ["new", "tmp"]) restores.push(await unreadableDirectory(join(root, "api", directory)));
      }
      const walked = await repositoryAudit(plain);
      expect(walked).toEqual({
        status: "failed",
        paths: ["api/a.py"],
        failures: ["api/new source-incomplete", "api/tmp source-incomplete"],
      });
      expect(await repositoryAudit(checkout)).toEqual(walked);
    } finally {
      for (const restore of restores) await restore();
    }
  }, 60_000);

  it("audits the entries a folder listed before its listing failed partway, and reports the folder", async () => {
    const plain = await mkdtemp(join(tmpdir(), "rerail-scan-plain-"));
    roots.push(plain);
    await mkdir(join(plain, "api", "flaky"), { recursive: true });
    for (const name of ["a", "b", "c", "d", "e"]) await writeFile(join(plain, "api", "flaky", `${name}.py`), "NEEDLE\n");
    await writeFile(join(plain, "api", "top.py"), "NEEDLE\n");
    const flaky = join(await realpath(plain), "api", "flaky");
    const listed: string[] = [];
    // This file's `opendir` is the spy being mocked, and so is what `vi.importActual` returns: the listing opens through
    // `opendirSync`, as the spool test above reads through `open`.
    vi.mocked(opendir).mockImplementation(async (path, options) => {
      const directory = opendirSync(path, options);
      return String(path) === flaky ? failingListing(directory, { after: 2, listed }) : directory;
    });
    try {
      const result = await repositoryAudit(plain);
      expect(listed).toHaveLength(2);
      expect(result).toEqual({
        status: "failed",
        paths: [...listed.map(name => `api/flaky/${name}`), "api/top.py"].toSorted(),
        failures: ["api/flaky source-incomplete"],
      });
    } finally {
      vi.mocked(opendir).mockReset();
    }
  }, 60_000);

  it("stops a walk at its byte budget after the same files, the first in path order, whatever order a folder lists them in", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    const api = join(await realpath(root), "api");
    // One file more than the source budget holds, each just under the per-file budget. They are hard links to one
    // file, so the folder takes one file's disk and only its listing differs between the two audits.
    const size = MAX_SOURCE_BYTES - 1_000;
    const names = Array.from({ length: Math.floor(AUDIT_MAX_SOURCE_BYTES / size) + 1 }, (_, index) => `f${String(index).padStart(2, "0")}.py`);
    const source = Buffer.alloc(size, "#");
    source.write("NEEDLE\n");
    source[size - 1] = 0x0a;
    await writeFile(join(api, names[0]!), source);
    for (const name of names.slice(1)) await link(join(api, names[0]!), join(api, name));
    const reads = vi.spyOn(fsPromises, "readFile");
    /** One repository audit with `api` listed in the order of `listing`: the files it read, in order, and its result. */
    const audit = async (listing: readonly string[]) => {
      vi.mocked(opendir).mockImplementation(async (path, options) => {
        const directory = opendirSync(path, options);
        return String(path) === api ? listingInOrder(directory, listing) : directory;
      });
      reads.mockClear();
      const [auditId] = await enforcer.audit("byte-budget", { scope: "repository", mode: "log" });
      const result = audits.get(auditId!)?.result;
      return {
        status: result?.status,
        read: reads.mock.calls.map(([path]) => String(path)).filter(path => dirname(path) === api).map(path => basename(path)),
        violations: (result?.violations ?? []).map(violation => violation.path).toSorted(),
        failures: result?.failures,
      };
    };
    try {
      await enforcer.openWorkspace("byte-budget");
      // The walk reads the files in path order. The last one would pass the budget: the walk stops there and says so,
      // so the files before it are the ones audited, whichever order the folder was listed in.
      const stopped = {
        status: "failed",
        read: names,
        violations: names.slice(0, -1).map(name => `api/${name}`),
        failures: [expect.objectContaining({
          clauseId: "scan", code: "budget-exceeded", message: "Repository scan exceeded its 100,000-file or 512 MiB source budget",
        })],
      };
      await expect(audit(names)).resolves.toEqual(stopped);
      await expect(audit(names.toReversed())).resolves.toEqual(stopped);
    } finally {
      vi.mocked(opendir).mockReset();
      reads.mockRestore();
      await enforcer.close();
    }
  }, 120_000);

  it("evaluates generated edits in a Git checkout holding an unreadable untracked directory", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    let restore: () => Promise<void> = async () => undefined;
    try {
      await writeFile(join(root, "api", "a.py"), "safe()\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "--", "capabilities.json", "api/a.py"], { cwd: root });
      execFileSync("git", gitCommit, { cwd: root });
      restore = await unreadableDirectory(join(root, "api", "new"));
      await enforcer.openWorkspace("unreadable-untracked");
      await enforcer.auditGeneratedEdits("unreadable-untracked", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n" }],
      });
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      expect((await settled(enforcer)).violations).toEqual([expect.objectContaining({ path: "api/a.py", code: "NEEDLE" })]);
      expect(auditForPath(audits, "api/a.py")?.result).toMatchObject({ status: "completed", failures: [] });
    } finally {
      await restore();
      await enforcer.close();
    }
  });

  it("records a Git checkout's clean directories while another process holds the store writer", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    const holder = await storeWriterHolder(process.env.RERAIL_CONTENT_STORE_PATH!);
    // Only a JavaScript thread left free while the reuse-cache write waits can hand the lock back.
    const binding = holdStoreWriterForCompletion(holder, () =>
      queueHostTurn(() => void holder.request("release").catch(() => undefined)));
    try {
      await writeFile(join(root, ".gitignore"), ".rerail/\ncapabilities.json\n");
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "--", ".gitignore", "api/a.py"], { cwd: root });
      execFileSync("git", gitCommit, { cwd: root });
      await enforcer.openWorkspace("busy-store-completion");
      const before = completedDirectories();
      const [auditId] = await enforcer.audit("busy-store-completion", { scope: "repository", mode: "log" });
      expect(audits.get(auditId!)?.result).toMatchObject({
        status: "completed",
        violations: [expect.objectContaining({ path: "api/a.py", code: "NEEDLE" })],
        failures: [],
      });
      expect(completedDirectories()).toBeGreaterThan(before);
    } finally {
      binding.mockRestore();
      await holder.close();
      await enforcer.close();
    }
  }, 30_000);

  it("opens the workspace and evaluates edits while another process holds the store writer for the whole attach", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    const holder = await storeWriterHolder(process.env.RERAIL_CONTENT_STORE_PATH!);
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      await writeFile(join(root, ".gitignore"), ".rerail/\ncapabilities.json\n");
      await writeFile(join(root, "api", "a.py"), "safe()\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "--", ".gitignore", "api/a.py"], { cwd: root });
      execFileSync("git", gitCommit, { cwd: root });
      // The store is a cache: an attach that waits out its write deadline runs without reuse instead of failing.
      await holder.request("lock");
      await enforcer.openWorkspace("busy-store-attach");
      await enforcer.auditGeneratedEdits("busy-store-attach", {
        revision: 1,
        checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "NEEDLE\n" }],
      });
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      expect((await settled(enforcer)).violations).toEqual([expect.objectContaining({ path: "api/a.py", code: "NEEDLE" })]);
      expect(auditForPath(audits, "api/a.py")?.result).toMatchObject({ status: "completed", failures: [] });
      const failures = [...audits.values()].flatMap(audit => audit.result?.failures ?? []);
      expect(failures).not.toContainEqual(expect.objectContaining({ clauseId: "catalog" }));
      const reports = stderr.mock.calls.map(([chunk]) => String(chunk)).filter(line => line.includes("repository reuse"));
      expect(reports).toEqual([expect.stringMatching(/^rerail: repository reuse is disabled: content store busy: /u)]);
    } finally {
      stderr.mockRestore();
      await holder.request("release").catch(() => undefined);
      await holder.close();
      await enforcer.close();
    }
  }, 60_000);

  it("keeps a finished audit's violations when its reuse-cache write fails", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    const holder = await storeWriterHolder(process.env.RERAIL_CONTENT_STORE_PATH!);
    const binding = holdStoreWriterForCompletion(holder, () => undefined);
    try {
      await writeFile(join(root, ".gitignore"), ".rerail/\ncapabilities.json\n");
      await writeFile(join(root, "api", "a.py"), "NEEDLE\n");
      execFileSync("git", ["init", "--quiet"], { cwd: root });
      execFileSync("git", ["add", "--", ".gitignore", "api/a.py"], { cwd: root });
      execFileSync("git", gitCommit, { cwd: root });
      await enforcer.openWorkspace("failed-completion");
      const before = completedDirectories();
      const [auditId] = await enforcer.audit("failed-completion", { scope: "repository", mode: "log" });
      await holder.request("release");
      expect(completedDirectories()).toBe(before);
      expect(audits.get(auditId!)?.result).toMatchObject({
        status: "completed",
        violations: [expect.objectContaining({ path: "api/a.py", code: "NEEDLE" })],
        failures: [],
      });
    } finally {
      binding.mockRestore();
      await holder.close();
      await enforcer.close();
    }
  }, 30_000);

  it("closes a workspace whose storage attach throws before it starts", async () => {
    const { enforcer } = await fixture();
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
    }
  });

  it("reports once why repository reuse is disabled when the storage attach says why", async () => {
    const { recorder, audits } = recordingAudits();
    const { enforcer } = await fixture({ audits: recorder });
    const reason = "cannot read .rerailignore: Is a directory (os error 21)";
    const unattached = () => Object.assign(pagingWorkspace((cursor) => openPage(cursor, 1)), {
      enableStorageAsync: async () => ({ attached: false, reason, discoveryMilliseconds: 0, publicationMilliseconds: 0 }),
    });
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const binding = nativeWorkspaces(unattached(), unattached(), unattached());
    try {
      await enforcer.openWorkspace("unattached-reuse");
      for (let scan = 0; scan < 2; scan += 1) {
        const [auditId] = await enforcer.audit("unattached-reuse", { scope: "repository", mode: "log" });
        expect(audits.get(auditId!)?.result?.status).toBe("completed");
      }
      const reports = stderr.mock.calls.map(([chunk]) => String(chunk)).filter((line) => line.includes("repository reuse"));
      expect(reports).toEqual([`rerail: repository reuse is disabled: ${reason}\n`]);
    } finally {
      stderr.mockRestore();
      binding.mockRestore();
      await enforcer.close();
    }
  });

  it("writes how long each storage attach took, split into discovery and publication, to the timing log", async () => {
    const lines: string[] = [];
    const { enforcer } = await fixture({ timingLog: line => lines.push(line) });
    const timed = (attached: boolean) => Object.assign(pagingWorkspace((cursor) => openPage(cursor, 1)), {
      enableStorageAsync: async () => ({ attached, discoveryMilliseconds: 12, publicationMilliseconds: 34 }),
    });
    const binding = nativeWorkspaces(timed(true), timed(false));
    try {
      await enforcer.openWorkspace("timed-attach");
      await enforcer.audit("timed-attach", { scope: "touched-files", mode: "log", paths: ["api"] });
      expect(lines.filter((line) => line.startsWith("rerail: repository storage"))).toEqual([
        expect.stringMatching(/^rerail: repository storage attached in \d+ ms \(discovery 12 ms, publication 34 ms\)\n$/u),
        expect.stringMatching(/^rerail: repository storage not attached in \d+ ms \(discovery 12 ms, publication 34 ms\)\n$/u),
      ]);
    } finally {
      binding.mockRestore();
      await enforcer.close();
    }
  });

  it("attaches the content store with the secret-like names it must never hold", async () => {
    const { enforcer } = await fixture();
    const passed: unknown[] = [];
    const workspace = Object.assign(pagingWorkspace((cursor) => openPage(cursor, 1)), {
      enableStorageAsync: async (_root: string, _storePath: string, secretNames: unknown) => {
        passed.push(secretNames);
        return { attached: true, discoveryMilliseconds: 0, publicationMilliseconds: 0 };
      },
    });
    const binding = nativeWorkspaces(workspace);
    try {
      await enforcer.openWorkspace("secret-names");
      expect(passed).toEqual([{ files: SENSITIVE_FILE_NAMES, folders: SENSITIVE_FOLDER_NAMES }]);
    } finally {
      binding.mockRestore();
      await enforcer.close();
    }
  });

  it("writes one line per repository audit with its phase times, files, violations, failures and status", async () => {
    const { recorder } = recordingAudits();
    const lines: string[] = [];
    const { root, enforcer } = await fixture({ audits: recorder, timingLog: line => lines.push(line) });
    await writeFile(join(root, "api", "once.py"), "NEEDLE\n");
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    execFileSync("git", ["add", "--", "capabilities.json", "api/once.py"], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
    try {
      await enforcer.openWorkspace("timed-audit");
      await enforcer.audit("timed-audit", { scope: "touched-files", mode: "log", paths: ["api/once.py"] });
      expect(lines.filter((line) => line.startsWith("rerail: repository audit"))).toEqual([]);
      await enforcer.audit("timed-audit", { scope: "repository", mode: "log" });
      expect(lines.filter((line) => line.startsWith("rerail: repository audit"))).toEqual([
        expect.stringMatching(/^rerail: repository audit completed in \d+ ms \(index \d+ ms, result pages \d+ ms\): [1-9]\d* files, 1 violations, 0 failures\n$/u),
      ]);
    } finally {
      await enforcer.close();
    }
  }, 30_000);

  it("writes no timing line to stderr unless it is given a timing log", async () => {
    const { recorder } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    await writeFile(join(root, "api", "once.py"), "NEEDLE\n");
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    execFileSync("git", ["add", "--", "capabilities.json", "api/once.py"], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      await enforcer.openWorkspace("untimed");
      await enforcer.audit("untimed", { scope: "repository", mode: "log" });
      expect(stderr.mock.calls.map(([chunk]) => String(chunk)).filter((line) => line.startsWith("rerail: repository"))).toEqual([]);
    } finally {
      stderr.mockRestore();
      await enforcer.close();
    }
  }, 30_000);

  it("ends a result's wait for the steering edits when the evaluator stops before checking them", async () => {
    const { enforcer, rules } = await fixture();
    const unpaged = () => { throw new Error("the workspace is never paged"); };
    const cannotApply = async () => { throw new Error("the replacement cannot apply"); };
    const failing = Object.assign(pagingWorkspace(unpaged), { applyAsync: cannotApply, applyBufferAsync: cannotApply });
    const binding = nativeWorkspaces(pagingWorkspace(unpaged), failing);
    try {
      await enforcer.openWorkspace("pump-fails");
      // A rule change makes the next check replace the workspace first, and the replacement fails: no edit is checked.
      rules.upsert(needleCodeRule(2, "TWO", "TWO"));
      await enforcer.auditGeneratedEdits("pump-fails", {
        revision: 1, checkpoints: ["edit"], edits: [{ path: "api/own.py", language: "python", generatedText: "NEEDLE\n" }],
      });
      const first = await Promise.race([
        enforcer.settleGeneratedEdits("steering").then(() => "the wait ended"),
        settled(enforcer).then(() => "the evaluator stopped"),
      ]);
      expect(first).toBe("the wait ended");
    } finally {
      binding.mockRestore();
      await enforcer.close();
    }
  });

  it("never applies to a replacement that a newer catalog superseded while it replayed", async () => {
    const { enforcer, rules } = await fixture();
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
                    rules.upsert(needleCodeRule(3, "THREE", "THREE"));
          enforcer.refreshCodeRules();
        }
        return completedApply;
      },
      close: () => { supersededClosed = true; },
    });
    const binding = nativeWorkspaces(pagingWorkspace(unpaged), superseded, pagingWorkspace(unpaged));
    try {
      await enforcer.openWorkspace("superseded-replay");
      await enforcer.auditGeneratedEdits("superseded-replay", {
        revision: 1, checkpoints: ["edit"],
        edits: [{ path: "api/a.py", language: "python", generatedText: "safe()\n" }],
      });
      await settled(enforcer);
            rules.upsert(needleCodeRule(2, "TWO", "TWO"));
      await enforcer.auditGeneratedEdits("superseded-replay", {
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
    }
  });
});

const completedApply = JSON.stringify({
  revision: 1,
  evaluation: { status: "completed", violations: [], failures: [], coverage: [], metrics: { files: 0, visitedNodes: 0, witnesses: 0 } },
});

/** A native workspace double that serves result pages from `page` and reports each apply to `onApply`. */
function pagingWorkspace(page: (cursor: number) => string, onApply: () => void = () => undefined): NativeCodeRuleDslWorkspace {
  return {
    enableStorageAsync: async () => ({ attached: true, discoveryMilliseconds: 0, publicationMilliseconds: 0 }), addCodeRule: () => undefined, acceptsPath: () => true, acceptsTextPath: () => false, readsServerGraph: () => false, readsPackageManifests: () => false,
    tryApply: () => null,
    applyAsync: async () => { onApply(); return completedApply; },
    applyBufferAsync: async () => { onApply(); return completedApply; },
    memoryProfile: () => JSON.stringify({}),
    resultPageAsync: async (cursor: number) => page(cursor),
    evictInactive: () => false, nodeReference: () => null,
    resolveNodeReference: () => JSON.stringify(null), setDocumentResidencyLimit: () => undefined, setRoot: () => undefined, close: () => undefined,
  } as unknown as NativeCodeRuleDslWorkspace;
}

/** Serves the given workspaces, in order, to each `new CodeRuleDslWorkspace()`; code rules still compile natively. */
function nativeWorkspaces(...workspaces: NativeCodeRuleDslWorkspace[]) {
  const binding = native.loadNativeBinding();
  return vi.spyOn(native, "loadNativeBinding").mockReturnValue(Object.assign(Object.create(binding) as NativeBinding, {
    modulePath: () => null,
    CodeRuleDslWorkspace: class {
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
  it("applies session edits while an audit pages its private workspace", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
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
          edit = enforcer.auditGeneratedEdits("page-session", {
            revision: 1, checkpoints: ["edit"],
            edits: [{ path: "api/apply.py", language: "python", generatedText: "safe()\n" }],
          });
        });
      // The audit keeps paging until the session edit reaches its workspace (or the page limit).
      return openPage(cursor, limit, sessionApplyPage !== undefined);
    });
    const binding = nativeWorkspaces(session, scan);
    try {
      await writeFile(join(root, "api", "page.py"), "safe()\n");
      await enforcer.openWorkspace("page-session");
      await enforcer.audit("page-session", { scope: "touched-files", mode: "log", paths: ["api/page.py"] });
      await edit;
      await enforcer.settleGeneratedEdits();
      expect(sessionApplyPage).toBeDefined();
      expect(sessionApplyPage!).toBeLessThan(limit);
      expect(pages.length).toBeLessThan(limit);
      expect(audits.get("run-1")?.result?.status).toBe("completed");
    } finally {
      binding.mockRestore();
      await enforcer.close();
    }
  });

  it("stops paging once the enforcer is interrupted", async () => {
    const { root, enforcer } = await fixture({ audits: recordingAudits().recorder });
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
      await enforcer.audit("page-interrupt", { scope: "touched-files", mode: "log", paths: ["api/page.py"] });
      expect(pages).toEqual([0]);
    } finally {
      binding.mockRestore();
      await enforcer.close();
    }
  });

  it("yields to the host before fetching the next result page", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
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
      await enforcer.audit("page-yield", { scope: "touched-files", mode: "log", paths: ["api/page.py"] });
      expect(pages).toEqual([0, 1]);
      expect(audits.get("run-1")?.result?.status).toBe("completed");
    } finally {
      binding.mockRestore();
      await enforcer.close();
    }
  });

  it("rejects an audit whose later result page holds a malformed violation", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    const violation = {
      id: "violation-1", ruleId: "test:needle", clauseId: "needle", code: "NEEDLE", mode: "steer",
      message: "needle", path: "api/page.py", captures: {}, variables: {},
    };
    const session = pagingWorkspace(() => { throw new Error("the session workspace is never paged"); });
    const scan = pagingWorkspace((cursor) => JSON.stringify({
      revision: 1, cursor: cursor + 1, done: cursor === 1, failures: [], coverage: [], bytes: 0,
      // The first page is valid; the second's violation has a mode the contract does not allow.
      violations: [cursor === 0 ? violation : { ...violation, id: "violation-2", mode: "fatal" }],
    }));
    const binding = nativeWorkspaces(session, scan);
    try {
      await writeFile(join(root, "api", "page.py"), "safe()\n");
      await enforcer.openWorkspace("page-malformed");
      await enforcer.audit("page-malformed", { scope: "touched-files", mode: "log", paths: ["api/page.py"] });
      expect(audits.get("run-1")?.result).toEqual({
        status: "failed",
        violations: [],
        failures: [expect.objectContaining({ clauseId: "scan", code: "adapter-error", stage: "scan" })],
      });
    } finally {
      binding.mockRestore();
      await enforcer.close();
    }
  });

  it("caps the failure list of a scan that fails, and keeps how many it leaves out", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    const session = pagingWorkspace(() => { throw new Error("the session workspace is never paged"); });
    const scan = pagingWorkspace(() => { throw new Error("the result stream broke"); });
    const binding = nativeWorkspaces(session, scan);
    try {
      // 25 files past the byte budget fail to read before the scan itself fails.
      const oversized = Array.from({ length: 25 }, (_, index) => `api/oversized-${index}.py`);
      await Promise.all(oversized.map(async (path) => {
        await writeFile(join(root, path), "");
        await fsPromises.truncate(join(root, path), 32_000_001);
      }));
      await writeFile(join(root, "api", "page.py"), "safe()\n");
      await enforcer.openWorkspace("scan-fails");
      await enforcer.audit("scan-fails", { scope: "touched-files", mode: "log", paths: [...oversized, "api/page.py"] });
      const result = audits.get("run-1")?.result;
      expect(result).toMatchObject({ status: "failed", unlistedFailureCount: 5 });
      expect(result?.failures).toHaveLength(21);
      expect(result?.failures).toContainEqual(expect.objectContaining({ clauseId: "scan", code: "adapter-error", stage: "scan" }));
    } finally {
      binding.mockRestore();
      await enforcer.close();
    }
  });

  it("lists each rule the file's work stopped, however many failures came before it", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    const gap = (ruleId: string, code: string, index: number) => ({
      ruleId, clauseId: "c", code, message: "out of work", path: `api/f${index}.py`,
    });
    // One rule runs out of work on 21 files, then the file's work stops another on one more.
    const failures = [
      ...Array.from({ length: 21 }, (_, index) => gap("test:own", "budget-exceeded", index)),
      gap("test:file", "file-budget-exceeded", 21),
    ];
    const session = pagingWorkspace(() => { throw new Error("the session workspace is never paged"); });
    const scan = pagingWorkspace(() => JSON.stringify({
      revision: 1, cursor: 1, done: true, failures, coverage: [], bytes: 0, violations: [],
    }));
    const binding = nativeWorkspaces(session, scan);
    try {
      await writeFile(join(root, "api", "page.py"), "safe()\n");
      await enforcer.openWorkspace("file-work-listed");
      await enforcer.audit("file-work-listed", { scope: "touched-files", mode: "log", paths: ["api/page.py"] });
      const result = audits.get("run-1")?.result;
      expect(result?.status).toBe("completed");
      expect(result?.failures).toContainEqual(expect.objectContaining({ ruleId: "test:file", code: "file-budget-exceeded" }));
      expect(result?.failures).toHaveLength(22);
    } finally {
      binding.mockRestore();
      await enforcer.close();
    }
  });

  it("folds into a blind file only the failures its row stands for, so a cross-file query's failure still fails the audit", async () => {
    const { recorder, audits } = recordingAudits();
    const { root, enforcer } = await fixture({ audits: recorder });
    const path = "api/x.py";
    const failures = [
      { ruleId: "test:local", clauseId: "c", code: "budget-exceeded", message: "clause work budget exceeded", path },
      { ruleId: "test:cross", clauseId: "g", code: "source-incomplete", message: "A cross-file query source is incomplete", path },
    ];
    const blindFiles = [{ path, reason: "budget", clauses: 1, message: "clause work budget exceeded", folded: [["test:local", "c"]] }];
    const session = pagingWorkspace(() => { throw new Error("the session workspace is never paged"); });
    const scan = pagingWorkspace(() => JSON.stringify({
      revision: 1, cursor: 1, done: true, failures, blindFiles, coverage: [], bytes: 0, violations: [],
    }));
    const binding = nativeWorkspaces(session, scan);
    try {
      await writeFile(join(root, "api", "page.py"), "safe()\n");
      await enforcer.openWorkspace("cross-blind");
      await enforcer.audit("cross-blind", { scope: "touched-files", mode: "log", paths: ["api/page.py"] });
      const result = audits.get("run-1")?.result;
      expect(result?.status).toBe("failed");
      expect(result?.failures).toEqual(expect.arrayContaining([
        expect.objectContaining({ ruleId: "test:cross", clauseId: "g", code: "source-incomplete", path }),
        expect.objectContaining({ clauseId: "file", path, blind: "budget" }),
      ]));
      expect(result?.failures).not.toContainEqual(expect.objectContaining({ ruleId: "test:local" }));
    } finally {
      binding.mockRestore();
      await enforcer.close();
    }
  });
});
