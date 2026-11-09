import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { describe, expect, it } from "vitest";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/**
 * The hooks a coding agent starts through a parent it may kill on a timeout: cmd.exe (Cursor), PowerShell (Codex) or the
 * hook launcher (Claude Code). On Windows killing that parent leaves node running, holding its session's lock.
 */
const ENTRIES = ["src/hosts/claude/claude-hook.ts", "src/hosts/codex/codex-hook.ts", "src/hosts/cursor/cursor-hook.ts"];

/** The calls made, in order, inside the module's `if (launchedDirectly(import.meta.url)) { ... }` block. */
function callsWhenLaunched(path: string): string[] {
  const source = ts.createSourceFile(path, readFileSync(join(ROOT, path), "utf8"), ts.ScriptTarget.Latest, true);
  const launched = source.statements.find((statement): statement is ts.IfStatement =>
    ts.isIfStatement(statement) && ts.isCallExpression(statement.expression)
    && ts.isIdentifier(statement.expression.expression) && statement.expression.expression.text === "launchedDirectly");
  if (launched === undefined) return [];
  const calls: string[] = [];
  const visit = (node: ts.Node): void => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression)) calls.push(node.expression.text);
    ts.forEachChild(node, visit);
  };
  visit(launched.thenStatement);
  return calls;
}

describe("the hook entries a coding agent starts", () => {
  it("each watches its parent before it serves the hook, so it ends when the agent gives up on it", () => {
    for (const path of ENTRIES) {
      const calls = callsWhenLaunched(path);
      expect(calls, path).toContain("exitWhenParentGone");
      expect(calls.indexOf("exitWhenParentGone"), path).toBeLessThan(calls.indexOf("serveHook"));
    }
  });
});
