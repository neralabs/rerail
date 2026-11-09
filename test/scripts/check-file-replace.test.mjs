// scripts/check/check-file-replace.mjs: every rename in src/ goes through the helper that retries one Windows refuses.

import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { fileReplaceProblems, HELPER } from "../../scripts/check/check-file-replace.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const rules = (problems) => problems.map(({ path, line, rule }) => `${path}:${line}:${rule}`);

describe("check-file-replace", () => {
  it("flags every way a module outside the helper renames a file", () => {
    const problems = fileReplaceProblems([
      { path: "src/a.ts", text: 'import { rename } from "node:fs/promises";\n' },
      { path: "src/b.ts", text: 'import { mkdirSync, renameSync as move } from "node:fs";\n' },
      { path: "src/c.ts", text: 'import * as fs from "node:fs";\nfs.renameSync("a", "b");\n' },
      { path: "src/d.ts", text: 'import fs from "fs";\nawait fs.promises.rename("a", "b");\n' },
      { path: "src/e.mjs", text: 'import { promises as p } from "node:fs";\nawait p.rename("a", "b");\n' },
    ]);
    expect(rules(problems)).toEqual(["src/a.ts:1:bypass", "src/b.ts:1:bypass", "src/c.ts:2:bypass", "src/d.ts:2:bypass", "src/e.mjs:2:bypass"]);
    expect(problems[0].message).toContain(HELPER);
  });

  it("allows the helper itself, renames of anything but fs, and code outside src/", () => {
    expect(fileReplaceProblems([
      { path: HELPER, text: 'import { rename } from "node:fs/promises";\n' },
      { path: "src/a.ts", text: 'import { renameWithRetry } from "./engine/file-replace.js";\nawait renameWithRetry("a", "b");\n' },
      { path: "src/b.ts", text: 'import * as fs from "node:fs";\nconst map = { rename: () => 1 };\nmap.rename();\nfs.readFileSync("x");\n' },
      { path: "test/c.ts", text: 'import { rename } from "node:fs/promises";\n' },
    ])).toEqual([]);
  });

  it("passes on this checkout's src/", () => {
    const run = spawnSync(process.execPath, [join(ROOT, "scripts/check/check-file-replace.mjs")], { cwd: ROOT, encoding: "utf8" });
    expect(run.stderr).toBe("");
    expect(run.status).toBe(0);
  });
});
