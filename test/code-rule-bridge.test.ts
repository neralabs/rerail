import { lstat, mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import type * as FileSystem from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CodeRuleBridge, generatedEditPaths, isCommandTool, type SourceObservationCache } from "../src/code-rule-bridge.js";
import type { CodeRuleEditBatch, CodeRuleAuditor, CodeRuleResponse } from "../src/code-rule.js";
import { codeRuleDialectForPath } from "../src/code-rule-languages.js";

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
  it("extracts supported generated file paths", () => {
    expect(generatedEditPaths("write", { path: "src/a.py", content: "pass" })).toEqual(["src/a.py"]);
    expect(generatedEditPaths("edit", { patch: "[src/a.ts#A1B2]\nPUT 1.=1:\n+pass\n" })).toEqual(["src/a.ts"]);
    expect(generatedEditPaths("ast_edit", { paths: ["src/a.rs", "docs/policy.md"] })).toEqual(["src/a.rs", "docs/policy.md"]);
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
      ["session-1", {
        revision: 2,
        checkpoints: ["final"],
        edits: [{
          path: "src/a.py", language: "python",
          previousReadError: "Source changed outside a tool with a captured preimage", generatedText: "external edit\n",
        }],
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

  it("labels Blade templates with the blade language, not php, and suffix-only basenames by their extname", async () => {
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
});
