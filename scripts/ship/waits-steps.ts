// The edit sequence of the step benchmarks (scripts/profiling/edit-bench.sh, and its shapes in
// docs/rule-syntax/site/bench/bench-shapes.sh on the final-shape server-plan branch), for the user waits: one file of
// the repository edited step by step, each step over the ones before it, the same edits on both sides, each measured
// through the product (waits.ts): the edit as the agent's Write through the hooks (its verdict, W1) and a repository
// scan after it (W5).
//
//   cold     a fresh store; no edit
//   warm     nothing changed
//   append   a function appended to the file
//   body     a statement added inside the file's first function
//   comment  a comment line added before the file's first function
//   import   an import added at the top of the file
//   rule     one rule's document changed (its description), the pack added again and approved
//
// Shapes, as the step benchmarks have them:
//   full      the whole corpus, every rule the repository's packs turn on    cold warm append body comment import rule
//   one-file  the edited file alone, the same rules                         cold warm append
//   one-rule  the whole corpus, one rule on (the pack's others turned off)  cold warm append rule
//
// The edits are those of the step benchmarks for Python; a TypeScript or JavaScript file gets the same edits in its
// own syntax. Pure functions; waits.ts applies them.

import { extname } from "node:path";

export const STEPS = ["cold", "warm", "append", "body", "comment", "import", "rule"] as const;
export type Step = (typeof STEPS)[number];
export const FILE_EDITS: readonly Step[] = ["append", "body", "comment", "import"];

export const SHAPES = {
  full: { corpus: "all", rules: "all", steps: ["cold", "warm", "append", "body", "comment", "import", "rule"] },
  "one-file": { corpus: "one", rules: "all", steps: ["cold", "warm", "append"] },
  "one-rule": { corpus: "all", rules: "one", steps: ["cold", "warm", "append", "rule"] },
} as const satisfies Record<string, { corpus: "all" | "one"; rules: "all" | "one"; steps: readonly Step[] }>;
export type Shape = keyof typeof SHAPES;

type Syntax = "python" | "script";

function syntaxOf(path: string): Syntax {
  const extension = extname(path);
  if (extension === ".py") return "python";
  if ([".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"].includes(extension)) return "script";
  throw new Error(`no step edits for ${extension} files (${path})`);
}

/** The first function definition's line: its start offset, its end (after the newline) and its indentation. */
function firstFunction(source: string, syntax: Syntax): { start: number; end: number; indent: string; opensBody: boolean } {
  const pattern = syntax === "python" ? /^( *)(?:async +)?def [^\n]*:\n/m : /^([ \t]*)(?:export +)?(?:default +)?(?:async +)?function\b[^\n]*\n/m;
  const match = pattern.exec(source);
  if (match === null) throw new Error("the file has no function to edit");
  return { start: match.index, end: match.index + match[0].length, indent: match[1]!, opensBody: syntax === "python" || /\{\s*\n$/.test(match[0]) };
}

/** `source` after the file edit `step` (append, body, comment, import), as the step benchmarks make it. */
export function editSource(step: Step, source: string, path: string): string {
  const syntax = syntaxOf(path);
  switch (step) {
    case "append":
      return syntax === "python"
        ? `${source}\n\ndef _t31_added(x):\n    assert x, "edited"\n    return x\n`
        : `${source}\n\nexport function _t31Added(x: any): any {\n  if (!x) throw new Error("edited");\n  return x;\n}\n`;
    case "body": {
      const f = firstFunction(source, syntax);
      if (!f.opensBody) throw new Error("the file's first function does not open its body on its line");
      // The statement takes the indentation of the body's first line, else one level in.
      const indent = /^[ \t]*/.exec(source.slice(f.end))![0] || `${f.indent}    `;
      return `${source.slice(0, f.end)}${indent}${syntax === "python" ? "_t31_local = 0" : "const _t31Local = 0;"}\n${source.slice(f.end)}`;
    }
    case "comment": {
      const f = firstFunction(source, syntax);
      return `${source.slice(0, f.start)}${f.indent}${syntax === "python" ? "# t31: a comment" : "// t31: a comment"}\n${source.slice(f.start)}`;
    }
    case "import":
      return `${syntax === "python" ? "import json as _t31_json" : 'import * as _t31Path from "node:path";'}\n${source}`;
    default:
      throw new Error(`${step} is no file edit`);
  }
}

/** The rule step's edit of a rule document: its description changed, so its revision does, and its findings do not. */
export function editRuleDocument(document: string): string {
  const parsed = JSON.parse(document) as { description?: string };
  if (typeof parsed.description !== "string") throw new Error("the rule document has no description to edit");
  parsed.description = `${parsed.description} (edited)`;
  return `${JSON.stringify(parsed, null, 2)}\n`;
}

/** A pack.json with only `rule` on: the one-rule shape's pack. */
export function oneRulePack(packJson: string, rule: string): string {
  const pack = JSON.parse(packJson) as { rules: { id: string; on?: boolean }[] };
  if (!pack.rules.some((r) => r.id === rule)) throw new Error(`the pack has no rule ${rule}`);
  for (const r of pack.rules) r.on = r.id === rule;
  return `${JSON.stringify(pack, null, 2)}\n`;
}

/** The file the steps edit: the first, in path order, of the corpus's files of the session's language over 4 KiB that the edits apply to. */
export function pickStepFile(files: readonly { path: string; size: number; source: () => string }[]): string {
  for (const file of [...files].sort((a, b) => (a.path < b.path ? -1 : 1))) {
    if (file.size <= 4096) continue;
    try {
      let source = file.source();
      for (const step of FILE_EDITS) source = editSource(step, source, file.path);
      return file.path;
    } catch {
      // Not editable by every step (no function, a body that opens on a later line): the next file.
    }
  }
  throw new Error("no file of the corpus takes every step's edit");
}
