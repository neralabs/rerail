import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { Readable } from "node:stream";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";
import { describe, expect, it, vi } from "vitest";
import { serveHookEntry, type HookStreams } from "../../src/harness/hook-adapter.js";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/**
 * The hooks a coding agent starts through a parent it may kill on a timeout: cmd.exe (Cursor), PowerShell (Codex) or the
 * hook launcher (Claude Code). On Windows killing that parent leaves node running, holding its session's lock.
 */
const ENTRIES = ["src/hosts/claude/claude-hook.ts", "src/hosts/codex/codex-hook.ts", "src/hosts/cursor/cursor-hook.ts"];

/** The module node was started with in this test process, as a hook entry's `import.meta.url` is in a hook process. */
const LAUNCHED = pathToFileURL(process.argv[1]!).href;

function capture(stdin: string): { streams: HookStreams; written: { stdout: string } } {
  const written = { stdout: "" };
  return {
    written,
    streams: {
      stdin: Readable.from([stdin]),
      stdout: { write: (chunk, callback) => { written.stdout += chunk; callback(); } },
      stderr: { write: () => true },
    },
  };
}

describe("serveHookEntry", () => {
  it("ends a Windows hook whose parent is gone while the hook still runs, and serves the hook", async () => {
    const exit = vi.fn();
    const alive = vi.fn(() => false);
    const { streams, written } = capture("{}");
    await serveHookEntry(LAUNCHED, async () => {
      // The agent killed the PowerShell or cmd.exe that started the hook, which leaves node running on Windows.
      await vi.waitFor(() => expect(exit).toHaveBeenCalledWith(0));
      return { answered: true };
    }, { watch: { platform: "win32", parentPid: 4242, alive, exit, intervalMs: 5 }, streams });
    expect(alive).toHaveBeenCalledWith(4242);
    expect(written.stdout).toBe('{"answered":true}\n');
  });

  it("does nothing in a module that was imported, not started", async () => {
    const alive = vi.fn(() => false);
    const run = vi.fn(async () => ({}));
    const { streams, written } = capture("{}");
    await serveHookEntry(import.meta.url, run, { watch: { platform: "win32", parentPid: 4242, alive, exit: vi.fn(), intervalMs: 5 }, streams });
    await new Promise(done => setTimeout(done, 20));
    expect(run).not.toHaveBeenCalled();
    expect(alive).not.toHaveBeenCalled();
    expect(written.stdout).toBe("");
  });
});

/** Whether `statement` is `await serveHookEntry(import.meta.url, …)`. */
function isEntryCall(statement: ts.Statement): boolean {
  if (!ts.isExpressionStatement(statement) || !ts.isAwaitExpression(statement.expression)) return false;
  const call = statement.expression.expression;
  return ts.isCallExpression(call) && ts.isIdentifier(call.expression) && call.expression.text === "serveHookEntry"
    && call.arguments[0]?.getText() === "import.meta.url";
}

describe("the hook entries a coding agent starts", () => {
  it("each serves its hook through serveHookEntry, which watches the parent first", () => {
    for (const path of ENTRIES) {
      const source = ts.createSourceFile(path, readFileSync(join(ROOT, path), "utf8"), ts.ScriptTarget.Latest, true);
      expect(source.statements.filter(isEntryCall), path).toHaveLength(1);
    }
  });
});
