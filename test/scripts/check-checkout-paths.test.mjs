// scripts/check/check-checkout-paths.mjs: modules that key records by a checkout path resolve it through one helper.

import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { checkoutPathProblems, HELPER } from "../../scripts/check/check-checkout-paths.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const rules = (problems) => problems.map(({ path, line, rule }) => `${path}:${line}:${rule}`);

describe("check-checkout-paths", () => {
  it("flags a keyed module that resolves a path with node:fs's realpath", () => {
    const problems = checkoutPathProblems([
      { path: "src/engine/catalog/a.ts", text: 'import { realpathSync } from "node:fs";\n' },
      { path: "src/engine/audit/b.ts", text: 'import { realpath } from "node:fs/promises";\n' },
      { path: "src/runtime/runtime.ts", text: 'import * as fs from "node:fs";\nfs.realpathSync.native("x");\n' },
      { path: "src/api/operations.ts", text: 'import fs from "fs";\nawait fs.promises.realpath("x");\n' },
    ]);
    expect(rules(problems)).toEqual([
      "src/engine/catalog/a.ts:1:bypass", "src/engine/audit/b.ts:1:bypass", "src/runtime/runtime.ts:2:bypass", "src/api/operations.ts:2:bypass",
    ]);
    expect(problems[0].message).toContain(HELPER);
  });

  it("allows the helper, modules that key nothing by a checkout, and the helper's own functions", () => {
    expect(checkoutPathProblems([
      { path: HELPER, text: 'import { realpathSync } from "node:fs";\n' },
      { path: "src/engine/git.ts", text: 'import { realpathSync } from "node:fs";\n' },
      { path: "src/engine/catalog/c.ts", text: 'import { checkoutRealpath, physicalPath } from "./checkout-control.js";\n' },
    ])).toEqual([]);
  });

  it("passes on this checkout's src/", () => {
    const run = spawnSync(process.execPath, [join(ROOT, "scripts/check/check-checkout-paths.mjs")], { cwd: ROOT, encoding: "utf8" });
    expect(run.stderr).toBe("");
    expect(run.status).toBe(0);
  });
});
