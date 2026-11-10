// scripts/check/check-folder-removal.mjs: every folder a test makes is removed through the helper that names a process
// left running in it and waits out any other holder on Windows.

import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { EXEMPT, folderRemovalProblems, HELPER } from "../../scripts/check/check-folder-removal.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const rules = (problems) => problems.map(({ path, line, rule }) => `${path}:${line}:${rule}`);

describe("check-folder-removal", () => {
  it("flags a test that removes its temp folder directly", () => {
    const problems = folderRemovalProblems([{
      path: "test/a.test.ts",
      text: 'import { mkdtemp, rm } from "node:fs/promises";\nafterEach(async () => {\n  await rm(root, { recursive: true, force: true });\n});\n',
    }]);
    expect(rules(problems)).toEqual(["test/a.test.ts:3:direct"]);
    expect(problems[0].message).toContain(HELPER);
  });

  it("flags every other way a test removes a folder itself", () => {
    expect(rules(folderRemovalProblems([
      { path: "test/b.test.ts", text: 'import { rmSync as remove } from "node:fs";\nremove(root, { force: true, recursive: true });\n' },
      { path: "test/c.test.ts", text: 'import * as fs from "node:fs";\nfs.rmSync(root, { recursive: true });\nfs.rmdirSync(root);\n' },
      { path: "test/d.test.mjs", text: 'import fs from "fs";\nawait fs.promises.rm(root, options);\n' },
      { path: "test/e.test.ts", text: 'import { promises as p } from "node:fs";\nawait p.rmdir(root);\nawait p.rm(root, { ...forced });\n' },
      { path: "test/support/f.ts", text: 'import { rimraf } from "rimraf";\n' },
    ]))).toEqual([
      "test/b.test.ts:2:direct", "test/c.test.ts:2:direct", "test/c.test.ts:3:direct", "test/d.test.mjs:2:direct",
      "test/e.test.ts:2:direct", "test/e.test.ts:3:direct", "test/support/f.ts:1:direct",
    ]);
  });

  it("allows removing one file, the helper itself, the exempt files and code outside test/", () => {
    expect(folderRemovalProblems([
      { path: "test/a.test.ts", text: 'import { rm, rmSync } from "node:fs/promises";\nawait rm(file);\nrmSync(file, { force: true, recursive: false });\n' },
      { path: "test/b.test.ts", text: 'import { removeFolder } from "./support/platform/remove-folder.js";\nawait removeFolder(root);\n' },
      { path: "test/c.test.ts", text: 'import type { rm } from "node:fs/promises";\nconst fake: typeof rm = async () => {};\nawait fake(root, { recursive: true });\n' },
      { path: HELPER, text: 'import { rm } from "node:fs/promises";\nawait rm(path, { recursive: true, force: true });\n' },
      ...[...EXEMPT.keys()].map(path => ({ path, text: 'import { rm } from "node:fs/promises";\nawait rm(prefix, { recursive: true, force: true });\n' })),
      { path: "src/a.ts", text: 'import { rm } from "node:fs/promises";\nawait rm(path, { recursive: true });\n' },
    ])).toEqual([]);
  });

  it("passes on this checkout's test/", () => {
    const run = spawnSync(process.execPath, [join(ROOT, "scripts/check/check-folder-removal.mjs")], { cwd: ROOT, encoding: "utf8" });
    expect(run.stderr).toBe("");
    expect(run.status).toBe(0);
  });
});
