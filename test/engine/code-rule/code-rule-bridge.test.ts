import { execFileSync } from "node:child_process";
import type * as ChildProcess from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { appendFile, lstat, mkdir, mkdtemp, readFile, realpath, rename, rm, utimes, writeFile } from "node:fs/promises";
import type * as FileSystem from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, resolve, sep } from "node:path";
import { setFlagsFromString } from "node:v8";
import { runInNewContext } from "node:vm";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CodeRuleBridge,
  filesystemClock,
  generatedEditPaths,
  isCommandTool,
  ledgerPreimage,
  type SourceObservationCache,
} from "../../../src/engine/code-rule/code-rule-bridge.js";
import { MAX_FINISHED_CALLS, ToolCallWindows } from "../../../src/engine/code-rule/tool-call-windows.js";
import type { CodeRuleEditBatch, CodeRuleAuditor, CodeRuleResponse } from "../../../src/engine/code-rule/code-rule.js";
import type { IgnoreRulesChange } from "../../../src/engine/code-rule/ignore-rule-files.js";
import { codeRuleDialectForPath } from "../../../src/engine/code-rule/code-rule-languages.js";
import type * as WalkReads from "../../../src/engine/code-rule/walk-read-buffers.js";
import { holdRemovals } from "../../support/platform/held-removal.js";
import { removeFolder, runningIn } from "../../support/platform/remove-folder.js";

const discovery = vi.hoisted(() => ({
  reads: new Map<string, Array<() => Promise<void>>>(),
  directories: new Map<string, Array<() => Promise<void>>>(),
}));
/**
 * The arguments of every Git process Rerail started without blocking, for the tests that pin what a pass costs, and a
 * Git command (`failing`) that is made to fail, by an option Git does not know, while it is set.
 */
const asyncGitCalls = vi.hoisted(() => ({ args: [] as string[][], failing: undefined as string | undefined }));
/** How many of the next Git reads run, in Git's place, a program that stays until it is ended, as a Git still at work does. */
const heldGit = vi.hoisted(() => ({ reads: 0 }));
/** How many of the files a walk read were made into text (`LentBytes.text`), for the tests that pin what a walk keeps. */
const decodedReads = vi.hoisted(() => ({ count: 0 }));
/** Each buffer a file read filled while `watching`, by `readFile` or a file handle's `read`, for what a walk allocates. */
const readBuffers = vi.hoisted(() => ({ watching: false, seen: new Set<ArrayBufferLike>() }));
vi.mock("node:child_process", async importOriginal => {
  const actual = await importOriginal<typeof ChildProcess>();
  const { promisify } = await import("node:util");
  const execFile = (...call: Parameters<typeof actual.execFile>) => {
    if (Array.isArray(call[1])) {
      const args = call[1] as readonly string[];
      const fails = asyncGitCalls.failing !== undefined && args.includes(asyncGitCalls.failing);
      const forwarded = fails ? [...args, "--no-such-option"] : [...args];
      call[1] = forwarded;
      asyncGitCalls.args.push(forwarded);
      if (heldGit.reads > 0) {
        heldGit.reads -= 1;
        call[0] = process.execPath;
        call[1] = ["-e", "setTimeout(() => {}, 30_000)", "--", ...forwarded];
      }
    }
    return (actual.execFile as (...forwarded: unknown[]) => ReturnType<typeof actual.execFile>)(...call);
  };
  // `promisify(execFile)` resolves to `{ stdout, stderr }` through this symbol: the wrapper keeps it.
  return { ...actual, execFile: Object.assign(execFile, { [promisify.custom]: (actual.execFile as unknown as Record<symbol, unknown>)[promisify.custom] }) };
});
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  return {
    ...actual,
    readFile: async (...args: Parameters<typeof actual.readFile>) => {
      const contents = await actual.readFile(...args);
      if (readBuffers.watching && Buffer.isBuffer(contents)) readBuffers.seen.add(contents.buffer);
      return contents;
    },
    open: async (...args: Parameters<typeof actual.open>) => {
      const handle = await actual.open(...args);
      if (!readBuffers.watching) return handle;
      // Every form of `read` passes through unchanged; only the buffer it fills is noted.
      const read = handle.read.bind(handle) as (...call: unknown[]) => ReturnType<typeof handle.read>;
      handle.read = ((...call: unknown[]) => {
        if (Buffer.isBuffer(call[0])) readBuffers.seen.add(call[0].buffer);
        return read(...call);
      }) as typeof handle.read;
      return handle;
    },
    opendir: async (...args: Parameters<typeof actual.opendir>) => {
      await discovery.directories.get(String(args[0]))?.shift()?.();
      return await actual.opendir(...args);
    },
    // A test can make Windows hold a folder for a moment (holdRemovals).
    rm: vi.fn(actual.rm),
  };
});
vi.mock("../../../src/engine/code-rule/walk-read-buffers.js", async importOriginal => {
  const actual = await importOriginal<typeof WalkReads>();
  /** A walk's read held once its bytes are in and the file closed (`discovery.reads`), each copy made into text counted. */
  class HeldReadBuffers extends actual.WalkReadBuffers {
    override async lend<T>(absolute: string, size: number, use: (read: WalkReads.LentBytes) => Promise<T>): Promise<T> {
      return await super.lend(absolute, size, async read => {
        await discovery.reads.get(absolute)?.shift()?.();
        return await use({ bytes: read.bytes, text: () => countedText(read) });
      });
    }
  }
  function countedText(read: WalkReads.LentBytes): string {
    decodedReads.count += 1;
    return read.text();
  }
  return { ...actual, WalkReadBuffers: HeldReadBuffers };
});

interface DiscoveryGate {
  entered: Promise<void>;
  release(): void;
  started: boolean;
}
const bridges: CodeRuleBridge[] = [];
const releases: Array<() => void> = [];
function holdDiscovery(kind: "reads" | "directories", path: string): DiscoveryGate {
  const entered = Promise.withResolvers<void>();
  const released = Promise.withResolvers<void>();
  const gate = { entered: entered.promise, release: released.resolve, started: false };
  const queue = discovery[kind].get(path) ?? [];
  queue.push(async () => {
    gate.started = true;
    entered.resolve();
    await released.promise;
  });
  discovery[kind].set(path, queue);
  releases.push(gate.release);
  return gate;
}

function createBridge(...args: ConstructorParameters<typeof CodeRuleBridge>): CodeRuleBridge {
  const bridge = new CodeRuleBridge(...args);
  bridges.push(bridge);
  return bridge;
}

/** Starts a checkpoint that blocks on `gate`; `pass` resolves once the gate is released and the walk is idle. */
const passes: Array<Promise<unknown>> = [];
async function requestDiscovery(
  bridge: CodeRuleBridge,
  gate: DiscoveryGate,
  checkpoint: "turn" | "final" = "turn",
): Promise<{ pass: Promise<CodeRuleResponse> }> {
  const pass = bridge.checkpoint(checkpoint);
  passes.push(pass.catch(() => undefined));
  await gate.entered;
  expect(gate.started).toBe(true);
  return { pass };
}
async function settled<T>(promise: Promise<T>): Promise<boolean> {
  return await Promise.race([promise.then(() => true, () => true), new Promise<boolean>(resolve => setImmediate(() => resolve(false)))]);
}

const directories: string[] = [];
afterEach(async () => {
  releases.splice(0).forEach(release => release());
  discovery.reads.clear();
  discovery.directories.clear();
  heldGit.reads = 0;
  readBuffers.watching = false;
  readBuffers.seen.clear();
  await Promise.all(passes.splice(0));
  await Promise.all(bridges.splice(0).map(bridge => bridge.close()));
  await Promise.all(directories.splice(0).map(async path => await removeFolder(path)));
});

async function workspace(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-policy-bridge-"));
  directories.push(root);
  await mkdir(join(root, "src"));
  return await realpath(root);
}
/**
 * Linux stamps files from a per-tick clock, so files written just before a session open can share its tick.
 * Waits until the filesystem clock has passed every stamp under `paths` so later writes are unambiguous.
 */
async function awaitClockAfter(...paths: string[]): Promise<void> {
  const stamps = await Promise.all(paths.map(async path => { const info = await lstat(path); return Math.max(info.mtimeMs, info.ctimeMs); }));
  const since = Math.max(...stamps);
  const probe = await mkdtemp(join(tmpdir(), "rerail-policy-clock-"));
  directories.push(probe);
  for (;;) {
    await writeFile(join(probe, "tick"), "");
    const info = await lstat(join(probe, "tick"));
    if (Math.max(info.mtimeMs, info.ctimeMs) > since) return;
  }
}
setFlagsFromString("--expose-gc");
const gc = runInNewContext("gc") as () => void;
/** Heap bytes in use once full collections have run and their sweeping has finished. */
async function heapInUse(): Promise<number> {
  for (let pass = 0; pass < 3; pass += 1) {
    gc();
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  return process.memoryUsage().heapUsed;
}
function auditor(auditGeneratedEdits: CodeRuleAuditor["auditGeneratedEdits"]): CodeRuleAuditor {
  return { openWorkspace: vi.fn(async () => undefined), acceptsPath: () => true, auditGeneratedEdits, close: vi.fn(async () => undefined) };
}
function response(batch: CodeRuleEditBatch) {
  return { revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] };
}

describe("CodeRuleBridge", () => {
  it("extracts the paths an edit tool names, any file, leaving out multi-line strings", () => {
    expect(generatedEditPaths("write", { path: "src/a.py", content: "pass" })).toEqual(["src/a.py"]);
    expect(generatedEditPaths("edit", { patch: "[src/a.ts#A1B2]\nPUT 1.=1:\n+pass\n" })).toEqual(["src/a.ts"]);
    expect(generatedEditPaths("ast_edit", { paths: ["src/a.rs", "docs/policy.md"] })).toEqual(["src/a.rs", "docs/policy.md"]);
    expect(generatedEditPaths("write", { path: "Justfile", content: "build:\n" })).toEqual(["Justfile"]);
    expect(generatedEditPaths("apply_patch", { input: "patch body\nwith no file header\n" })).toEqual([]);
    expect(generatedEditPaths("read", { path: "src/a.py" })).toEqual([]);
  });

  it("recognises command tools across hosts", () => {
    for (const tool of ["Bash", "shell", "local_shell", "exec_command", "run_terminal_cmd", "container.exec"]) expect(isCommandTool(tool)).toBe(true);
    for (const tool of ["read", "Read", "write", "edit", "apply_patch", "Grep", "Glob", "WebFetch"]) expect(isCommandTool(tool)).toBe(false);
  });

  it("captures and completes an edit while a repository audit remains pending", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "before\n");
    const entered = Promise.withResolvers<void>();
    const release = Promise.withResolvers<readonly string[]>();
    releases.push(() => release.resolve(["audit-run"]));
    const bridge = createBridge(root, {
      ...auditor(async (_sessionId, batch) => response(batch)),
      audit: async () => {
        entered.resolve();
        return await release.promise;
      },
    });
    await bridge.openWorkspace("session-1");
    let auditCompleted = false;
    const audit = bridge.audit({ scope: "repository", mode: "log" }).then(result => {
      auditCompleted = true;
      return result;
    });
    await entered.promise;
    const capture = bridge.begin("edit-1", "write", { path: "src/a.py" });
    try {
      await expect(capture).resolves.toEqual({ "src/a.py": { text: "before\n" } });
      await writeFile(path, "after\n");
      await expect(bridge.finish("edit-1")).resolves.toEqual([
        expect.objectContaining({ revision: 1, disposition: "continue" }),
      ]);
      expect(auditCompleted).toBe(false);
    } finally {
      release.resolve(["audit-run"]);
      await capture;
      await audit;
    }
  });

  it("captures only the digest and size of a secret-like file as its preimage, never its text", async () => {
    const root = await workspace();
    const text = "TOKEN = 'sk-live-0123456789'\n";
    await writeFile(join(root, "src", "secrets.py"), text);
    await writeFile(join(root, "src", "a.py"), "before\n");
    const bridge = createBridge(root, auditor(async (_sessionId, batch) => response(batch)));
    await bridge.openWorkspace("session-1");
    await expect(bridge.begin("edit-1", "write", { path: ["src/secrets.py", "src/a.py"] })).resolves.toEqual({
      "src/secrets.py": { withheld: { sha256: createHash("sha256").update(text).digest("hex"), bytes: Buffer.byteLength(text) } },
      "src/a.py": { text: "before\n" },
    });
  });

  it("opens and checks tool edits while a stale baseline read is stalled", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "before\n");
    await writeFile(join(root, "src", "untouched.py"), "existing\n");
    await awaitClockAfter(path, join(root, "src", "untouched.py"));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    // Hold bytes already read from disk, not the tool's later preimage/postimage reads.
    const baseline = holdDiscovery("reads", path);
    let opened = false;
    const opening = bridge.openWorkspace("session-1").then(() => { opened = true; });
    await baseline.entered;
    await vi.waitFor(() => expect(opened).toBe(true));
    await opening;

    await expect(bridge.begin("edit-1", "write", { path: "src/a.py" })).resolves.toEqual({
      "src/a.py": { text: "before\n" },
    });
    await writeFile(path, "tool edit\n");
    await expect(bridge.finish("edit-1")).resolves.toEqual([response({ revision: 1, checkpoints: [], edits: [] })]);
    expect(auditGeneratedEdits.mock.calls).toEqual([["session-1", {
      revision: 1,
      checkpoints: ["edit", "function"],
      edits: [{ path: "src/a.py", language: "python", previousText: "before\n", generatedText: "tool edit\n" }],
    }]]);

    // A turn checkpoint waits for the walk instead of reporting partial coverage.
    const turn = bridge.checkpoint("turn");
    passes.push(turn.catch(() => undefined));
    expect(await settled(turn)).toBe(false);
    baseline.release();
    await expect(turn).resolves.toEqual({ revision: 1, disposition: "continue", violations: [], failures: [] });
    // A complete reconciliation must neither replay stale baseline bytes nor emit untouched code.
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);

    await writeFile(path, "external edit\n");
    await bridge.checkpoint("final");
    expect(auditGeneratedEdits.mock.calls).toEqual([
      ["session-1", {
        revision: 1,
        checkpoints: ["edit", "function"],
        edits: [{ path: "src/a.py", language: "python", previousText: "before\n", generatedText: "tool edit\n" }],
      }],
      // Made while no call of the session ran, the external edit is another writer's: the evaluator, holding the file as
      // the tool edit left it, is told in Log, with no generated lines.
      ["session-1", {
        revision: 2,
        checkpoints: ["final"],
        mode: "log",
        edits: [{ path: "src/a.py", language: "python", previousText: "external edit\n", generatedText: "external edit\n" }],
      }],
    ]);
  });

  it("reconciles shell writes at a command tool boundary before its result returns", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "transaction.fetch_value('SELECT 1')\n");
    await awaitClockAfter(path);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    expect(auditGeneratedEdits).not.toHaveBeenCalled();

    await expect(bridge.begin("shell-1", "shell", { command: "python3 -c \"...write_text(...)\"" })).resolves.toEqual({});
    await writeFile(path, "cursor.execute('SELECT 1')\n");
    await expect(bridge.finish("shell-1")).resolves.toEqual([expect.objectContaining({ revision: 1, disposition: "continue" })]);
    expect(auditGeneratedEdits.mock.calls).toEqual([["session-1", {
      revision: 1,
      checkpoints: ["edit", "function"],
      edits: [{
        path: "src/a.py", language: "python",
        previousReadError: "Source changed outside a tool with a captured preimage", generatedText: "cursor.execute('SELECT 1')\n",
      }],
    }]]);

    // A read-only command and a read tool find nothing new.
    await bridge.begin("shell-2", "shell", { command: "sed -n '1,200p' src/a.py" });
    await expect(bridge.finish("shell-2")).resolves.toEqual([]);
    await bridge.begin("read-1", "read", { path: "src/a.py" });
    await expect(bridge.finish("read-1")).resolves.toEqual([]);
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);

    // A failing command may still have written before it failed.
    await bridge.begin("shell-3", "Bash", { command: "python3 write_then_crash.py" });
    await writeFile(path, "cursor.execute('SELECT 2')\n");
    await expect(bridge.finish("shell-3", true)).resolves.toEqual([expect.objectContaining({ revision: 2 })]);
    expect(auditGeneratedEdits).toHaveBeenLastCalledWith("session-1", expect.objectContaining({
      revision: 2,
      edits: [expect.objectContaining({ path: "src/a.py", generatedText: "cursor.execute('SELECT 2')\n" })],
    }));
  });

  it("gives a shell write the preimage the session or Git holds, including its first sight of a file Git does not hold", async () => {
    const root = await workspace();
    const clean = join(root, "src", "clean.py");
    const dirty = join(root, "src", "dirty.py");
    await writeFile(clean, "one\n");
    await writeFile(dirty, "committed\n");
    commitAll(root);
    // Changed before the session and never staged: neither the index nor HEAD holds what the session first saw.
    await writeFile(dirty, "uncommitted\n");
    await awaitClockAfter(clean, dirty);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");

    // The session's first sight of a file Git does not hold is kept, so a shell write to it counts only its own lines.
    await bridge.begin("shell-1", "shell", { command: "./rewrite.sh" });
    await writeFile(clean, "one\ntwo\n");
    await writeFile(dirty, "uncommitted\nmore\n");
    await bridge.finish("shell-1");
    expect(auditGeneratedEdits.mock.calls.at(-1)?.[1].edits).toEqual([
      { path: "src/clean.py", language: "python", previousText: "one\n", generatedText: "one\ntwo\n" },
      { path: "src/dirty.py", language: "python", previousText: "uncommitted\n", generatedText: "uncommitted\nmore\n" },
    ]);

    // The session saw what the first write made of the file, so the next write knows its preimage.
    await bridge.begin("shell-2", "shell", { command: "./rewrite.sh" });
    await writeFile(dirty, "uncommitted\nmore\nagain\n");
    await bridge.finish("shell-2");
    expect(auditGeneratedEdits.mock.calls.at(-1)?.[1].edits).toEqual([
      { path: "src/dirty.py", language: "python", previousText: "uncommitted\nmore\n", generatedText: "uncommitted\nmore\nagain\n" },
    ]);
  });

  it("gives a shell write to a file untracked before the session the text the session first saw", async () => {
    const root = await workspace();
    const untracked = join(root, "src", "untracked.py");
    await writeFile(join(root, "src", "kept.py"), "kept\n");
    commitAll(root);
    await writeFile(untracked, "draft\n");
    await awaitClockAfter(untracked);
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => writeFile(untracked, "draft\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/untracked.py", language: "python", previousText: "draft\n", generatedText: "draft\nmine\n" }],
    ]);
  });

  it("keeps first-sight text only within its budget, and measures a shell edit past it as before", async () => {
    const root = await workspace();
    const [first, second] = [join(root, "src", "a.py"), join(root, "src", "b.py")];
    await writeFile(first, "old\n");
    await writeFile(second, "old\n");
    commitAll(root);
    await writeFile(first, "aaaa\n");
    await writeFile(second, "bbbb\n");
    await awaitClockAfter(first, second);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits), { firstSightTextBytes: 5 });
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await shell(bridge, async () => {
      await writeFile(first, "aaaa\nmine\n");
      await writeFile(second, "bbbb\nmine\n");
    });
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([[
      { path: "src/a.py", language: "python", previousText: "aaaa\n", generatedText: "aaaa\nmine\n" },
      {
        path: "src/b.py", language: "python",
        previousReadError: "Source changed outside a tool with a captured preimage", generatedText: "bbbb\nmine\n",
      },
    ]]);
  });

  it("measures a shell edit from the text a later walk first saw in a folder the baseline could not read", async () => {
    const root = await realpath(await workspace());
    const draft = join(root, "deep", "draft.py");
    await writeFile(join(root, "src", "kept.py"), "kept\n");
    commitAll(root);
    await mkdir(join(root, "deep"));
    await writeFile(draft, "draft\n");
    await awaitClockAfter(draft);
    discovery.directories.set(join(root, "deep"), [async () => { throw new Error("EMFILE: too many open files"); }]);
    const { bridge, auditGeneratedEdits } = await opened(root);
    // The next walk reads the folder: its first sight of the file.
    await bridge.checkpoint("turn");
    await shell(bridge, () => writeFile(draft, "draft\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "deep/draft.py", language: "python", previousText: "draft\n", generatedText: "draft\nmine\n" }],
    ]);
  });

  it("makes no text of the files a later walk first sees, as its baseline, in a folder the baseline could not read", async () => {
    const root = await realpath(await workspace());
    await mkdir(join(root, "deep"));
    const files = Array.from({ length: 20 }, (_, index) => join(root, "deep", `f${index}.py`));
    await Promise.all(files.map((file, index) => writeFile(file, `x = ${index}\n`)));
    commitAll(root);
    await awaitClockAfter(...files);
    discovery.directories.set(join(root, "deep"), [async () => { throw new Error("EMFILE: too many open files"); }]);
    decodedReads.count = 0;
    const { bridge, auditGeneratedEdits } = await opened(root);
    // A later walk reads the folder: its first sight of every file there, none of which can be an edit.
    await bridge.checkpoint("turn");
    expect(decodedReads.count).toBe(0);
    await shell(bridge, () => writeFile(files[0]!, "x = 0\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "deep/f0.py", language: "python", previousText: "x = 0\n", generatedText: "x = 0\nmine\n" }],
    ]);
  });

  it("audits a shell command's writes that its own walk is the first to see, in a folder the baseline could not read", async () => {
    const root = await realpath(await workspace());
    await writeFile(join(root, "src", "kept.py"), "kept\n");
    await mkdir(join(root, "deep"));
    await writeFile(join(root, "deep", "old.py"), "old\n");
    commitAll(root);
    await awaitClockAfter(join(root, "deep", "old.py"));
    // Every walk the session's opening makes fails to list the folder; the command's walk is the first to read it.
    const failing = async () => { throw new Error("EMFILE: too many open files"); };
    discovery.directories.set(join(root, "deep"), Array.from({ length: 20 }, () => failing));
    const { bridge, auditGeneratedEdits } = await opened(root);
    discovery.directories.delete(join(root, "deep"));
    await shell(bridge, async () => {
      await writeFile(join(root, "deep", "old.py"), "old\nviolation\n");
      await writeFile(join(root, "deep", "new.py"), "violation\n");
    });
    const uncaptured = "Source changed during initial discovery without a captured preimage";
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([[[
      { path: "deep/new.py", language: "python", previousReadError: uncaptured, generatedText: "violation\n" },
      { path: "deep/old.py", language: "python", previousReadError: uncaptured, generatedText: "old\nviolation\n" },
    ], undefined]]);
  });

  it("measures a shell edit from the text of a file a command moved into the session folder, stamped before the session", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "a\n");
    commitAll(root);
    const outside = await workspace();
    await mkdir(join(outside, "pkg"));
    const moved = join(outside, "pkg", "m.py");
    await writeFile(moved, "moved\n");
    await awaitClockAfter(join(root, "src", "a.py"), moved);
    const { bridge, auditGeneratedEdits } = await opened(root);
    // A move keeps the stamps of the files in the folder, so the walk first sees m.py with a stamp from before the session.
    await shell(bridge, () => rename(join(outside, "pkg"), join(root, "src", "pkg")));
    await shell(bridge, () => writeFile(join(root, "src", "pkg", "m.py"), "moved\nmine\n"));
    // Whether the move itself is audited depends on the stamps the platform gives a moved folder: only the edit is pinned.
    expect(auditGeneratedEdits.mock.calls.at(-1)?.[1].edits).toEqual([
      { path: "src/pkg/m.py", language: "python", previousText: "moved\n", generatedText: "moved\nmine\n" },
    ]);
  });

  it("audits nothing Git wrote over a file changed before the session, and measures the next shell edit from Git's text", async () => {
    const root = await workspace();
    const dirty = join(root, "src", "dirty.py");
    await writeFile(dirty, "committed\n");
    commitAll(root);
    await writeFile(dirty, "uncommitted\n");
    await awaitClockAfter(dirty);
    const { bridge, auditGeneratedEdits } = await opened(root);
    // The session's first sight of the file is no change of its own, and no longer the file's text once Git wrote over it.
    await shell(bridge, () => git(root, "stash", "--quiet"));
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shell(bridge, () => writeFile(dirty, "committed\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/dirty.py", language: "python", previousText: "committed\n", generatedText: "committed\nmine\n" }],
    ]);
  });

  it("closes and reopens without waiting for an abandoned baseline read", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "old session baseline\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const engine = auditor(auditGeneratedEdits);
    const bridge = createBridge(root, engine);
    const abandoned = holdDiscovery("reads", path);
    let opened = false;
    const opening = bridge.openWorkspace("old-session").then(() => { opened = true; });
    await abandoned.entered;
    await vi.waitFor(() => expect(opened).toBe(true));
    await opening;
    await bridge.close();
    await writeFile(path, "replacement baseline\n");
    const replacement = holdDiscovery("reads", path);
    await bridge.openWorkspace("replacement-session");
    await replacement.entered;
    await bridge.begin("replacement-edit", "write", { path: "src/a.py" });
    await writeFile(path, "replacement edit\n");
    await bridge.finish("replacement-edit");

    abandoned.release();
    replacement.release();
    await bridge.checkpoint("turn");
    expect(auditGeneratedEdits.mock.calls).toEqual([["replacement-session", {
      revision: 1,
      checkpoints: ["edit", "function"],
      edits: [{
        path: "src/a.py", language: "python",
        previousText: "replacement baseline\n", generatedText: "replacement edit\n",
      }],
    }]]);
    expect(engine.close).toHaveBeenCalledTimes(1);
  });

  it("reports a command's uncaptured shell writes made before baseline discovery reaches their files", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "existing before startup\n");
    await writeFile(join(root, "src", "untouched.py"), "existing unchanged\n");
    await awaitClockAfter(join(root, "src", "a.py"), join(root, "src", "untouched.py"));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    const baseline = holdDiscovery("directories", root);
    await bridge.openWorkspace("session-1");
    await baseline.entered;
    await bridge.begin("shell-1", "Bash", { command: "./replace.sh" });
    await writeFile(join(root, "src", "a.py"), "external replacement\n");
    await writeFile(join(root, "src", "new.py"), "external creation\n");
    const final = bridge.checkpoint("final");
    passes.push(final.catch(() => undefined));
    baseline.release();

    await expect(final).resolves.toEqual({ revision: 1, disposition: "continue", violations: [], failures: [] });
    expect(auditGeneratedEdits.mock.calls).toEqual([["session-1", {
      revision: 1,
      checkpoints: ["final"],
      edits: [
        {
          path: "src/a.py", language: "python",
          previousReadError: "Source changed during initial discovery without a captured preimage", generatedText: "external replacement\n",
        },
        {
          path: "src/new.py", language: "python",
          previousReadError: "Source changed during initial discovery without a captured preimage", generatedText: "external creation\n",
        },
      ],
    }]]);
    await bridge.checkpoint("turn");
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    await expect(bridge.finish("shell-1")).resolves.toEqual([]);
  });

  it("reconciles a shell write made before the baseline reaches its file at the command boundary", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "transaction.fetch_value('SELECT 1')\n");
    await awaitClockAfter(path);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    const baseline = holdDiscovery("directories", root);
    await bridge.openWorkspace("session-1");
    await baseline.entered;

    await bridge.begin("shell-1", "Bash", { command: "python3 -c \"...write_text(...)\"" });
    await writeFile(path, "cursor.execute('SELECT 1')\n");
    const result = bridge.finish("shell-1");
    passes.push(result.catch(() => undefined));
    expect(await settled(result)).toBe(false);
    baseline.release();
    await expect(result).resolves.toEqual([expect.objectContaining({ revision: 1 })]);
    expect(auditGeneratedEdits.mock.calls).toEqual([["session-1", {
      revision: 1,
      checkpoints: ["edit", "function"],
      edits: [{
        path: "src/a.py", language: "python",
        previousReadError: "Source changed during initial discovery without a captured preimage", generatedText: "cursor.execute('SELECT 1')\n",
      }],
    }]]);
  });

  it("answers a turn checkpoint once its wait runs out on a baseline walk, and audits what the walk finds behind it", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "before\n");
    await awaitClockAfter(path);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits), { discoveryWaitMs: 50 });
    const baseline = holdDiscovery("directories", root);
    await bridge.openWorkspace("session-1");
    await baseline.entered;
    await bridge.begin("shell-1", "Bash", { command: "./write.sh &" });
    await writeFile(path, "shell write\n");

    await expect(bridge.checkpoint("turn")).resolves.toEqual(expect.objectContaining({
      failures: [expect.objectContaining({ code: "source-incomplete", message: "Initial source discovery is still pending" })],
    }));
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    baseline.release();
    await vi.waitFor(() => expect(auditGeneratedEdits).toHaveBeenCalledTimes(1));
    expect(auditGeneratedEdits.mock.calls).toEqual([["session-1", expect.objectContaining({
      edits: [expect.objectContaining({ path: "src/a.py", generatedText: "shell write\n" })],
    })]]);
  });

  it("answers a shell result once its wait runs out on a baseline walk, marks the command boundary, and audits the write behind it", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "before\n");
    await awaitClockAfter(path);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const pending: string[] = [];
    const bridge = createBridge(root, auditor(auditGeneratedEdits), { discoveryWaitMs: 50, onDiscoveryPending: boundary => { pending.push(boundary); } });
    const baseline = holdDiscovery("directories", root);
    await bridge.openWorkspace("session-1");
    await baseline.entered;

    await bridge.begin("shell-1", "Bash", { command: "python3 -c \"...write_text(...)\"" });
    await writeFile(path, "shell write\n");
    await expect(bridge.finish("shell-1")).resolves.toEqual([]);
    expect(pending).toEqual(["command"]);
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    baseline.release();
    await vi.waitFor(() => expect(auditGeneratedEdits).toHaveBeenCalledTimes(1));
    expect(auditGeneratedEdits.mock.calls).toEqual([["session-1", expect.objectContaining({
      edits: [expect.objectContaining({ path: "src/a.py", generatedText: "shell write\n" })],
    })]]);
  });

  it("answers a boundary and still closes the evaluator when recording that the walk is still pending fails", async () => {
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const root = await workspace();
    const evaluator = auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch)));
    const onDiscoveryPending = vi.fn(() => { throw new Error("database is locked"); });
    const bridge = createBridge(root, evaluator, { discoveryWaitMs: 50, onDiscoveryPending });
    const baseline = holdDiscovery("directories", root);
    try {
      await bridge.openWorkspace("session-1");
      await baseline.entered;

      await expect(bridge.checkpoint("turn")).resolves.toEqual(expect.objectContaining({
        failures: [expect.objectContaining({ code: "source-incomplete" })],
      }));
      await expect(bridge.close()).resolves.toBeUndefined();
      expect(onDiscoveryPending.mock.calls).toEqual([["turn"], ["close"]]);
      expect(evaluator.close).toHaveBeenCalledTimes(1);
      expect(stderr).toHaveBeenCalledWith(expect.stringContaining("recording failed: database is locked"));
    } finally {
      stderr.mockRestore();
    }
  });

  it("discovers a command's shell write that races the startup snapshot read", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "before\n");
    await awaitClockAfter(path);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    const reading = holdDiscovery("reads", path);
    await bridge.openWorkspace("session-1");
    await reading.entered;
    await bridge.begin("shell-race", "Bash", { command: "./write.sh" });
    await writeFile(path, "after\n");
    reading.release();
    await bridge.finish("shell-race");
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", expect.objectContaining({
      edits: [expect.objectContaining({ path: "src/a.py", generatedText: "after\n" })],
    }));
    await bridge.checkpoint("turn");
    // Each audit is the session's own discovery, never a Log audit of another writer's change.
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.mode)).not.toContain("log");
    expect(auditGeneratedEdits).toHaveBeenLastCalledWith("session-1", expect.objectContaining({
      edits: [expect.objectContaining({ path: "src/a.py", generatedText: "after\n" })],
    }));
  });

  it("delivers committed full-file snapshots in deterministic batches", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    await writeFile(join(root, "src", "b.rs"), "fn before() {}\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));

    await bridge.openWorkspace("session-1");
    await bridge.begin("call-1", "write", { path: ["src/b.rs", "src/a.py"] });
    await writeFile(join(root, "src", "a.py"), "after\n");
    await writeFile(join(root, "src", "b.rs"), "fn after() {}\n");
    await bridge.finish("call-1");
    await bridge.close();

    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", {
      revision: 1,
      checkpoints: ["edit", "function"],
      edits: [
        { path: "src/a.py", language: "python", previousText: "before\n", generatedText: "after\n" },
        { path: "src/b.rs", language: "rust", previousText: "fn before() {}\n", generatedText: "fn after() {}\n" },
      ],
    });
  });

  it("labels React files with the tsx language so the engine selects the JSX grammar", async () => {
    const root = await workspace();
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = new CodeRuleBridge(root, auditor(auditGeneratedEdits));

    await bridge.openWorkspace("session-1");
    await bridge.begin("call-1", "write", { path: ["src/view.tsx", "src/plain.ts"] });
    await writeFile(join(root, "src", "view.tsx"), "export const View = () => <div />;\n");
    await writeFile(join(root, "src", "plain.ts"), "export const plain = 1;\n");
    await bridge.finish("call-1");
    await bridge.close();

    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", expect.objectContaining({
      edits: [
        { path: "src/plain.ts", language: "typescript", generatedText: "export const plain = 1;\n" },
        { path: "src/view.tsx", language: "tsx", generatedText: "export const View = () => <div />;\n" },
      ],
    }));
  });

  it("labels Blade templates with the blade language, not php, suffix-only basenames by their extname, and an unlisted name as text", async () => {
    const root = await workspace();
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = new CodeRuleBridge(root, auditor(auditGeneratedEdits));

    await bridge.openWorkspace("session-1");
    await bridge.begin("call-1", "write", { path: ["src/page.blade.php", "src/Kernel.php", "src/.blade.php", "src/.php"] });
    await writeFile(join(root, "src", "page.blade.php"), "{{ $title }}\n");
    await writeFile(join(root, "src", "Kernel.php"), "<?php\necho 1;\n");
    await writeFile(join(root, "src", ".blade.php"), "<?php\necho 2;\n");
    await writeFile(join(root, "src", ".php"), "<?php\necho 3;\n");
    await bridge.finish("call-1");
    await bridge.close();

    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", expect.objectContaining({
      edits: [
        { path: "src/.blade.php", language: "php", generatedText: "<?php\necho 2;\n" },
        { path: "src/.php", language: "text", generatedText: "<?php\necho 3;\n" },
        { path: "src/Kernel.php", language: "php", generatedText: "<?php\necho 1;\n" },
        { path: "src/page.blade.php", language: "blade", generatedText: "{{ $title }}\n" },
      ],
    }));
  });

  it("classifies benchmark dialects by the same basename suffix rule as the bridge", () => {
    expect(codeRuleDialectForPath("src/page.blade.php")).toBe("blade");
    expect(codeRuleDialectForPath("src/Page.Blade.PHP")).toBe("blade");
    expect(codeRuleDialectForPath("src/.blade.php")).toBe("php");
    expect(codeRuleDialectForPath("src/.php")).toBeUndefined();
  });

  it("does not consume revisions for failed, unsupported, or no-op transactions", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));

    await bridge.openWorkspace("session-1");
    await bridge.begin("failed", "write", { path: "src/a.py" });
    await writeFile(join(root, "src", "a.py"), "failed\n");
    await bridge.finish("failed", true);
    await bridge.begin("unsupported", "read", { path: "src/a.py" });
    await bridge.finish("unsupported");
    await bridge.begin("noop", "write", { path: "src/a.py" });
    await writeFile(join(root, "src", "a.py"), "failed\n");
    await bridge.finish("noop");
    await bridge.begin("committed", "write", { path: "src/a.py" });
    await writeFile(join(root, "src", "a.py"), "after\n");
    await bridge.finish("committed");

    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", expect.objectContaining({ revision: 1, edits: [expect.objectContaining({ path: "src/a.py", generatedText: "after\n" })] }));
  });

  it("discovers shell changes without treating scope changes or identical rewrites as edits", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    await awaitClockAfter(join(root, "src", "a.py"));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const onRevision = vi.fn();
    let enabled = false;
    const bridge = createBridge(root, {
      ...auditor(auditGeneratedEdits), acceptsPath: () => enabled,
    }, { onRevision });

    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    enabled = true;
    await bridge.checkpoint("final");
    expect(auditGeneratedEdits).not.toHaveBeenCalled();

    await bridge.begin("shell-1", "Bash", { command: "./write.sh &" });
    await writeFile(join(root, "src", "a.py"), "after\n");
    await bridge.checkpoint("final");
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    // Rewriting identical bytes changes the stamp but not the source.
    const rewrite = holdDiscovery("directories", root);
    const { pass } = await requestDiscovery(bridge, rewrite);
    await writeFile(join(root, "src", "a.py"), "after\n");
    rewrite.release();
    await pass;
    await bridge.checkpoint("turn");

    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", expect.objectContaining({
      revision: 1,
      checkpoints: ["final"],
      edits: [expect.objectContaining({ path: "src/a.py", previousReadError: expect.any(String), generatedText: "after\n" })],
    }));
    expect(onRevision).toHaveBeenCalledWith("session-1", 1, ["src/a.py"]);
    await expect(bridge.finish("shell-1")).resolves.toEqual([]);
  });

  it("opens the workspace before the baseline walk and publishes the walk once settled", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    const observations: SourceObservationCache = new Map();
    const bridge = createBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), { observations });

    const baseline = holdDiscovery("reads", join(root, "src", "a.py"));
    await bridge.openWorkspace("session-1");
    await baseline.entered;
    expect(observations.size).toBe(0);
    baseline.release();

    await vi.waitFor(() => expect([...observations.get(root)?.keys() ?? []]).toEqual(["src/a.py"]));
    expect(observations.get(root)?.get("src/a.py")).toMatchObject({ language: "python", digest: expect.any(String), stamp: expect.any(String) });
  });

  it("still discovers sources changed between sessions that share baseline observations", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    await writeFile(join(root, "src", "b.py"), "stable\n");
    // A stamp from the same tick as the first walk is racy and would be re-read, so let the clock pass it.
    await awaitClockAfter(join(root, "src", "a.py"), join(root, "src", "b.py"));
    const observations: SourceObservationCache = new Map();
    const first = createBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), { observations });
    await first.openWorkspace("session-1");
    await vi.waitFor(() => expect(observations.get(root)?.get("src/b.py")?.digest).toBeDefined());
    const shared = new Map(observations.get(root));
    await first.close();

    await writeFile(join(root, "src", "a.py"), "after\n");
    await awaitClockAfter(join(root, "src", "a.py"));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const second = createBridge(root, auditor(auditGeneratedEdits), { observations });
    const unchangedRead = holdDiscovery("reads", join(root, "src", "b.py"));
    await second.openWorkspace("session-2");
    // The second baseline reuses b.py's digest by stamp and re-reads only the changed a.py.
    await vi.waitFor(() => expect(observations.get(root)?.get("src/a.py")?.digest).not.toBe(shared.get("src/a.py")?.digest));
    expect(observations.get(root)?.get("src/b.py")).toEqual(shared.get("src/b.py"));
    expect(unchangedRead.started).toBe(false);
    expect(auditGeneratedEdits).not.toHaveBeenCalled();

    await shell(second, () => writeFile(join(root, "src", "a.py"), "after again\n"));
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-2", expect.objectContaining({
      edits: [expect.objectContaining({ path: "src/a.py", previousReadError: expect.any(String), generatedText: "after again\n" })],
    }));
  });

  it("keeps one observation of an unchanged source for all the sessions that share baseline observations", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "read.py"), "stable\n");
    await writeFile(join(root, "src", "skipped.py"), "stable\n");
    await awaitClockAfter(join(root, "src", "read.py"), join(root, "src", "skipped.py"));
    const observations: SourceObservationCache = new Map();
    // A source no rule reads is observed by its stamp alone.
    const open = async (sessionId: string): Promise<void> => {
      const bridge = createBridge(root, {
        ...auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))),
        acceptsPath: path => path !== "src/skipped.py",
      }, { observations });
      await bridge.openWorkspace(sessionId);
    };
    await open("session-1");
    await vi.waitFor(() => expect(observations.get(root)?.get("src/read.py")?.digest).toBeDefined());
    const first = observations.get(root)!;
    await open("session-2");
    await vi.waitFor(() => expect(observations.get(root)).not.toBe(first));
    // Each live session keeps a baseline of every source of its checkout, so a copy each would cost a whole checkout's
    // observations a session.
    expect(observations.get(root)?.get("src/read.py")).toBe(first.get("src/read.py"));
    expect(observations.get(root)?.get("src/skipped.py")).toBe(first.get("src/skipped.py"));
  });

  it("costs each session a table of the shared observations, not copies of them or of their paths", { timeout: 60_000 }, async () => {
    const root = await workspace();
    const files = 2_000;
    // Names of about 165 characters: a path string of a session's own then costs more than the session's table, so a
    // session that keys its table by the walk's own path strings fails the bound by a wide margin.
    const name = (index: number): string => `module_${index}_${"x".repeat(150)}.py`;
    for (let index = 0; index < files; index += 1) await writeFile(join(root, "src", name(index)), "stable\n");
    await awaitClockAfter(join(root, "src", name(files - 1)));
    const observations: SourceObservationCache = new Map();
    const open = async (sessionId: string): Promise<void> => {
      const published = observations.get(root);
      const bridge = createBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), { observations });
      await bridge.openWorkspace(sessionId);
      await vi.waitFor(() => expect(observations.get(root)).not.toBe(published), { timeout: 20_000 });
      // The walk that follows a baseline has finished too: nothing a walk holds while it runs is counted.
      await bridge.checkpoint("turn");
    };
    // The first sessions read every source and pay for the compiled code.
    await open("session-1");
    await open("session-2");
    const before = await heapInUse();
    const sessions = 5;
    for (let index = 0; index < sessions; index += 1) await open(`session-${index + 3}`);
    // Each session's tables hold 60 to 69 bytes a source. A path string of its own brings that to about 250, and an
    // observation of its own, which "keeps one observation of an unchanged source" also catches, to about 195.
    expect((await heapInUse() - before) / sessions / files).toBeLessThan(105);
  });

  it("abandons the baseline walk of an interrupted session", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    const observations: SourceObservationCache = new Map();
    const bridge = new CodeRuleBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), { observations });

    await bridge.openWorkspace("session-1");
    bridge.interrupt();
    await bridge.close();

    expect(observations.size).toBe(0);
  });

  it("closes mid-walk with no Git it started still running in the folder, which can then be removed at once", async () => {
    const root = await workspace();
    git(root, "init", "--quiet");
    const bridge = new CodeRuleBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), {
      onIgnoreRulesChanged: () => undefined,
    });

    await bridge.openWorkspace("session-1");
    // The baseline walk reads HEAD and the ignore rules ask Git for their exclude files, neither waited for by the open.
    expect(runningIn(root)).not.toEqual([]);
    await bridge.close();

    expect(runningIn(root)).toEqual([]);
    // One try: on Windows a process still running in the folder keeps it from being removed.
    await removeFolder(root, { retry: false });
  });

  it("reopens with no Git the earlier session started still running in the folder", async () => {
    const root = await workspace();
    git(root, "init", "--quiet");
    const bridge = createBridge(root, auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))));
    // The first session's walk reads HEAD first, through a Git that runs until it is ended.
    heldGit.reads = 1;
    await bridge.openWorkspace("session-1");
    const first = runningIn(root);
    expect(first).not.toEqual([]);

    await bridge.openWorkspace("session-2");
    // The second session's own Git may still run; none of the first session's does.
    expect(runningIn(root).filter(entry => first.includes(entry))).toEqual([]);
  });

  it("sends a deletion with its preimage instead of an unreadable workspace scan", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "before\n");
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));

    await bridge.openWorkspace("session-1");
    await bridge.begin("delete", "edit", { path: "src/a.py" });
    await rm(join(root, "src", "a.py"));
    await bridge.finish("delete");

    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", {
      revision: 1,
      checkpoints: ["edit", "function"],
      edits: [{ path: "src/a.py", language: "python", previousText: "before\n", deleted: true }],
    });
  });

  it("reports a failed preimage capture instead of treating it as a new file", async () => {
    const root = await workspace();
    await mkdir(join(root, "src", "a.py"));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));

    await bridge.openWorkspace("session-1");
    await bridge.begin("replace", "write", { path: "src/a.py" });
    await removeFolder(join(root, "src", "a.py"));
    await writeFile(join(root, "src", "a.py"), "after\n");
    await bridge.finish("replace");
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", expect.objectContaining({ edits: [expect.objectContaining({ path: "src/a.py", previousReadError: expect.any(String), generatedText: "after\n" })] }));
  });

  it("audits a write outside the session folder and never reports it deleted by a later walk of the folder", async () => {
    const base = await workspace();
    const session = join(base, "session");
    const sibling = join(base, "sibling");
    await mkdir(join(session, "src"), { recursive: true });
    await mkdir(sibling);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(session, {
      ...auditor(auditGeneratedEdits),
      sourceRoot: path => resolve(session, path).startsWith(`${sibling}${sep}`) ? sibling : session,
    });

    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await bridge.begin("call-1", "write", { path: join(sibling, "x.py") });
    await writeFile(join(sibling, "x.py"), "after\n");
    await bridge.finish("call-1");
    // The walk covers the session folder only; the sibling file it never sees is not a deletion.
    await bridge.checkpoint("turn");
    await bridge.checkpoint("final");

    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([
      { path: "../sibling/x.py", language: "python", generatedText: "after\n" },
    ]);
  });

  it("does not read a file the auditor finds no owner for", async () => {
    const root = await workspace();
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    // The owner can change between `acceptsPath` and the read; no owner must never mean "the session folder".
    const bridge = createBridge(root, { ...auditor(auditGeneratedEdits), sourceRoot: () => undefined });

    await bridge.openWorkspace("session-1");
    await bridge.begin("call-1", "write", { path: "src/a.py" });
    await writeFile(join(root, "src", "a.py"), "after\n");
    await bridge.finish("call-1");

    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits)).toEqual([]);
  });
});

describe("ledgerPreimage", () => {
  it("keeps the text of a source that only names a token, and withholds one that assigns one", () => {
    const source = "import { getAuthTokenFromHeader } from './auth';\n\nconst token = getAuthTokenFromHeader;\nexport default token;\n";
    expect(ledgerPreimage({ text: source })).toEqual({ text: source });
    const leaky = "const token = 'Zk9xQ2mP7vLw8s';\n";
    expect(ledgerPreimage({ text: leaky })).toEqual({
      withheld: { sha256: createHash("sha256").update(leaky).digest("hex"), bytes: Buffer.byteLength(leaky) },
    });
  });
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}
/** `path` relative to `root` as the bridge names a file, with `/` between its parts on every platform. */
function repositoryPath(root: string, path: string): string {
  return relative(root, path).split(sep).join("/");
}
/** Makes `root` a repository whose one commit holds every file in it. */
function commitAll(root: string): void {
  git(root, "init", "--quiet");
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
}
/** A repository on `main`, with a branch `other` ahead of it: it changes `src/a.py`, adds `src/new.py` and removes `src/gone.py`. */
async function twoBranches(): Promise<string> {
  const root = await workspace();
  await writeFile(join(root, "src", "a.py"), "old\n");
  await writeFile(join(root, "src", "gone.py"), "gone\n");
  git(root, "init", "--quiet", "--initial-branch=main");
  // Git writes the files as committed, LF, whatever the machine sets: Git for Windows sets core.autocrlf=true for every
  // repository. The describe block on core.autocrlf sets it for its own repositories.
  git(root, "config", "core.autocrlf", "false");
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
  git(root, "switch", "--quiet", "-c", "other");
  await writeFile(join(root, "src", "a.py"), "old\nviolation\n");
  await writeFile(join(root, "src", "new.py"), "violation\n");
  await rm(join(root, "src", "gone.py"));
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "other");
  git(root, "switch", "--quiet", "main");
  await awaitClockAfter(join(root, "src", "a.py"), join(root, "src", "gone.py"));
  return root;
}
/** A bridge opened on `root` with its baseline taken, and its auditor's calls. */
async function opened(root: string, extra: Partial<CodeRuleAuditor> = {}, worktreeWalkGapMs = 0) {
  const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
  const bridge = createBridge(root, { ...auditor(auditGeneratedEdits), ...extra }, { worktreeWalkGapMs });
  await bridge.openWorkspace("session-1");
  await bridge.checkpoint("turn");
  await bridge.settleWorktreeWalk();
  return { bridge, auditGeneratedEdits };
}
/** Runs `run` as a shell tool call of `bridge`, and what the call's finish answered. */
async function shell(bridge: CodeRuleBridge, run: () => void | Promise<void>): Promise<readonly CodeRuleResponse[]> {
  await bridge.begin("shell-1", "shell", { command: "./run.sh" });
  await run();
  return await bridge.finish("shell-1");
}
/** How many Git processes `run` started to read a checkout's `HEAD`, and to list its unmerged entries. */
async function gitReads(run: () => Promise<unknown>): Promise<{ heads: number; entries: number }> {
  asyncGitCalls.args.length = 0;
  await run();
  const named = (name: string) => asyncGitCalls.args.filter(args => args.includes(name)).length;
  return { heads: named("--verify"), entries: named("--unmerged") };
}
/**
 * A repository with a checkout in `lib` that owns its files: a repository of its own, or with `linked` a worktree of one
 * elsewhere, whose `.git` is a file. `lib` is on `ours`, which changed `src/gone.py`; `other` removed it, so merging
 * `other` stops on a modify/delete conflict that changes no file. With `both`, each branch also changed `src/both.py`
 * in its own place, and that merge writes it.
 */
async function nestedCheckout({ both = false, linked = false } = {}) {
  const root = await workspace();
  await writeFile(join(root, "src", "a.py"), "old\n");
  commitAll(root);
  const nested = join(root, "lib");
  const repository = linked ? await workspace() : nested;
  await mkdir(join(repository, "src"), { recursive: true });
  await writeFile(join(repository, "src", "gone.py"), "gone\n");
  await writeFile(join(repository, "src", "both.py"), "1\n2\n3\n4\n5\n6\n7\n");
  git(repository, "init", "--quiet", "--initial-branch=main");
  git(repository, "add", ".");
  git(repository, "commit", "--quiet", "-m", "seed");
  git(repository, "switch", "--quiet", "-c", "other");
  await rm(join(repository, "src", "gone.py"));
  if (both) await writeFile(join(repository, "src", "both.py"), "theirs\n2\n3\n4\n5\n6\n7\n");
  git(repository, "commit", "--quiet", "-am", "other");
  git(repository, "switch", "--quiet", "-c", "ours", "main");
  await writeFile(join(repository, "src", "gone.py"), "gone\nours\n");
  if (both) await writeFile(join(repository, "src", "both.py"), "1\n2\n3\n4\n5\n6\nours\n");
  git(repository, "commit", "--quiet", "-am", "ours");
  if (linked) {
    git(repository, "switch", "--quiet", "main");
    git(repository, "worktree", "add", "--quiet", nested, "ours");
  }
  await awaitClockAfter(join(nested, "src", "gone.py"), join(nested, "src", "both.py"));
  const extra: Partial<CodeRuleAuditor> = { sourceRoot: path => resolve(root, path).startsWith(`${nested}${sep}`) ? nested : root };
  return { root, nested, extra };
}

/** `root` (from `twoBranches`) with a worktree of `branch`, and an auditor that owns its files and walks it. */
async function withWorktree(root: string, place: string, branch = "other") {
  git(root, "worktree", "add", "--quiet", place, branch);
  const worktree = await realpath(place);
  const roots = [worktree];
  const extra: Partial<CodeRuleAuditor> = {
    sourceRoot: path => roots.find(known => resolve(root, path).startsWith(`${known}${sep}`)) ?? root,
    worktreeRoots: () => roots,
  };
  return { worktree, roots, extra };
}

describe("CodeRuleBridge and every worktree of the repository", () => {
  /** Runs `run` as a shell call, then lets the walk of the other worktrees, which the call does not wait for, find it. */
  async function shellInWorktree(bridge: CodeRuleBridge, run: () => void | Promise<void>): Promise<void> {
    await shell(bridge, run);
    await bridge.settleWorktreeWalk();
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
  }

  it("audits a shell write in a worktree outside the session's folder", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: repositoryPath(root, join(worktree, "src", "a.py")), language: "python", previousText: "old\nviolation\n", generatedText: "old\nviolation\nmine\n" }],
    ]);
  });

  it("records a shell write in another worktree in Log, so it steers no session, and steers on one in its own folder", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n"));
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits.map(edit => edit.path), batch.mode])).toEqual([
      [[repositoryPath(root, join(worktree, "src", "a.py"))], "log"],
      [["src/a.py"], undefined],
    ]);
  });

  it("measures a shell edit to a file changed before the session, in a worktree inside its folder, from the text first seen", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(root, "wt"));
    const path = join(worktree, "src", "a.py");
    await writeFile(path, "old\nviolation\nlocal\n");
    await awaitClockAfter(path);
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => writeFile(path, "old\nviolation\nlocal\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "wt/src/a.py", language: "python", previousText: "old\nviolation\nlocal\n", generatedText: "old\nviolation\nlocal\nmine\n" }],
    ]);
  });

  it("records a shell edit of a file a modify/delete conflict in another worktree left as it was, where that worktree's walk found no change", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "gone.py"), "gone\n");
    git(root, "init", "--quiet", "--initial-branch=main");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "seed");
    git(root, "switch", "--quiet", "-c", "other");
    await rm(join(root, "src", "gone.py"));
    git(root, "commit", "--quiet", "-am", "other");
    git(root, "switch", "--quiet", "-c", "kept", "main");
    await writeFile(join(root, "src", "gone.py"), "gone\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    git(root, "switch", "--quiet", "main");
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"), "kept");
    const path = join(worktree, "src", "gone.py");
    await awaitClockAfter(path);
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => { try { git(worktree, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shellInWorktree(bridge, () => writeFile(path, "gone\nours\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path: repositoryPath(root, path), language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }], "log"],
    ]);
  });

  it("audits a shell write in a worktree in a folder the session's checkout ignores", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(root, ".ignored", "w"));
    const { bridge, auditGeneratedEdits } = await opened(root, {
      ...extra, ignoresPath: (path, directory) => directory && path === ".ignored",
    });
    await shellInWorktree(bridge, () => writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits.map(edit => edit.path))).toEqual([[".ignored/w/src/a.py"]]);
  });

  it("answers a command without waiting for the walk of the other worktrees", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n");
    // The session's own walk answers the command; the worktree's change is found behind it.
    expect(await shell(bridge, () => undefined)).toEqual([]);
    await bridge.settleWorktreeWalk();
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits.map(edit => edit.path))).toEqual([repositoryPath(root, join(worktree, "src", "a.py"))]);
  });

  it("takes a worktree's first walk as its baseline, though `git worktree add` wrote it after the session opened, and records a later change", async () => {
    const root = await twoBranches();
    const roots: string[] = [];
    const changes: IgnoreRulesChange[] = [];
    const bridge = createBridge(root, {
      ...auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))),
      sourceRoot: path => roots.find(known => resolve(root, path).startsWith(`${known}${sep}`)) ?? root,
      worktreeRoots: () => roots,
    }, { worktreeWalkGapMs: 0, onIgnoreRulesChanged: change => { changes.push(change); } });
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    const place = join(await workspace(), "w");
    git(root, "worktree", "add", "--quiet", place, "other");
    const worktree = await realpath(place);
    await writeFile(join(worktree, ".gitignore"), "dist/\n");
    roots.push(worktree);

    await shellInWorktree(bridge, () => undefined);
    expect(changes).toEqual([]);
    await shellInWorktree(bridge, () => appendFile(join(worktree, ".gitignore"), "scratch/\n"));
    await shell(bridge, () => undefined);
    expect(changes).toEqual([{ path: join(worktree, ".gitignore"), via: "observed", ignoresMore: ["scratch/"] }]);
  });

  it("walks a worktree inside the session's folder once", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(root, "wt"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits.map(edit => edit.path))).toEqual(["wt/src/a.py"]);
  });

  it("leaves the other worktrees' sources, and files a tool wrote outside its folder, out of the observations it shares once it closes", async () => {
    const root = await twoBranches();
    const roots: string[] = [];
    for (const name of ["w1", "w2", "w3"]) {
      const place = join(await workspace(), name);
      git(root, "worktree", "add", "--quiet", "--detach", place, "other");
      roots.push(await realpath(place));
    }
    const sibling = join(await workspace(), "sibling");
    await mkdir(sibling);
    const observations: SourceObservationCache = new Map();
    const bridge = createBridge(root, {
      ...auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))),
      sourceRoot: path => [...roots, sibling].find(known => resolve(root, path).startsWith(`${known}${sep}`)) ?? root,
      worktreeRoots: () => roots,
    }, { worktreeWalkGapMs: 0, observations });
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await bridge.begin("call-1", "write", { path: join(sibling, "x.py") });
    await writeFile(join(sibling, "x.py"), "after\n");
    await bridge.finish("call-1");
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    const own = ["src/a.py", "src/gone.py"];
    const others = roots.flatMap(worktree => ["a.py", "new.py"].map(file => relative(root, join(worktree, "src", file)).split(sep).join("/")));
    const outside = relative(root, join(sibling, "x.py")).split(sep).join("/");
    // While the session walks them, a session opening in its folder starts from the worktrees' sources too.
    expect([...observations.get(root)?.keys() ?? []].sort()).toEqual([...others, ...own, outside].sort());
    await bridge.close();
    // The daemon keeps a folder's observations while it serves the folder, so each folder sessions started in held every
    // file of every worktree: its heap grew with the folders times the files of all the repository's worktrees.
    expect([...observations.get(root)?.keys() ?? []].sort()).toEqual(own);
  });

  it("checks a later session's edits in its folder, in a worktree and outside the folder as it did before an earlier session there closed", async () => {
    const root = await twoBranches();
    const { worktree, roots } = await withWorktree(root, join(await workspace(), "w"));
    const sibling = join(await workspace(), "sibling");
    await mkdir(sibling);
    await writeFile(join(sibling, "x.py"), "outside\n");
    // Every session in the folder starts from one observation cache, as with the runtime that serves the folder.
    const observations: SourceObservationCache = new Map();
    const open = async (sessionId: string) => {
      const audit = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
      const bridge = createBridge(root, {
        ...auditor(audit),
        sourceRoot: path => [...roots, sibling].find(known => resolve(root, path).startsWith(`${known}${sep}`)) ?? root,
        worktreeRoots: () => roots,
      }, { worktreeWalkGapMs: 0, observations });
      await bridge.openWorkspace(sessionId);
      await bridge.checkpoint("turn");
      await bridge.settleWorktreeWalk();
      // Each audit as its edits and its mode: `undefined` steers the session, `log` only records.
      return { bridge, audited: () => audit.mock.calls.map(([, batch]) => [batch.edits, batch.mode]) };
    };
    let calls = 0;
    const toolWrite = async (bridge: CodeRuleBridge, path: string, text: string) => {
      const id = `write-${++calls}`;
      await bridge.begin(id, "write", { path });
      await writeFile(path, text);
      await bridge.finish(id);
    };
    // The walk of the other worktrees, which a shell call does not wait for, finds what it wrote there.
    const shellWrite = async (bridge: CodeRuleBridge, path: string, text: string) => {
      const id = `shell-${++calls}`;
      await bridge.begin(id, "shell", { command: "./run.sh" });
      await writeFile(path, text);
      await bridge.finish(id);
      await bridge.settleWorktreeWalk();
      await bridge.checkpoint("turn");
      await bridge.settleWorktreeWalk();
    };
    const named = (absolute: string) => relative(root, absolute).split(sep).join("/");
    const edit = (path: string, previousText: string, generatedText: string) => ({ path, language: "python", previousText, generatedText });

    const first = await open("session-1");
    await toolWrite(first.bridge, join(sibling, "x.py"), "outside\nfirst\n");
    await shellWrite(first.bridge, join(worktree, "src", "new.py"), "violation\nfirst\n");
    await first.bridge.close();
    expect(first.audited()).toEqual([
      [[edit(named(join(sibling, "x.py")), "outside\n", "outside\nfirst\n")], undefined],
      [[edit(named(join(worktree, "src", "new.py")), "violation\n", "violation\nfirst\n")], "log"],
    ]);

    // Another process changes a file in each place while no session runs.
    await writeFile(join(worktree, "src", "a.py"), "old\nviolation\nbetween\n");
    await writeFile(join(root, "src", "a.py"), "old\nbetween\n");
    await writeFile(join(sibling, "x.py"), "outside\nfirst\nbetween\n");
    await awaitClockAfter(join(worktree, "src", "a.py"), join(root, "src", "a.py"), join(sibling, "x.py"));

    const second = await open("session-2");
    // What changed while no session ran is no session's edit.
    expect(second.audited()).toEqual([]);
    await toolWrite(second.bridge, join(worktree, "src", "a.py"), "old\nviolation\nbetween\nmine\n");
    await shellWrite(second.bridge, join(worktree, "src", "new.py"), "violation\nfirst\nmine\n");
    await shellWrite(second.bridge, join(root, "src", "a.py"), "old\nbetween\nmine\n");
    await toolWrite(second.bridge, join(sibling, "x.py"), "outside\nfirst\nbetween\nmine\n");
    expect(second.audited()).toEqual([
      [[edit(named(join(worktree, "src", "a.py")), "old\nviolation\nbetween\n", "old\nviolation\nbetween\nmine\n")], undefined],
      [[edit(named(join(worktree, "src", "new.py")), "violation\nfirst\n", "violation\nfirst\nmine\n")], "log"],
      [[edit("src/a.py", "old\nbetween\n", "old\nbetween\nmine\n")], undefined],
      [[edit(named(join(sibling, "x.py")), "outside\nfirst\nbetween\n", "outside\nfirst\nbetween\nmine\n")], undefined],
    ]);
  });

  it("keeps its own sources, and none of the worktrees inside its folder, in the observations it shares once it closes", async () => {
    const root = await twoBranches();
    // A folder of its own whose path starts with a worktree's: `wt/w1-notes` beside `wt/w1`.
    await mkdir(join(root, "wt", "w1-notes"), { recursive: true });
    await writeFile(join(root, "wt", "w1-notes", "a.py"), "notes\n");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "notes");
    const roots = ["w1", "w2"].map(name => join(root, "wt", name));
    for (const place of roots) git(root, "worktree", "add", "--quiet", "--detach", place, "other");
    const observations: SourceObservationCache = new Map();
    const bridge = createBridge(root, {
      ...auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))),
      sourceRoot: path => roots.find(known => resolve(root, path).startsWith(`${known}${sep}`)) ?? root,
      worktreeRoots: () => roots,
    }, { worktreeWalkGapMs: 0, observations });
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    const own = ["src/a.py", "src/gone.py", "wt/w1-notes/a.py"];
    const others = ["w1", "w2"].flatMap(name => ["a.py", "new.py"].map(file => `wt/${name}/src/${file}`));
    expect([...observations.get(root)?.keys() ?? []].sort()).toEqual([...own, ...others].sort());
    await bridge.close();
    expect([...observations.get(root)?.keys() ?? []].sort()).toEqual(own);
  });

  it("audits nothing when a worktree is removed, or of one Git adds later", async () => {
    const root = await twoBranches();
    const { worktree, roots, extra } = await withWorktree(root, join(await workspace(), "w"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => {
      git(root, "worktree", "remove", worktree);
      roots.length = 0;
    });
    const later = join(await workspace(), "later");
    await shellInWorktree(bridge, async () => {
      git(root, "worktree", "add", "--quiet", later, "other");
      roots.push(await realpath(later));
    });
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  /** The paths `audit` was given, over every call. */
  const auditedPaths = (audit: ReturnType<typeof vi.fn>): string[] =>
    audit.mock.calls.flatMap(([, batch]) => (batch as CodeRuleEditBatch).edits.map(edit => edit.path));

  it("walks again for a boundary that came while a walk was running, so a write the walk had passed is seen", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    await mkdir(join(worktree, "deep", "deeper"), { recursive: true });
    await writeFile(join(worktree, "top.py"), "old\n");
    await awaitClockAfter(join(worktree, "top.py"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    // The walk reads each level whole before the next, so this gate holds it after it has read `top.py`.
    const walk = holdDiscovery("directories", join(worktree, "deep", "deeper"));
    await bridge.checkpoint("turn");
    await walk.entered;
    await writeFile(join(worktree, "top.py"), "old\nviolation\n");
    await bridge.checkpoint("turn");
    walk.release();
    await bridge.settleWorktreeWalk();
    expect(auditedPaths(auditGeneratedEdits)).toEqual([repositoryPath(root, join(worktree, "top.py"))]);
  });

  it("takes what a walk could not read in a worktree as a baseline once it can, but for what was written since", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    await mkdir(join(worktree, "deep"));
    await writeFile(join(worktree, "deep", "old.py"), "violation\n");
    await awaitClockAfter(join(worktree, "deep", "old.py"));
    discovery.directories.set(join(worktree, "deep"), [async () => { throw new Error("EMFILE: too many open files"); }]);
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    // The gap is the session's, as the session folder's own walk reports one.
    expect((await bridge.checkpoint("turn")).failures).toEqual([
      expect.objectContaining({ code: "source-incomplete", message: expect.stringContaining("EMFILE") }),
    ]);
    await bridge.settleWorktreeWalk();
    await writeFile(join(worktree, "deep", "new.py"), "violation\n");
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    expect(auditedPaths(auditGeneratedEdits)).toEqual([repositoryPath(root, join(worktree, "deep", "new.py"))]);
    expect((await bridge.checkpoint("turn")).failures).toEqual([]);
  });

  it("makes no text of the files a worktree's first walk takes as its baseline, and records a later change from Git's text", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const files = Array.from({ length: 20 }, (_, index) => join(worktree, "src", `f${index}.py`));
    await Promise.all(files.map((file, index) => writeFile(file, `x = ${index}\n`)));
    git(worktree, "add", ".");
    git(worktree, "commit", "--quiet", "-m", "files");
    await awaitClockAfter(...files);
    decodedReads.count = 0;
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    expect(decodedReads.count).toBe(0);
    await shellInWorktree(bridge, () => writeFile(files[0]!, "x = 0\nmine\n"));
    const path = relative(root, files[0]!).split(sep).join("/");
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path, language: "python", previousText: "x = 0\n", generatedText: "x = 0\nmine\n" }], "log"],
    ]);
    // The one file that changed is the one read made into text.
    expect(decodedReads.count).toBe(1);
  });

  it("reads the files of a worktree's first walk into buffers it reuses, not a buffer of their own each", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const files = Array.from({ length: 64 }, (_, index) => join(worktree, "src", `f${index}.py`));
    await Promise.all(files.map((file, index) => writeFile(file, `x = ${index}\n`)));
    git(worktree, "add", ".");
    git(worktree, "commit", "--quiet", "-m", "files");
    await awaitClockAfter(...files);
    readBuffers.watching = true;
    await opened(root, extra);
    readBuffers.watching = false;
    // A walk runs 16 reads at once (OBSERVATION_CONCURRENCY), and two walked here: the session folder and the worktree.
    expect(readBuffers.seen.size).toBeGreaterThan(0);
    expect(readBuffers.seen.size).toBeLessThanOrEqual(2 * 16);
  });

  it("makes no text of the files a later worktree walk first sees, as its baseline, in a folder the last one could not read", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    await mkdir(join(worktree, "deep"));
    const files = Array.from({ length: 20 }, (_, index) => join(worktree, "deep", `f${index}.py`));
    await Promise.all(files.map((file, index) => writeFile(file, `x = ${index}\n`)));
    git(worktree, "add", ".");
    git(worktree, "commit", "--quiet", "-m", "files");
    await awaitClockAfter(...files);
    discovery.directories.set(join(worktree, "deep"), [async () => { throw new Error("EMFILE: too many open files"); }]);
    decodedReads.count = 0;
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    // The walk that reads the folder comes after the worktree's first: only that folder's failure makes these a baseline.
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    expect(decodedReads.count).toBe(0);
    await shellInWorktree(bridge, () => writeFile(files[0]!, "x = 0\nmine\n"));
    const path = relative(root, files[0]!).split(sep).join("/");
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path, language: "python", previousText: "x = 0\n", generatedText: "x = 0\nmine\n" }], "log"],
    ]);
  });

  it("records another writer's change in a worktree Git added during the session, made before that worktree's first walk", async () => {
    const root = await twoBranches();
    const roots: string[] = [];
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, {
      ...auditor(auditGeneratedEdits),
      sourceRoot: path => roots.find(known => resolve(root, path).startsWith(`${known}${sep}`)) ?? root,
      worktreeRoots: () => roots,
    }, { worktreeWalkGapMs: 0 });
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    const place = join(await workspace(), "w");
    git(root, "worktree", "add", "--quiet", place, "other");
    const worktree = await realpath(place);
    await writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n");
    roots.push(worktree);
    await shellInWorktree(bridge, () => undefined);
    const path = relative(root, join(worktree, "src", "a.py")).split(sep).join("/");
    // Whether Git's own write of src/new.py is recorded too depends on how its checkout wrote line breaks.
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([[expect.arrayContaining([
      { path, language: "python", previousReadError: "Source changed during initial discovery without a captured preimage", generatedText: "old\nviolation\nmine\n" },
    ]), "log"]]);
  });

  it("records a file a shell command moved into another worktree, stamped before the session, and measures its next edit from that text", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const outside = await workspace();
    await mkdir(join(outside, "pkg"));
    const moved = join(outside, "pkg", "m.py");
    await writeFile(moved, "moved\n");
    await awaitClockAfter(moved);
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => rename(join(outside, "pkg"), join(worktree, "src", "pkg")));
    await shellInWorktree(bridge, () => writeFile(join(worktree, "src", "pkg", "m.py"), "moved\nmine\n"));
    const path = relative(root, join(worktree, "src", "pkg", "m.py")).split(sep).join("/");
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path, language: "python", generatedText: "moved\n" }], "log"],
      [[{ path, language: "python", previousText: "moved\n", generatedText: "moved\nmine\n" }], "log"],
    ]);
  });

  it("reports a worktree walk that never ran as pending when the session closes, and none with no worktree to walk", async () => {
    const root = await twoBranches();
    const { extra } = await withWorktree(root, join(await workspace(), "w"));
    const pending: string[] = [];
    const bridge = createBridge(root, { ...auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), ...extra }, {
      worktreeWalkGapMs: 60_000, onDiscoveryPending: boundary => { pending.push(boundary); },
    });
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    // The next walk is timed a minute out; the session closes first.
    await bridge.checkpoint("turn");
    await bridge.close();
    expect(pending).toEqual(["close"]);

    const alone = createBridge(await twoBranches(), { ...auditor(vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch))), worktreeRoots: () => [] }, {
      worktreeWalkGapMs: 60_000, onDiscoveryPending: boundary => { pending.push(`alone:${boundary}`); },
    });
    await alone.openWorkspace("session-2");
    await alone.checkpoint("turn");
    await alone.checkpoint("turn");
    await alone.close();
    expect(pending).toEqual(["close"]);
  });

  it("reports no deletions for a nested worktree the other walk dropped while the session's walk ran", async () => {
    const root = await twoBranches();
    const { worktree, roots, extra } = await withWorktree(root, join(root, "wt"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra, 200);
    // The worktree walk this turn times runs only when the test moves the clock, once the next session walk has started.
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      await bridge.checkpoint("turn");
      await bridge.begin("shell-1", "shell", { command: "./write.sh &" });
      await writeFile(join(root, "src", "hold.py"), "mine\n");
      const walk = holdDiscovery("reads", join(root, "src", "hold.py"));
      await requestDiscovery(bridge, walk);
      git(root, "worktree", "remove", worktree);
      roots.length = 0;
      await vi.advanceTimersByTimeAsync(200);
      await bridge.settleWorktreeWalk();
      walk.release();
      await bridge.checkpoint("turn");
    } finally {
      vi.useRealTimers();
    }
    await expect(bridge.finish("shell-1")).resolves.toEqual([]);
    expect(auditedPaths(auditGeneratedEdits)).toEqual(["src/hold.py"]);
  });

  it("reports no deletions for a worktree the session's walk dropped while the worktree walk ran", async () => {
    const root = await twoBranches();
    const { worktree, roots, extra } = await withWorktree(root, join(await workspace(), "w"));
    await mkdir(join(worktree, "deep"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    const walk = holdDiscovery("directories", join(worktree, "deep"));
    await bridge.checkpoint("turn");
    await walk.entered;
    git(root, "worktree", "remove", "--force", worktree);
    roots.length = 0;
    await bridge.checkpoint("turn");
    walk.release();
    await bridge.settleWorktreeWalk();
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("records only the deletions of a worktree nested in another walked worktree and removed during its first walk", async () => {
    const root = await twoBranches();
    const { worktree, roots, extra } = await withWorktree(root, join(await workspace(), "w"));
    git(root, "worktree", "add", "--quiet", "-b", "nested", join(worktree, "nest"), "main");
    const nested = await realpath(join(worktree, "nest"));
    // A walk opens one level of folders at a time, so it opens this folder of the outer worktree after it has read the
    // nested worktree's files: a gate here holds the walk between those reads and the audit of what it found.
    const deepest = join(worktree, "deep", "deeper", "deepest");
    await mkdir(deepest, { recursive: true });
    await awaitClockAfter(join(nested, "src", "a.py"), join(nested, "src", "gone.py"));
    const nest = relative(root, nested).split(sep).join("/");
    const { bridge, auditGeneratedEdits } = await opened(root, {
      ...extra,
      sourceRoot: path => roots.filter(known => resolve(root, path).startsWith(`${known}${sep}`)).sort((a, b) => b.length - a.length)[0] ?? root,
      // The outer worktree ignores the folder that holds the nested one, as a main checkout often ignores its
      // worktrees' folder, so the outer worktree's walk never reads the nested worktree's files.
      ignoresPath: (path, directory) => directory && path === nest,
    });
    roots.push(nested);
    const walk = holdDiscovery("directories", deepest);
    await bridge.checkpoint("turn");
    await walk.entered;
    git(root, "worktree", "remove", "--force", nested);
    roots.splice(roots.indexOf(nested), 1);
    // The session folder's walk finds the worktree gone: the held walk no longer takes its files as a baseline.
    await bridge.checkpoint("turn");
    walk.release();
    await bridge.settleWorktreeWalk();
    await bridge.checkpoint("turn");
    await bridge.settleWorktreeWalk();
    // No one wrote these files. A walk that kept their text recorded each one created, then deleted.
    const deleted = { language: "python", previousReadError: "Source was deleted without a captured preimage", deleted: true };
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits, batch.mode])).toEqual([
      [[{ path: `${nest}/src/a.py`, ...deleted }, { path: `${nest}/src/gone.py`, ...deleted }], "log"],
    ]);
  });

  it("reports no deletions for a worktree removed while its own walk ran, with no other walk between", async () => {
    const root = await twoBranches();
    const { worktree, roots, extra } = await withWorktree(root, join(await workspace(), "w"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    const walk = holdDiscovery("directories", join(worktree, "src"));
    await bridge.checkpoint("turn");
    await walk.entered;
    git(root, "worktree", "remove", worktree);
    roots.length = 0;
    walk.release();
    await bridge.settleWorktreeWalk();
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });
});

describe("CodeRuleBridge and Git's own writes under core.autocrlf", () => {
  /** `twoBranches` with `core.autocrlf=true`, as Git for Windows' installer sets it, its files checked out again in CRLF. */
  async function crlfBranches(): Promise<string> {
    const root = await twoBranches();
    git(root, "config", "core.autocrlf", "true");
    await rm(join(root, "src", "a.py"));
    await rm(join(root, "src", "gone.py"));
    git(root, "checkout", "--", ".");
    await awaitClockAfter(join(root, "src", "a.py"), join(root, "src", "gone.py"));
    return root;
  }

  it("audits nothing a checkout wrote with CRLF line breaks", async () => {
    const root = await crlfBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await expect(shell(bridge, () => git(root, "checkout", "--quiet", "other"))).resolves.toEqual([]);
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("audits nothing restored from another branch with CRLF line breaks", async () => {
    const root = await crlfBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await expect(shell(bridge, () => git(root, "checkout", "other", "--", "src/a.py"))).resolves.toEqual([]);
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("audits a shell write with an LF line a checkout never writes, though another branch holds its text with LF", async () => {
    const root = await crlfBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    // `other` holds "old\nviolation\n", which a checkout here writes with a CRLF after each line.
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\r\nviolation\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\r\n", generatedText: "old\r\nviolation\n" }],
    ]);
  });

  it("gives a shell write the preimage Git holds, in the CRLF line breaks the file held it in", async () => {
    const root = await crlfBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\r\nmine\r\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\r\n", generatedText: "old\r\nmine\r\n" }],
    ]);
  });

  it("audits a shell write that gives an LF file CRLF line breaks where Git would never write them", async () => {
    const root = await twoBranches();
    // As on macOS and Linux, and on Windows set up to check files out as they are stored.
    git(root, "config", "core.autocrlf", "false");
    git(root, "config", "core.eol", "lf");
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\r\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\n", generatedText: "old\r\n" }],
    ]);
  });

  it("credits Git with CRLF line breaks an eol attribute or a text file under core.eol=crlf has it write", async () => {
    for (const setup of [
      { attributes: "*.py eol=crlf\n", eol: "lf" },
      { attributes: "*.py text\n", eol: "crlf" },
    ]) {
      const root = await twoBranches();
      git(root, "config", "core.autocrlf", "false");
      git(root, "config", "core.eol", setup.eol);
      await writeFile(join(root, ".git", "info", "attributes"), setup.attributes);
      await rm(join(root, "src", "a.py"));
      git(root, "checkout", "--", "src/a.py");
      await awaitClockAfter(join(root, "src", "a.py"));
      const { bridge, auditGeneratedEdits } = await opened(root);
      await expect(shell(bridge, () => git(root, "checkout", "--quiet", "other")), setup.attributes).resolves.toEqual([]);
      expect(auditGeneratedEdits, setup.attributes).not.toHaveBeenCalled();
    }
  });

  it("credits Git with the CRLF line breaks a text or eol=crlf attribute has it write into a blob that already holds a CR", async () => {
    for (const setup of [
      { attributes: "*.py eol=crlf\n", autocrlf: "false" },
      // Git for Windows' default core.autocrlf with a text attribute.
      { attributes: "*.py text\n", autocrlf: "true" },
    ]) {
      const root = await twoBranches();
      // Committed with no attribute, so the blobs keep their CRs: a CRLF beside an LF, and a lone CR.
      git(root, "switch", "--quiet", "other");
      await writeFile(join(root, "src", "a.py"), "old\r\nviolation\n");
      await writeFile(join(root, "src", "new.py"), "violation\rmore\n");
      git(root, "commit", "--quiet", "-am", "carriage returns");
      git(root, "switch", "--quiet", "main");
      git(root, "config", "core.autocrlf", setup.autocrlf);
      await writeFile(join(root, ".git", "info", "attributes"), setup.attributes);
      await rm(join(root, "src", "a.py"));
      git(root, "checkout", "--", "src/a.py");
      await awaitClockAfter(join(root, "src", "a.py"));
      const { bridge, auditGeneratedEdits } = await opened(root);
      // A checkout writes every LF without its CR as CRLF here, whatever else the blob holds.
      await expect(shell(bridge, () => git(root, "checkout", "--quiet", "other")), setup.attributes).resolves.toEqual([]);
      expect(auditGeneratedEdits, setup.attributes).not.toHaveBeenCalled();
    }
  });
});

describe("CodeRuleBridge and Git's own writes", () => {
  it.each([
    ["checkout", ["checkout", "--quiet", "other"]],
    ["switch", ["switch", "--quiet", "other"]],
    ["a fast-forward merge", ["merge", "--quiet", "--ff-only", "other"]],
    ["a rebase", ["rebase", "--quiet", "other"]],
    ["a hard reset", ["reset", "--quiet", "--hard", "other"]],
  ])("audits nothing %s rewrote", async (_name, command) => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await expect(shell(bridge, () => git(root, ...command))).resolves.toEqual([]);
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it.each([
    ["a shell redirect of its blob", (root: string) => writeFile(join(root, "src", "a.py"), execFileSync("git", ["show", "other:src/a.py"], { cwd: root }))],
    ["a shell redirect of a remote-tracking branch's blob", async (root: string) => {
      git(root, "update-ref", "refs/remotes/origin/other", "other");
      git(root, "branch", "--quiet", "-D", "other");
      await writeFile(join(root, "src", "a.py"), execFileSync("git", ["show", "origin/other:src/a.py"], { cwd: root }));
    }],
    ["a checkout of its path", (root: string) => git(root, "checkout", "other", "--", "src/a.py")],
    ["a restore from its source", (root: string) => git(root, "restore", "--source", "other", "src/a.py")],
    // Rerail cannot tell a write of the same bytes from a restore, and takes it for one (docs/reference.md).
    ["a plain write of exactly its text", (root: string) => writeFile(join(root, "src", "a.py"), "old\nviolation\n")],
  ])("audits nothing restored from another branch by %s, and measures the next shell edit from it", async (_name, restore) => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await expect(shell(bridge, () => restore(root))).resolves.toEqual([]);
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nviolation\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\nviolation\n", generatedText: "old\nviolation\nmine\n" }],
    ]);
  });

  it("audits a file restored from a branch its own command committed it to", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, async () => {
      await writeFile(join(root, "src", "a.py"), "old\nmine\n");
      git(root, "commit", "--quiet", "-am", "mine");
      git(root, "branch", "backup");
    });
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\n", generatedText: "old\nmine\n" }],
    ]);
  });

  it("audits text a branch holds only at another path", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => writeFile(join(root, "src", "b.py"), "old\nviolation\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/b.py", language: "python", generatedText: "old\nviolation\n" }],
    ]);
  });

  it("reports only its own line for a shell edit after a checkout", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => git(root, "checkout", "--quiet", "other"));
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nviolation\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\nviolation\n", generatedText: "old\nviolation\nmine\n" }],
    ]);
  });

  it("does not take another session's checkout for this session's writes", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    git(root, "checkout", "--quiet", "other");
    await bridge.checkpoint("turn");
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("keeps auditing a file written and committed in one command, against the tree before the commit", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, async () => {
      await writeFile(join(root, "src", "a.py"), "old\nmine\n");
      git(root, "commit", "--quiet", "-am", "mine");
    });
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\n", generatedText: "old\nmine\n" }],
    ]);
  });

  it("measures a file written in the command that moved the checkout from what Git wrote", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, async () => {
      git(root, "checkout", "--quiet", "other");
      await writeFile(join(root, "src", "a.py"), "old\nviolation\nmine\n");
    });
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\nviolation\n", generatedText: "old\nviolation\nmine\n" }],
    ]);
  });

  it("still tells the auditor, with no generated lines and in log mode, of a file this session had changed", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nmine\n"));
    await shell(bridge, () => git(root, "restore", "src/a.py"));
    expect(auditGeneratedEdits.mock.calls.at(-1)?.[1]).toMatchObject({
      mode: "log",
      edits: [{ path: "src/a.py", language: "python", previousText: "old\n", generatedText: "old\n" }],
    });
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(2);
  });

  it("audits nothing a merge wrote when it stopped on a conflict, and a later shell edit from the conflicted text", async () => {
    const root = await twoBranches();
    await writeFile(join(root, "src", "a.py"), "old\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "a.py"));
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nresolved\n"));
    const [edit] = auditGeneratedEdits.mock.calls.at(-1)![1].edits;
    expect(edit).toMatchObject({ path: "src/a.py", generatedText: "old\nresolved\n" });
    expect(edit?.previousText).toContain("<<<<<<<");
    expect(edit?.previousReadError).toBeUndefined();
  });

  it("audits nothing a merge wrote when it stopped on a modify/delete conflict, and a later shell edit from our text", async () => {
    const root = await twoBranches();
    await writeFile(join(root, "src", "gone.py"), "gone\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "gone.py"));
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shell(bridge, () => writeFile(join(root, "src", "gone.py"), "gone\nours\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/gone.py", language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }],
    ]);
  });

  it("audits a shell edit of a conflicted file that holds our text again, from that text", async () => {
    const root = await twoBranches();
    await writeFile(join(root, "src", "a.py"), "old\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "a.py"));
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    await shell(bridge, () => git(root, "checkout", "--ours", "src/a.py"));
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nours\nmine\n"));
    const batch = auditGeneratedEdits.mock.calls.at(-1)![1];
    expect(batch.mode).not.toBe("log");
    expect(batch.edits).toEqual([{ path: "src/a.py", language: "python", previousText: "old\nours\n", generatedText: "old\nours\nmine\n" }]);
  });

  it("keeps the last unmerged entries it read through a pass where Git could not list them", async () => {
    const root = await twoBranches();
    await writeFile(join(root, "src", "a.py"), "old\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "a.py"));
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    // The index changes, so the pass asks Git for the entries, and Git fails.
    asyncGitCalls.failing = "--unmerged";
    try {
      await shell(bridge, async () => {
        git(root, "checkout", "--ours", "src/a.py");
        const later = new Date(Date.now() + 5_000);
        await utimes(join(root, ".git", "index"), later, later);
      });
    } finally {
      asyncGitCalls.failing = undefined;
    }
    expect(asyncGitCalls.args.some(args => args.includes("--no-such-option"))).toBe(true);
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nours\nmine\n"));
    const batch = auditGeneratedEdits.mock.calls.at(-1)![1];
    expect(batch.mode).not.toBe("log");
    expect(batch.edits).toEqual([{ path: "src/a.py", language: "python", previousText: "old\nours\n", generatedText: "old\nours\nmine\n" }]);
  });

  it("takes a second conflict on a file still unmerged since the last walk as Git's", async () => {
    const root = await twoBranches();
    git(root, "switch", "--quiet", "-c", "second");
    await writeFile(join(root, "src", "a.py"), "old\nsecond\n");
    git(root, "commit", "--quiet", "-am", "second");
    git(root, "switch", "--quiet", "main");
    await writeFile(join(root, "src", "a.py"), "old\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "a.py"));
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nresolved\n"));
    await shell(bridge, () => {
      git(root, "add", ".");
      git(root, "commit", "--quiet", "-m", "merged");
      try { git(root, "merge", "--quiet", "second"); } catch { /* The conflict is the point. */ }
    });
    const batch = auditGeneratedEdits.mock.calls.at(-1)![1];
    expect(batch.mode).toBe("log");
    expect(batch.edits[0]?.generatedText).toContain("<<<<<<<");
  });

  it("starts no Git process for the unmerged entries until the index file changes", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(root);
    // A pass that finds nothing, then one that finds a shell edit: neither wrote the index.
    expect(await gitReads(() => bridge.checkpoint("turn"))).toEqual({ heads: 1, entries: 0 });
    expect((await gitReads(() => shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nmine\n")))).entries).toBe(0);
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    // The pass after an index write reads it once, and the one after that does not.
    expect((await gitReads(() => shell(bridge, () => git(root, "add", "src/a.py")))).entries).toBe(1);
    expect(await gitReads(() => bridge.checkpoint("turn"))).toEqual({ heads: 1, entries: 0 });
  });

  it("reads one checkout each pass for a session opened in a subfolder of it, once a walk has found a change there too", async () => {
    const root = await twoBranches();
    const { bridge, auditGeneratedEdits } = await opened(join(root, "src"), { sourceRoot: () => root });
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nmine\n"));
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    asyncGitCalls.args.length = 0;
    await bridge.checkpoint("turn");
    expect(asyncGitCalls.args.filter(args => args.includes("--verify"))).toHaveLength(1);
  });

  it("audits a shell edit of a file a modify/delete conflict left as it was, where the conflict's pass found no change", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "gone.py"), "gone\n");
    git(root, "init", "--quiet", "--initial-branch=main");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "seed");
    git(root, "switch", "--quiet", "-c", "other");
    await rm(join(root, "src", "gone.py"));
    git(root, "commit", "--quiet", "-am", "other");
    git(root, "switch", "--quiet", "main");
    await writeFile(join(root, "src", "gone.py"), "gone\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "gone.py"));
    const { bridge, auditGeneratedEdits } = await opened(root);
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    await shell(bridge, () => writeFile(join(root, "src", "gone.py"), "gone\nours\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/gone.py", language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }],
    ]);
    expect(auditGeneratedEdits.mock.calls[0]?.[1].mode).not.toBe("log");
  });

  it("audits a shell edit of a file a modify/delete conflict left as it was, in a session opened in a subfolder of the checkout", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "gone.py"), "gone\n");
    git(root, "init", "--quiet", "--initial-branch=main");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "seed");
    git(root, "switch", "--quiet", "-c", "other");
    await rm(join(root, "src", "gone.py"));
    git(root, "commit", "--quiet", "-am", "other");
    git(root, "switch", "--quiet", "main");
    await writeFile(join(root, "src", "gone.py"), "gone\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    await awaitClockAfter(join(root, "src", "gone.py"));
    const { bridge, auditGeneratedEdits } = await opened(join(root, "src"), { sourceRoot: () => root });
    await shell(bridge, () => { try { git(root, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    await shell(bridge, () => writeFile(join(root, "src", "gone.py"), "gone\nours\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "gone.py", language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }],
    ]);
  });

  it("takes no conflict a checkout already had when it is first read for a merge that just stopped", async () => {
    const root = await twoBranches();
    git(root, "switch", "--quiet", "-c", "ours");
    await writeFile(join(root, "src", "a.py"), "old\nours\n");
    git(root, "commit", "--quiet", "-am", "ours");
    git(root, "switch", "--quiet", "main");
    // A checkout inside the session's folder, stopped on a conflict in `src/a.py` before the session opens.
    const worktree = join(root, "wt");
    git(root, "worktree", "add", "--quiet", "wt", "ours");
    try { git(worktree, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ }
    await writeFile(join(worktree, "src", "new.py"), "staged\n");
    git(worktree, "add", "src/new.py");
    await writeFile(join(worktree, "src", "new.py"), "staged\nlocal\n");
    await awaitClockAfter(join(worktree, "src", "new.py"));
    // Git cannot list the entries while the session opens, so no walk has that checkout's until the pass that finds a change in it.
    asyncGitCalls.failing = "--unmerged";
    const { bridge, auditGeneratedEdits } = await opened(root, {
      sourceRoot: path => resolve(root, path).startsWith(`${worktree}${sep}`) ? worktree : root,
    }).finally(() => { asyncGitCalls.failing = undefined; });
    // That pass is the first read of its entries: the file holding its index blob is no work of the merge.
    await shell(bridge, () => writeFile(join(worktree, "src", "new.py"), "staged\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "wt/src/new.py", language: "python", previousText: "staged\nlocal\n", generatedText: "staged\n" }],
    ]);
  });

  it("audits a shell edit of a file a modify/delete conflict left as it was, in a checkout inside the session's folder", async () => {
    const { root, nested, extra } = await nestedCheckout();
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shell(bridge, () => { try { git(nested, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
    await shell(bridge, () => writeFile(join(nested, "src", "gone.py"), "gone\nours\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "lib/src/gone.py", language: "python", previousText: "gone\nours\n", generatedText: "gone\nours\nmine\n" }],
    ]);
    expect(auditGeneratedEdits.mock.calls[0]?.[1].mode).not.toBe("log");
  });

  it("audits nothing a merge wrote beside a modify/delete conflict, in a worktree of another repository inside the session's folder", async () => {
    const { root, nested, extra } = await nestedCheckout({ both: true, linked: true });
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shell(bridge, () => { try { git(nested, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    expect(await readFile(join(nested, "src", "both.py"), "utf8")).toBe("theirs\n2\n3\n4\n5\n6\nours\n");
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("starts no Git process for a checkout inside the session's folder until its index file changes", async () => {
    const { root, nested, extra } = await nestedCheckout();
    const { bridge } = await opened(root, extra);
    // A pass that finds nothing reads the session checkout's `HEAD`, and nothing of the one inside it.
    expect(await gitReads(() => bridge.checkpoint("turn"))).toEqual({ heads: 1, entries: 0 });
    // The pass after a write to its index lists its entries once, and the one after that does not.
    const stopped = () => shell(bridge, () => { try { git(nested, "merge", "--quiet", "other"); } catch { /* The conflict is the point. */ } });
    expect(await gitReads(stopped)).toEqual({ heads: 1, entries: 1 });
    expect(await gitReads(() => bridge.checkpoint("turn"))).toEqual({ heads: 1, entries: 0 });
  });

  it("audits nothing of a worktree added inside the session's folder", async () => {
    const root = await twoBranches();
    const worktree = join(root, "wt");
    const { bridge, auditGeneratedEdits } = await opened(root, {
      sourceRoot: path => resolve(root, path).startsWith(`${worktree}${sep}`) ? worktree : root,
    });
    await shell(bridge, () => git(root, "worktree", "add", "--quiet", "wt", "other"));
    expect(auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("reports only its own line for a shell edit in a worktree inside the session's folder, from that worktree's blob", async () => {
    const root = await twoBranches();
    const worktree = join(root, "wt");
    const { bridge, auditGeneratedEdits } = await opened(root, {
      sourceRoot: path => resolve(root, path).startsWith(`${worktree}${sep}`) ? worktree : root,
    });
    await shell(bridge, () => git(root, "worktree", "add", "--quiet", "wt", "other"));
    await shell(bridge, () => writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "wt/src/a.py", language: "python", previousText: "old\nviolation\n", generatedText: "old\nviolation\nmine\n" }],
    ]);
  });
});

describe("CodeRuleBridge and another session's writes", () => {
  /** Two sessions open on `root` in this process, sharing its observations as the runtime does, with their baselines taken. */
  async function twoSessions(root: string) {
    const observations: SourceObservationCache = new Map();
    const open = async (sessionId: string) => {
      const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
      const bridge = createBridge(root, auditor(auditGeneratedEdits), { observations });
      await bridge.openWorkspace(sessionId);
      await bridge.checkpoint("turn");
      return { bridge, auditGeneratedEdits };
    };
    return [await open("session-a"), await open("session-b")] as const;
  }
  const mine = { path: "src/a.py", language: "python", previousText: "old\n", generatedText: "old\nmine\n" };
  /**
   * Waits until the filesystem clock has passed every window sampled so far: on Linux a window's end can share the next
   * write's tick, so what is written next is stamped strictly after the last call ended.
   */
  async function awaitClockPastNow(): Promise<void> {
    const probe = await mkdtemp(join(tmpdir(), "rerail-policy-now-"));
    directories.push(probe);
    await awaitClockAfter(probe);
  }

  it("checks a shell write for the session that ran it, and not for one whose read-only command or turn end came after", async () => {
    const root = await twoBranches();
    const [a, b] = await twoSessions(root);
    await shell(a.bridge, () => appendFile(join(root, "src", "a.py"), "mine\n"));
    await awaitClockPastNow();
    await expect(shell(b.bridge, () => undefined)).resolves.toEqual([]);
    await b.bridge.checkpoint("turn");
    expect(a.auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([[mine]]);
    expect(b.auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("does not take a write made while another session's command runs for its own", async () => {
    const root = await twoBranches();
    const [a, b] = await twoSessions(root);
    await a.bridge.begin("shell-a", "shell", { command: "./long.sh" });
    await appendFile(join(root, "src", "a.py"), "mine\n");
    await awaitClockPastNow();
    await shell(b.bridge, () => undefined);
    await a.bridge.finish("shell-a");
    expect(b.auditGeneratedEdits).not.toHaveBeenCalled();
    expect(a.auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([[mine]]);
  });

  it("tells nobody of a file only the other session changed, however often it changes it", async () => {
    const root = await twoBranches();
    const [a, b] = await twoSessions(root);
    for (const line of ["first\n", "second\n", "third\n"]) {
      await shell(a.bridge, () => appendFile(join(root, "src", "a.py"), line));
      await awaitClockPastNow();
      await shell(b.bridge, () => undefined);
    }
    await b.bridge.checkpoint("turn");
    expect(b.auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("measures a later shell edit of the file from the text the other session left once a walk has seen it", async () => {
    const root = await twoBranches();
    const [a, b] = await twoSessions(root);
    await shell(a.bridge, () => appendFile(join(root, "src", "a.py"), "mine\n"));
    await awaitClockPastNow();
    await shell(b.bridge, () => undefined);
    await shell(b.bridge, () => appendFile(join(root, "src", "a.py"), "theirs\n"));
    expect(b.auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\nmine\n", generatedText: "old\nmine\ntheirs\n" }],
    ]);
  });

  it("tells the auditor, with no generated lines and in log mode, of a file it had changed that the other session then changed", async () => {
    const root = await twoBranches();
    const [a, b] = await twoSessions(root);
    await shell(b.bridge, () => appendFile(join(root, "src", "a.py"), "theirs\n"));
    await awaitClockPastNow();
    await shell(a.bridge, () => appendFile(join(root, "src", "a.py"), "mine\n"));
    await b.bridge.checkpoint("turn");
    expect(b.auditGeneratedEdits.mock.calls.at(-1)?.[1]).toMatchObject({
      mode: "log",
      edits: [{ path: "src/a.py", language: "python", previousText: "old\ntheirs\nmine\n", generatedText: "old\ntheirs\nmine\n" }],
    });
    expect(b.auditGeneratedEdits).toHaveBeenCalledTimes(2);
  });

  it("takes a removal for the session whose command made it, by its folder's stamp", async () => {
    const root = await twoBranches();
    const [a, b] = await twoSessions(root);
    await shell(a.bridge, () => rm(join(root, "src", "gone.py")));
    await awaitClockPastNow();
    await shell(b.bridge, () => undefined);
    expect(a.auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/gone.py", language: "python", previousReadError: "Source was deleted without a captured preimage", deleted: true }],
    ]);
    expect(b.auditGeneratedEdits).not.toHaveBeenCalled();
  });

  it("takes nothing written while none of its calls ran for its own, and measures its next shell edit of the file from it", async () => {
    const root = await twoBranches();
    const [, b] = await twoSessions(root);
    // A process a command left running, or an editor: no call of the session ran.
    await appendFile(join(root, "src", "a.py"), "background\n");
    await awaitClockPastNow();
    await b.bridge.checkpoint("turn");
    expect(b.auditGeneratedEdits).not.toHaveBeenCalled();
    await shell(b.bridge, () => appendFile(join(root, "src", "a.py"), "mine\n"));
    expect(b.auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousText: "old\nbackground\n", generatedText: "old\nbackground\nmine\n" }],
    ]);
  });

  it("takes what a call whose result never came wrote before the turn's end, and nothing written after it", async () => {
    const root = await twoBranches();
    const [, b] = await twoSessions(root);
    // Interrupted: begun, never finished.
    await b.bridge.begin("shell-1", "shell", { command: "./run.sh" });
    await appendFile(join(root, "src", "a.py"), "mine\n");
    await b.bridge.checkpoint("turn");
    expect(b.auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.mode, ...batch.edits])).toEqual([[undefined, mine]]);
    await awaitClockPastNow();
    await appendFile(join(root, "src", "a.py"), "theirs\n");
    await awaitClockPastNow();
    await b.bridge.checkpoint("turn");
    expect(b.auditGeneratedEdits.mock.calls.at(-1)?.[1]).toMatchObject({
      mode: "log",
      edits: [{ path: "src/a.py", language: "python", previousText: "old\nmine\ntheirs\n", generatedText: "old\nmine\ntheirs\n" }],
    });
  });

  it("credits a call answered while the evaluator opened with what was written as it ran, and nothing written after", async () => {
    const root = await twoBranches();
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const indexing = Promise.withResolvers<void>();
    const entered = Promise.withResolvers<void>();
    const bridge = createBridge(root, {
      ...auditor(auditGeneratedEdits),
      openWorkspace: async () => {
        entered.resolve();
        await indexing.promise;
      },
    });
    const open = bridge.openWorkspace("session-1");
    passes.push(open);
    // The runtime answers the agent's calls without the bridge until the evaluator opens.
    await entered.promise;
    await bridge.unwatchedCall("shell-1");
    await appendFile(join(root, "src", "a.py"), "mine\n");
    await bridge.unwatchedResult("shell-1");
    await awaitClockPastNow();
    await writeFile(join(root, "src", "new.py"), "theirs\n");
    await awaitClockPastNow();
    indexing.resolve();
    await open;
    await bridge.checkpoint("turn");
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => batch.edits)).toEqual([
      [{ path: "src/a.py", language: "python", previousReadError: "Source changed during initial discovery without a captured preimage", generatedText: "old\nmine\n" }],
    ]);
  });
});

describe("ToolCallWindows", () => {
  it("forgets the calls a walk answered for, never one still running, and ends an interrupted call at a turn's end", () => {
    const windows = new ToolCallWindows();
    windows.begin("answered", 10);
    windows.finish("answered", 20);
    windows.begin("late", 25);
    windows.finish("late", 40);
    windows.begin("running", 30);
    windows.answered(30);
    expect(windows.ranDuring(15, 15)).toBe(false);
    expect(windows.ranDuring(35, 35)).toBe(true);
    expect(windows.ranDuring(1_000, 1_000)).toBe(true);
    windows.finishAll(50);
    expect(windows.ranDuring(45, 45)).toBe(true);
    expect(windows.ranDuring(60, 60)).toBe(false);
  });

  it(`keeps the newest ${MAX_FINISHED_CALLS} finished calls`, () => {
    const windows = new ToolCallWindows();
    for (let call = 0; call <= MAX_FINISHED_CALLS; call += 1) {
      windows.begin(`call-${call}`, call * 10);
      windows.finish(`call-${call}`, call * 10 + 1);
    }
    expect(windows.ranDuring(0, 0)).toBe(false);
    expect(windows.ranDuring(10, 10)).toBe(true);
    expect(windows.ranDuring(MAX_FINISHED_CALLS * 10, MAX_FINISHED_CALLS * 10)).toBe(true);
  });
});

describe("the file system's clock", () => {
  afterEach(() => {
    vi.mocked(rm).mockReset();
    vi.useRealTimers();
  });

  it("is sampled from its probe folder even while Windows holds that folder for a moment", async () => {
    const { rm: actualRm } = await vi.importActual<typeof FileSystem>("node:fs/promises");
    // Date.now() far from the file system's clock, so a sample that fell back to it shows.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(0);
    const refused = holdRemovals(vi.mocked(rm), actualRm, 2, path => /rerail-clock-[^\\/]+$/u.test(path));

    const clock = await filesystemClock();

    expect(refused).toHaveLength(2);
    expect(clock).toBeGreaterThan(0);
    expect(existsSync(refused[0]!)).toBe(false);
  });
});
