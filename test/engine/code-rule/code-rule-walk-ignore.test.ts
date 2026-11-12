import { execFileSync } from "node:child_process";
import type * as FileSystem from "node:fs";
import { appendFile, lstat, mkdir, mkdtemp, realpath, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

/**
 * What the walks under test ask of Git's ignore rules, how often they look for the engine's file, and how often they
 * ask the engine whether a language reads package manifests.
 */
const asked = vi.hoisted(() => ({ paths: 0, folders: 0, engineLookups: 0, manifests: 0 }));
/**
 * Whether the engine's Git ignore matchers are those of an engine built before `ignoredEntries` and `reread`, and
 * whether their `reread` fails, as it does where an exclude file cannot be read.
 */
const engine = vi.hoisted(() => ({ older: false, rereadFails: false }));
vi.mock("node:fs", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  const existsSync = (path: FileSystem.PathLike): boolean => {
    if (String(path).endsWith(".node")) asked.engineLookups += 1;
    return actual.existsSync(path);
  };
  return { ...actual, existsSync, default: { ...actual, existsSync } };
});
vi.mock("../../../src/engine/native/native.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../src/engine/native/native.js")>();
  type Matcher = import("../../../src/engine/native/native.js").NativeGitIgnoreMatcher;
  const counted = (matcher: Matcher): Matcher => ({
    ignored: (path, directory) => {
      asked.paths += 1;
      return matcher.ignored(path, directory);
    },
    ignoredBy: (path, directory) => {
      asked.paths += 1;
      return matcher.ignoredBy(path, directory);
    },
    ignoredEntries: (folder, names, directories) => {
      asked.folders += 1;
      return matcher.ignoredEntries(folder, names, directories);
    },
    reread: () => {
      if (engine.rereadFails) throw new Error("cannot read Git excludes: Could not read repository exclude: Permission denied");
      matcher.reread();
    },
  });
  return {
    ...actual,
    loadNativeBinding: () => {
      const binding = actual.loadNativeBinding();
      // The addon's own properties cannot be replaced; a binding with it as prototype overrides the one it needs.
      const counting = Object.create(binding) as typeof binding;
      Object.defineProperty(counting, "openGitIgnoreMatcher", {
        value: (root: string) => {
          const matcher = binding.openGitIgnoreMatcher(root);
          if (matcher === null) return null;
          const answers = counted(matcher);
          return engine.older ? { ignored: answers.ignored, ignoredBy: answers.ignoredBy } : answers;
        },
      });
      Object.defineProperty(counting, "readsPackageManifests", {
        value: (label: string) => {
          asked.manifests += 1;
          return binding.readsPackageManifests(label);
        },
      });
      return counting;
    },
  };
});

import { CheckoutOwners } from "../../../src/engine/catalog/checkout-owner.js";
import { CheckoutCodeRuleRouter } from "../../../src/engine/code-rule/checkout-code-rule-router.js";
import { CodeRuleBridge } from "../../../src/engine/code-rule/code-rule-bridge.js";
import { NativeCodeRuleEnforcer } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import type { AuditResult } from "../../../src/engine/audit/audit.js";
import type { CodeRuleEditBatch, CodeRuleResponse } from "../../../src/engine/code-rule/code-rule.js";
import { CheckoutCatalogs, startingCheckout } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import type { IgnoreRulesChange } from "../../../src/engine/code-rule/ignore-rule-files.js";
import { setUpTestRepository } from "../../support/repository-rules.js";

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, encoding: "utf8" });
}

const RULE = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
});

/**
 * The router with the edits the walks hand it recorded by session-relative path: the files whose new text it audits,
 * and the files gone from what the walks observe, as a file a nested `.gitignore` starts to ignore is.
 */
class RecordingRouter extends CheckoutCodeRuleRouter {
  readonly audited = new Set<string>();
  readonly gone = new Set<string>();

  override async auditGeneratedEdits(sessionId: string, batch: CodeRuleEditBatch): Promise<CodeRuleResponse> {
    for (const edit of batch.edits) (edit.deleted === true ? this.gone : this.audited).add(edit.path);
    return await super.auditGeneratedEdits(sessionId, batch);
  }
}

/** A Git checkout holding `files` (path to text), all committed but `untracked`, set up with a rule that reads Python. */
async function checkout(files: Record<string, string>, untracked: Record<string, string> = {}) {
  const base = await realpath(await mkdtemp(join(tmpdir(), "rerail-walk-ignore-")));
  cleanups.push(() => rm(base, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 }));
  const root = join(base, "checkout");
  const write = async (entries: Record<string, string>) => {
    for (const [path, text] of Object.entries(entries)) {
      await mkdir(dirname(join(root, path)), { recursive: true });
      await writeFile(join(root, path), text);
    }
  };
  await write(files);
  git(base, "init", "--quiet", "-b", "main", root);
  git(root, "add", "--force", ".");
  git(root, "commit", "--quiet", "-m", "seed");
  await write(untracked);
  const databasePath = join(base, "rerail.sqlite");
  setUpTestRepository(root, { library: [{ kind: "code", rule: RULE }] }, databasePath);
  await awaitClockAfter(root, [...Object.keys(files), ...Object.keys(untracked)]);
  return { base, root, databasePath };
}

/**
 * A session in `root` that follows `.gitignore`: its router and its bridge, open and past their baseline walk; the files
 * its audits found the rule broken in, the files each steer spoke for, and the changes to Git's ignore rules it saw.
 */
async function session(root: string, databasePath: string) {
  const catalogs = new CheckoutCatalogs(databasePath);
  const owners = new CheckoutOwners(() => catalogs.recordedCheckoutsVersion());
  const flagged = new Set<string>();
  const steered: string[][] = [];
  const ignoreChanges: IgnoreRulesChange[] = [];
  const found = (result: Pick<AuditResult, "violations">): void => {
    for (const violation of result.violations ?? []) if (violation.path !== undefined) flagged.add(violation.path);
  };
  const router = new RecordingRouter({
    sessionRoot: root, startingCheckout: startingCheckout(owners, root).checkoutRoot, owners, catalogs,
    steerDelivery: {
      requestSteer: async (_sessionId, _delivery, _content, metadata) => {
        steered.push([...(metadata?.scope ?? [])]);
        return undefined;
      },
    },
    audits: { start: () => "audit", finish: (_auditId, result) => found(result), progress: (_auditId, result) => found(result), interrupt: () => undefined },
    followGitignore: true,
  });
  const bridge = new CodeRuleBridge(root, router, { discoveryWaitMs: 120_000, onIgnoreRulesChanged: change => { ignoreChanges.push(change); } });
  cleanups.push(async () => {
    await bridge.close();
    catalogs.close();
  });
  await bridge.openWorkspace("walk-ignore");
  await bridge.checkpoint("turn");
  return { router, bridge, flagged, steered, ignoreChanges };
}

/** The Python files Git does not ignore in `root`: tracked ones, and untracked ones no ignore rule matches. */
function pythonGitKeeps(root: string): string[] {
  return git(root, "ls-files", "-z", "--cached", "--others", "--exclude-standard")
    .split("\0").filter(path => path.endsWith(".py")).sort();
}

/**
 * Linux stamps files from a per-tick clock, so a file written just before a walk can share its tick, and the walk would
 * not trust what it saw. Waits until the filesystem clock has passed every stamp under `paths`.
 */
async function awaitClockAfter(root: string, paths: readonly string[]): Promise<void> {
  const stamps = await Promise.all(paths.map(async path => {
    const info = await lstat(join(root, path));
    return Math.max(info.mtimeMs, info.ctimeMs);
  }));
  const tick = join(root, "..", "tick");
  for (;;) {
    await writeFile(tick, "");
    const info = await lstat(tick);
    if (Math.max(info.mtimeMs, info.ctimeMs) > Math.max(...stamps)) return;
  }
}

/** A shell command of the session's that adds `NEEDLE` to every one of `paths`, ignored or not; its result walks. */
async function touchEveryPythonFile(bridge: CodeRuleBridge, root: string, paths: readonly string[], id: string): Promise<void> {
  await bridge.begin(id, "Bash", { command: "./touch.sh" });
  for (const path of paths) await appendFile(join(root, path), "NEEDLE\n");
  await bridge.finish(id);
}

describe("a walk of a session that follows .gitignore", () => {
  it("asks Git's ignore rules once per folder, never per file, and finds the engine's file once, asking it no file's language", async () => {
    const files: Record<string, string> = { ".gitignore": "*.log\nout/\n" };
    for (let top = 0; top < 2; top += 1)
      for (let folder = 0; folder < 5; folder += 1)
        for (let name = 0; name < 10; name += 1) files[`src/a${top}/b${folder}/f${name}.py`] = "x = 1\n";
    const { root, databasePath } = await checkout(files, { "src/a0/b0/x.log": "log\n", "out/x.py": "x = 1\n" });
    const { bridge } = await session(root, databasePath);
    // Every file's stamp moves while its text stays, so the walk looks at each file again and audits none of them.
    const pythonFiles = Object.keys(files).filter(path => path.endsWith(".py"));
    const now = new Date();
    for (const path of pythonFiles) await utimes(join(root, path), now, now);
    await awaitClockAfter(root, pythonFiles);
    Object.assign(asked, { paths: 0, folders: 0, engineLookups: 0, manifests: 0 });

    await bridge.checkpoint("turn");

    // The folders the walk enters, Git ignoring out/: the checkout, src, src/a0, src/a1 and the ten src/aN/bM. No rule
    // reads server graphs, so whether Python reads package manifests is never asked.
    expect(asked).toEqual({ paths: 0, folders: 14, engineLookups: 0, manifests: 0 });
  });

  it("observes the files Git keeps, as Git's rules stand at each walk: nested, negated, excluded and tracked ones", async () => {
    const { base, root, databasePath } = await checkout(
      {
        ".gitignore": "*.gen.py\nthird_party/\n!keep.gen.py\nGenerated/\n",
        "api/.gitignore": "secret_*.py\n",
        "api/handler.py": "x = 1\n",
        "api/third_party/forced.py": "x = 1\n",
        "top.py": "x = 1\n",
        "notes/a.py": "x = 1\n",
      },
      {
        "api/secret_key.py": "x = 1\n",
        "api/keep.gen.py": "x = 1\n",
        "api/drop.gen.py": "x = 1\n",
        "third_party/out.py": "x = 1\n",
        "generated/out.py": "x = 1\n",
        "local.py": "x = 1\n",
        "excluded.py": "x = 1\n",
        "notes/b.py": "x = 1\n",
        "api/draft.py": "x = 1\n",
        "scratch.py": "x = 1\n",
        "late_local.py": "x = 1\n",
        "late_excluded.py": "x = 1\n",
      },
    );
    await writeFile(join(root, ".git", "info", "exclude"), "local.py\n");
    await writeFile(join(base, "excludes"), "excluded.py\n");
    git(root, "config", "core.excludesFile", join(base, "excludes"));
    const everyPythonFile = git(root, "ls-files", "-z", "--cached", "--others").split("\0").filter(path => path.endsWith(".py"));
    const { router, bridge } = await session(root, databasePath);

    await touchEveryPythonFile(bridge, root, everyPythonFile, "touch-1");
    expect([...router.audited].sort()).toEqual(pythonGitKeeps(root));
    // Git keeps a file it tracks whatever its rules say, and on a filesystem that ignores case it reads `Generated/`
    // as `generated/` (core.ignoreCase): the comparison with Git covers both.
    expect(router.audited).toContain("api/third_party/forced.py");

    // Between walks, the nested and the top `.gitignore`, `.git/info/exclude` and `core.excludesFile` start to ignore
    // files the walks observed, and Git stops tracking a file its rules match. The next walk drops what the nested
    // `.gitignore` and the untracking leave to Git, as it always did; the files the top `.gitignore` and the exclude
    // files start to ignore stay observed and checked for the session's life, as they were before walks read those again.
    await writeFile(join(root, "api", ".gitignore"), "secret_*.py\ndraft.py\n");
    await appendFile(join(root, ".gitignore"), "scratch.py\n");
    await appendFile(join(root, ".git", "info", "exclude"), "late_local.py\n");
    await appendFile(join(base, "excludes"), "late_excluded.py\n");
    git(root, "rm", "--quiet", "--cached", "api/third_party/forced.py");
    router.audited.clear();
    await touchEveryPythonFile(bridge, root, everyPythonFile, "touch-2");
    const keptChecked = ["late_excluded.py", "late_local.py", "scratch.py"];
    expect([...router.audited].sort()).toEqual([...pythonGitKeeps(root), ...keptChecked].sort());
    expect(router.audited).toContain("api/handler.py");
    expect([...router.gone].sort()).toEqual(["api/draft.py", "api/third_party/forced.py"]);
  });

  it("keeps checking a file the top .gitignore starts to ignore during the session, with its finding, and records the change", async () => {
    const { root, databasePath } = await checkout({ ".gitignore": "*.log\n", "top.py": "x = 1\n" }, { "scratch.py": "x = 1\n" });
    const { router, bridge, flagged, steered, ignoreChanges } = await session(root, databasePath);
    await touchEveryPythonFile(bridge, root, ["scratch.py"], "touch-1");
    await router.settleGeneratedEdits();
    expect(flagged).toContain("scratch.py");
    expect(steered.flat()).toContain("scratch.py");
    router.audited.clear();
    flagged.clear();
    steered.length = 0;

    // One shell command adds the file to the top `.gitignore` and breaks the rule in it again.
    await bridge.begin("ignore-it", "Bash", { command: "./ignore-scratch.sh" });
    await appendFile(join(root, ".gitignore"), "scratch.py\n");
    await appendFile(join(root, "scratch.py"), "NEEDLE\n");
    await bridge.finish("ignore-it");
    await router.settleGeneratedEdits();

    expect(git(root, "check-ignore", "scratch.py")).toBe("scratch.py\n");
    expect([...router.audited]).toEqual(["scratch.py"]);
    expect([...router.gone]).toEqual([]);
    expect(flagged).toContain("scratch.py");
    expect(steered.flat()).toContain("scratch.py");
    expect(ignoreChanges).toEqual([{ path: ".gitignore", via: "observed", ignoresMore: ["scratch.py"] }]);
  });

  it("observes a file the top .gitignore, .git/info/exclude or core.excludesFile stops ignoring from the next walk", async () => {
    const { base, root, databasePath } = await checkout(
      { ".gitignore": "*.log\nwas_top.py\n", "top.py": "x = 1\n" },
      { "was_top.py": "x = 1\n", "was_local.py": "x = 1\n", "was_excluded.py": "x = 1\n" },
    );
    await writeFile(join(root, ".git", "info", "exclude"), "was_local.py\n");
    await writeFile(join(base, "excludes"), "was_excluded.py\n");
    git(root, "config", "core.excludesFile", join(base, "excludes"));
    const everyPythonFile = git(root, "ls-files", "-z", "--cached", "--others").split("\0").filter(path => path.endsWith(".py"));
    const { router, bridge } = await session(root, databasePath);
    await touchEveryPythonFile(bridge, root, everyPythonFile, "touch-1");
    expect([...router.audited]).toEqual(["top.py"]);

    await writeFile(join(root, ".gitignore"), "*.log\n");
    await writeFile(join(root, ".git", "info", "exclude"), "\n");
    await writeFile(join(base, "excludes"), "\n");
    router.audited.clear();
    await touchEveryPythonFile(bridge, root, everyPythonFile, "touch-2");
    expect([...router.audited].sort()).toEqual(["top.py", "was_excluded.py", "was_local.py", "was_top.py"]);
    expect(pythonGitKeeps(root)).toEqual(["top.py", "was_excluded.py", "was_local.py", "was_top.py"]);
  });

  it("follows Git's rules with an engine older than the folder questions, asking per path, and says once to rebuild it", async () => {
    const { root, databasePath } = await checkout(
      { ".gitignore": "*.gen.py\nout/\n", "api/handler.py": "x = 1\n", "top.py": "x = 1\n" },
      { "api/new.py": "x = 1\n", "api/drop.gen.py": "x = 1\n", "out/x.py": "x = 1\n", "out/deep/y.py": "x = 1\n" },
    );
    const everyPythonFile = git(root, "ls-files", "-z", "--cached", "--others").split("\0").filter(path => path.endsWith(".py"));
    engine.older = true;
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      const { router, bridge } = await session(root, databasePath);

      await touchEveryPythonFile(bridge, root, everyPythonFile, "touch-older");

      expect([...router.audited].sort()).toEqual(pythonGitKeeps(root));
      const lines = stderr.mock.calls.map(([text]) => String(text)).filter(text => text.includes("Git ignore"));
      expect(lines).toEqual([expect.stringContaining("older than this Rerail; rebuild or reinstall it")]);
    } finally {
      stderr.mockRestore();
      engine.older = false;
    }
  });

  it("follows Git's rules as read before while a walk cannot read the ignore files again, and says so once", async () => {
    const { root, databasePath } = await checkout(
      { ".gitignore": "*.gen.py\nout/\n", "api/handler.py": "x = 1\n", "top.py": "x = 1\n" },
      { "drop.gen.py": "x = 1\n", "api/new.py": "x = 1\n", "api/drop.gen.py": "x = 1\n", "out/x.py": "x = 1\n", "out/deep/y.py": "x = 1\n" },
    );
    const everyPythonFile = git(root, "ls-files", "-z", "--cached", "--others").split("\0").filter(path => path.endsWith(".py"));
    const { router, bridge } = await session(root, databasePath);
    await touchEveryPythonFile(bridge, root, everyPythonFile, "touch-1");
    expect([...router.audited].sort()).toEqual(pythonGitKeeps(root));

    // The walk reads Git's ignore files again at the session folder, its first: there it fails from now on.
    engine.rereadFails = true;
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      for (const id of ["touch-2", "touch-3"]) {
        router.audited.clear();
        await touchEveryPythonFile(bridge, root, everyPythonFile, id);
        expect([...router.audited].sort()).toEqual(pythonGitKeeps(root));
      }
      const lines = stderr.mock.calls.map(([text]) => String(text)).filter(text => text.includes("Git"));
      expect(lines).toEqual([expect.stringContaining("cannot read Git's ignore files again for ., so walks follow them as read before")]);
    } finally {
      stderr.mockRestore();
      engine.rereadFails = false;
    }
  });

  it("opens a lane for a walk's questions about another worktree only where acceptsPath and ignoresPath of its files do", async () => {
    const { base, root, databasePath } = await checkout({ ".gitignore": "out/\n", "api/x.py": "x = 1\n" });
    git(root, "worktree", "add", "--quiet", "-b", "linked", join(base, "linked"));
    const lanes = vi.spyOn(NativeCodeRuleEnforcer.prototype, "openWorkspace");
    /** How many lanes a fresh session in `root` opens while `ask` asks it about the linked worktree. */
    const lanesOpened = async (sessionId: string, ask: (router: CheckoutCodeRuleRouter) => void): Promise<number> => {
      const catalogs = new CheckoutCatalogs(databasePath);
      const owners = new CheckoutOwners(() => catalogs.recordedCheckoutsVersion());
      const router = new CheckoutCodeRuleRouter({
        sessionRoot: root, startingCheckout: startingCheckout(owners, root).checkoutRoot, owners, catalogs,
        steerDelivery: { requestSteer: async () => undefined },
        audits: { start: () => "audit", finish: () => undefined, progress: () => undefined, interrupt: () => undefined },
        followGitignore: true,
      });
      cleanups.push(async () => {
        await router.close();
        catalogs.close();
      });
      await router.openWorkspace(sessionId);
      const before = lanes.mock.calls.length;
      ask(router);
      return lanes.mock.calls.length - before;
    };
    const entries = [{ name: ".gitignore", directory: false }, { name: "api", directory: true }, { name: "out", directory: true }];
    try {
      const walked = await lanesOpened("walk", router => {
        expect(router.ignoredEntries("../linked", entries)).toEqual([false, false, true]);
        expect(router.ignoredEntries("../linked/api", [{ name: "x.py", directory: false }])).toEqual([false]);
        expect(router.readsPath("../linked/api/x.py")).toBe(true);
      });
      const asked = await lanesOpened("ask", router => {
        expect(entries.map(entry => router.ignoresPath(`../linked/${entry.name}`, entry.directory))).toEqual([false, false, true]);
        expect(router.acceptsPath("../linked/api/x.py")).toBe(true);
      });
      // A walk asks these of every worktree it walks: a change to when the per-path questions open a lane holds for them.
      expect(walked).toBe(asked);
    } finally {
      lanes.mockRestore();
    }
  });
});
