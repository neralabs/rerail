// scripts/check/check-child-process.mjs: every child process in src/ starts through the helper that hides its window.

import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { childProcessProblems, HELPER } from "../../scripts/check/check-child-process.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const helper = { path: HELPER, text: readFileSync(join(ROOT, HELPER), "utf8") };
const rules = (problems) => problems.map(({ path, line, rule }) => `${path}:${line}:${rule}`);

describe("check-child-process", () => {
  it("flags every way a module outside the helper loads node:child_process as a value", () => {
    const problems = childProcessProblems([
      helper,
      { path: "src/a.ts", text: 'import { spawn } from "node:child_process";\n' },
      { path: "src/b.ts", text: 'import * as cp from "child_process";\n' },
      { path: "src/c.ts", text: 'const cp = await import("node:child_process");\n' },
      { path: "src/d.mjs", text: 'const cp = require("node:child_process");\n' },
      { path: "src/e.ts", text: 'export { execFile } from "node:child_process";\n' },
      { path: "src/f.ts", text: 'import cp = require("node:child_process");\n' },
      { path: "src/g.ts", text: '// one line\nimport { type ChildProcess, spawn } from "node:child_process";\n' },
    ]);
    expect(rules(problems)).toEqual([
      "src/a.ts:1:bypass", "src/b.ts:1:bypass", "src/c.ts:1:bypass", "src/d.mjs:1:bypass", "src/e.ts:1:bypass", "src/f.ts:1:bypass",
      "src/g.ts:2:bypass",
    ]);
    expect(problems[0].message).toContain(HELPER);
  });

  it("flags the module's name used as a value any other way: a createRequire, getBuiltinModule or module.require load", () => {
    const problems = childProcessProblems([
      helper,
      { path: "src/a.ts", text: 'import { createRequire } from "node:module";\nconst cp = createRequire(import.meta.url)("node:child_process");\n' },
      { path: "src/b.ts", text: 'const load = createRequire(import.meta.url);\nconst cp = load("child_process");\n' },
      { path: "src/c.ts", text: 'const cp = process.getBuiltinModule("node:child_process");\n' },
      { path: "src/d.cjs", text: 'const cp = module.require("child_process");\n' },
      { path: "src/e.ts", text: 'const cp = require(`node:child_process`);\n' },
      // A list of module names loads nothing.
      { path: "src/f.ts", text: 'const CODE_MODULES = new Set(["vm", "child_process"]);\n' },
    ]);
    expect(rules(problems)).toEqual(["src/a.ts:2:bypass", "src/b.ts:2:bypass", "src/c.ts:1:bypass", "src/d.cjs:1:bypass", "src/e.ts:1:bypass"]);
  });

  it("allows type-only imports, the helper's own, and code outside src/", () => {
    expect(childProcessProblems([
      helper,
      { path: "src/a.ts", text: 'import type { ChildProcess } from "node:child_process";\n' },
      { path: "src/b.ts", text: 'import { type ChildProcess } from "node:child_process";\nimport { spawn } from "./engine/child-process.js";\n' },
      { path: "src/e.ts", text: 'type Starter = typeof import("node:child_process").spawn;\nexport type { ChildProcess } from "node:child_process";\n' },
      { path: "test/c.ts", text: 'import { spawn } from "node:child_process";\n' },
      { path: "src/d.md", text: 'import { spawn } from "node:child_process";\n' },
    ])).toEqual([]);
  });

  it("requires the helper, and requires its code, not its comments, to set windowsHide: true", () => {
    expect(rules(childProcessProblems([]))).toEqual([`${HELPER}:1:hidden`]);
    expect(rules(childProcessProblems([{ path: HELPER, text: "export const spawn = childProcess.spawn;\n" }])))
      .toEqual([`${HELPER}:1:hidden`, `${HELPER}:1:path`]);
    const shown = helper.text.replace("const HIDDEN = { windowsHide: true } as const;", "const HIDDEN = { windowsHide: false } as const;");
    expect(shown).not.toBe(helper.text);
    // The helper's doc comment still says `windowsHide: true`.
    expect(shown).toContain("windowsHide: true");
    expect(rules(childProcessProblems([{ path: HELPER, text: shown }]))).toEqual([`${HELPER}:1:hidden`]);
  });

  it("flags a program named by a literal Windows would look for in the current folder", () => {
    const problems = childProcessProblems([
      helper,
      { path: "src/a.ts", text: 'import { spawn } from "./engine/child-process.js";\nspawn("rundll32", ["url.dll"]);\n' },
      { path: "src/b.ts", text: 'execFileSync("powershell.exe", ["-Command", "x"]);\n' },
      { path: "src/c.ts", text: 'spawnSync("/usr/bin/xcode-select", ["-p"]);\nspawn(String.raw`C:\\Windows\\System32\\taskkill.exe`, []);\nspawn(program, []);\n' },
    ]);
    expect(rules(problems)).toEqual(["src/a.ts:2:path", "src/b.ts:1:path"]);
  });

  it("passes on this checkout's src/", () => {
    const run = spawnSync(process.execPath, [join(ROOT, "scripts/check/check-child-process.mjs")], { cwd: ROOT, encoding: "utf8" });
    expect(run.stderr).toBe("");
    expect(run.status).toBe(0);
  });
});
