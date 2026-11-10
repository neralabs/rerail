import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFile, lstat, mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import type * as FileSystem from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, resolve, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CodeRuleBridge,
  generatedEditPaths,
  isCommandTool,
  ledgerPreimage,
  type SourceObservationCache,
} from "../../../src/engine/code-rule/code-rule-bridge.js";
import type { CodeRuleEditBatch, CodeRuleAuditor, CodeRuleResponse } from "../../../src/engine/code-rule/code-rule.js";
import type { IgnoreRulesChange } from "../../../src/engine/code-rule/ignore-rule-files.js";
import { codeRuleDialectForPath } from "../../../src/engine/code-rule/code-rule-languages.js";

const discovery = vi.hoisted(() => ({
  reads: new Map<string, Array<() => Promise<void>>>(),
  directories: new Map<string, Array<() => Promise<void>>>(),
}));
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  return {
    ...actual,
    readFile: async (...args: Parameters<typeof actual.readFile>) => {
      const contents = await actual.readFile(...args);
      await discovery.reads.get(String(args[0]))?.shift()?.();
      return contents;
    },
    opendir: async (...args: Parameters<typeof actual.opendir>) => {
      await discovery.directories.get(String(args[0]))?.shift()?.();
      return await actual.opendir(...args);
    },
  };
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
  await Promise.all(passes.splice(0));
  await Promise.all(bridges.splice(0).map(bridge => bridge.close()));
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })));
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
      // The session saw the file as its tool edit left it: that is the external edit's preimage.
      ["session-1", {
        revision: 2,
        checkpoints: ["final"],
        edits: [{ path: "src/a.py", language: "python", previousText: "tool edit\n", generatedText: "external edit\n" }],
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

  it("gives a shell write the preimage Git or the session holds only when its digest is the file's before the write", async () => {
    const root = await workspace();
    const clean = join(root, "src", "clean.py");
    const dirty = join(root, "src", "dirty.py");
    await writeFile(clean, "one\n");
    await writeFile(dirty, "committed\n");
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    execFileSync("git", ["add", "."], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
    // Changed before the session and never staged: neither the index nor HEAD holds what the session first saw.
    await writeFile(dirty, "uncommitted\n");
    await awaitClockAfter(clean, dirty);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    await bridge.openWorkspace("session-1");
    await bridge.checkpoint("turn");

    await bridge.begin("shell-1", "shell", { command: "./rewrite.sh" });
    await writeFile(clean, "one\ntwo\n");
    await writeFile(dirty, "uncommitted\nmore\n");
    await bridge.finish("shell-1");
    expect(auditGeneratedEdits.mock.calls.at(-1)?.[1].edits).toEqual([
      { path: "src/clean.py", language: "python", previousText: "one\n", generatedText: "one\ntwo\n" },
      {
        path: "src/dirty.py", language: "python",
        previousReadError: "Source changed outside a tool with a captured preimage", generatedText: "uncommitted\nmore\n",
      },
    ]);

    // The session saw what the first write made of the file, so the next write knows its preimage.
    await bridge.begin("shell-2", "shell", { command: "./rewrite.sh" });
    await writeFile(dirty, "uncommitted\nmore\nagain\n");
    await bridge.finish("shell-2");
    expect(auditGeneratedEdits.mock.calls.at(-1)?.[1].edits).toEqual([
      { path: "src/dirty.py", language: "python", previousText: "uncommitted\nmore\n", generatedText: "uncommitted\nmore\nagain\n" },
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

  it("reports uncaptured shell writes made before baseline discovery reaches their files", async () => {
    const root = await workspace();
    await writeFile(join(root, "src", "a.py"), "existing before startup\n");
    await writeFile(join(root, "src", "untouched.py"), "existing unchanged\n");
    await awaitClockAfter(join(root, "src", "a.py"), join(root, "src", "untouched.py"));
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    const baseline = holdDiscovery("directories", root);
    await bridge.openWorkspace("session-1");
    await baseline.entered;
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

  it("discovers a shell write that races the startup snapshot read", async () => {
    const root = await workspace();
    const path = join(root, "src", "a.py");
    await writeFile(path, "before\n");
    await awaitClockAfter(path);
    const auditGeneratedEdits = vi.fn(async (_sessionId: string, batch: CodeRuleEditBatch) => response(batch));
    const bridge = createBridge(root, auditor(auditGeneratedEdits));
    const reading = holdDiscovery("reads", path);
    await bridge.openWorkspace("session-1");
    await reading.entered;
    await writeFile(path, "after\n");
    reading.release();
    await bridge.checkpoint("turn");
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-1", expect.objectContaining({
      edits: [expect.objectContaining({ path: "src/a.py", generatedText: "after\n" })],
    }));
    await bridge.checkpoint("turn");
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

    await writeFile(join(root, "src", "a.py"), "after again\n");
    await second.checkpoint("turn");
    expect(auditGeneratedEdits).toHaveBeenCalledTimes(1);
    expect(auditGeneratedEdits).toHaveBeenCalledWith("session-2", expect.objectContaining({
      edits: [expect.objectContaining({ path: "src/a.py", previousReadError: expect.any(String), generatedText: "after again\n" })],
    }));
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
    await rm(join(root, "src", "a.py"), { recursive: true });
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
/** A repository on `main`, with a branch `other` ahead of it: it changes `src/a.py`, adds `src/new.py` and removes `src/gone.py`. */
async function twoBranches(): Promise<string> {
  const root = await workspace();
  await writeFile(join(root, "src", "a.py"), "old\n");
  await writeFile(join(root, "src", "gone.py"), "gone\n");
  git(root, "init", "--quiet", "--initial-branch=main");
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

/** `root` (from `twoBranches`) with a worktree of branch `other`, and an auditor that owns its files and walks it. */
async function withWorktree(root: string, place: string) {
  git(root, "worktree", "add", "--quiet", place, "other");
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
      [{ path: relative(root, join(worktree, "src", "a.py")), language: "python", previousText: "old\nviolation\n", generatedText: "old\nviolation\nmine\n" }],
    ]);
  });

  it("records a shell write in another worktree in Log, so it steers no session, and steers on one in its own folder", async () => {
    const root = await twoBranches();
    const { worktree, extra } = await withWorktree(root, join(await workspace(), "w"));
    const { bridge, auditGeneratedEdits } = await opened(root, extra);
    await shellInWorktree(bridge, () => writeFile(join(worktree, "src", "a.py"), "old\nviolation\nmine\n"));
    await shell(bridge, () => writeFile(join(root, "src", "a.py"), "old\nmine\n"));
    expect(auditGeneratedEdits.mock.calls.map(([, batch]) => [batch.edits.map(edit => edit.path), batch.mode])).toEqual([
      [[relative(root, join(worktree, "src", "a.py"))], "log"],
      [["src/a.py"], undefined],
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
    expect(auditGeneratedEdits.mock.calls.flatMap(([, batch]) => batch.edits.map(edit => edit.path))).toEqual([relative(root, join(worktree, "src", "a.py"))]);
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
    expect(auditedPaths(auditGeneratedEdits)).toEqual([relative(root, join(worktree, "top.py"))]);
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
    expect(auditedPaths(auditGeneratedEdits)).toEqual([relative(root, join(worktree, "deep", "new.py"))]);
    expect((await bridge.checkpoint("turn")).failures).toEqual([]);
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
    // Checked out again under these settings: a Windows machine-wide core.autocrlf=true wrote it with CRLF.
    await rm(join(root, "src", "a.py"));
    git(root, "checkout", "--", "src/a.py");
    await awaitClockAfter(join(root, "src", "a.py"));
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
