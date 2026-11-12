// scripts/check/check-owned-builtins.mjs: each built-in that one module owns is reached only through that module, and
// every folder a test makes is removed through test/support/platform/remove-folder.ts.

import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { ownedBuiltinProblems, REMOVE_FOLDER, RULES, TEST_EXEMPT } from "../../../scripts/check/check-owned-builtins.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const rules = (problems) => problems.map(({ path, line, rule }) => `${path}:${line}:${rule}`);
const rule = (id) => RULES.find(row => row.id === id);
const member = (base, path) => `${base}${path.map(part => `.${part}`).join("")}`;

/**
 * Every form that reaches the name `path` (`["constants", "O_NOFOLLOW"]`) of `module`, as the file's text and the line
 * the check reports.
 */
const FORMS = {
  "named import": (module, [head, ...rest]) => (rest.length === 0
    ? { text: `import { ${head} as h } from "${module}";\n`, line: 1 }
    : { text: `import { ${head} as h } from "${module}";\n${member("h", rest)};\n`, line: 2 }),
  "namespace import": (module, path) => ({ text: `import * as m from "${module}";\n${member("m", path)}("x");\n`, line: 2 }),
  "default import": (module, path) => ({ text: `import m from "${module}";\n${member("m", path)}("x");\n`, line: 2 }),
  "import = require": (module, path) => ({ text: `import m = require("${module}");\n${member("m", path)};\n`, line: 2 }),
  "require": (module, path) => ({ text: `const m = require("${module}");\n${member("m", path)};\n`, line: 2 }),
  "destructured require": (module, [head, ...rest]) => (rest.length === 0
    ? { text: `const { ${head}: h } = require("${module}");\n`, line: 1 }
    : { text: `const { ${head}: h } = require("${module}");\n${member("h", rest)};\n`, line: 2 }),
  "import()": (module, path) => ({ text: `const m = await import("${module}");\n${member("m", path)};\n`, line: 2 }),
  "member of import()": (module, path) => ({ text: `${member(`(await import("${module}"))`, path)};\n`, line: 1 }),
  "createRequire": (module, path) => ({ text: `const m = createRequire(import.meta.url)("${module}");\n${member("m", path)};\n`, line: 2 }),
  "getBuiltinModule": (module, path) => ({ text: `${member(`process.getBuiltinModule("${module}")`, path)};\n`, line: 1 }),
  "element access": (module, path) => ({ text: `import * as m from "${module}";\nm${path.map(part => `["${part}"]`).join("")};\n`, line: 2 }),
  "destructured binding": (module, [head, ...rest]) => (rest.length === 0
    ? { text: `import * as m from "${module}";\nconst { ${head}: h } = m;\n`, line: 2 }
    : { text: `import * as m from "${module}";\nconst { ${head}: h } = m;\n${member("h", rest)};\n`, line: 3 }),
  "computed key": (module, [head, ...rest]) => (rest.length === 0
    ? { text: `import * as m from "${module}";\nconst { ["${head}"]: h } = m;\n`, line: 2 }
    : { text: `import * as m from "${module}";\nconst { ["${head}"]: h } = m;\n${member("h", rest)};\n`, line: 3 }),
  "rest element": (module, path) => ({ text: `import * as m from "${module}";\nconst { ...n } = m;\n${member("n", path)};\n`, line: 3 }),
  "alias": (module, path) => ({ text: `import * as m from "${module}";\nconst n = m;\n${member("n", path)};\n`, line: 3 }),
  "cast": (module, path) => ({ text: `import * as m from "${module}";\n${member("(<typeof m>(m satisfies object) as typeof m)!", path)};\n`, line: 2 }),
  "use above its binding": (module, path) => ({ text: `function f() {\n  return ${member("m", path)};\n}\nconst m = require("${module}");\n`, line: 2 }),
  "use above an alias of a later binding": (module, path) => ({
    text: `function f() {\n  const g = () => ${member("n", path)};\n  const n = m;\n  return g();\n}\nconst m = require("${module}");\n`, line: 2,
  }),
  "re-export": (module, [head]) => ({ text: `export { ${head} } from "${module}";\n`, line: 1 }),
  "export *": module => ({ text: `export * from "${module}";\n`, line: 1 }),
  "export * as": module => ({ text: `export * as m from "${module}";\n`, line: 1 }),
  "local export": module => ({ text: `import * as m from "${module}";\nexport { m };\n`, line: 2 }),
  "export default": module => ({ text: `import * as m from "${module}";\nexport default m;\n`, line: 2 }),
  "exported alias": module => ({ text: `import * as m from "${module}";\nexport const n = m;\n`, line: 2 }),
};

/** A file each named rule covers, the owner aside. */
const COVERED = { "file-replace": "src/a.ts", "checkout-paths": "src/engine/catalog/a.ts", "no-follow": "src/a.ts", "test-folder-removal": "test/a.ts" };

describe("check-owned-builtins", () => {
  describe.each(Object.keys(COVERED))("the %s rule", (id) => {
    it.each(Object.keys(FORMS))("finds its names reached through %s, from every module it names", (form) => {
      const row = rule(id);
      const missed = [];
      for (const module of row.modules) {
        for (const path of row.names) {
          const { text, line } = FORMS[form](module, path);
          const found = ownedBuiltinProblems([{ path: COVERED[id], text }]).filter(problem => problem.rule === id).map(problem => problem.line);
          if (found.join() !== String(line)) missed.push(`${module} ${path.join(".")}: lines [${found.join()}], expected ${line}\n${text}`);
        }
      }
      expect(missed).toEqual([]);
    });
  });

  it("reads fs's promises namespace as fs/promises, and reports a realpath's .native once", () => {
    const problems = ownedBuiltinProblems([
      { path: "src/a.ts", text: 'import { mkdirSync, renameSync as move } from "node:fs";\n' },
      { path: "src/b.ts", text: 'import fs from "fs";\nawait fs.promises.rename("a", "b");\n' },
      { path: "src/c.mjs", text: 'import { promises as p } from "node:fs";\nawait p.rename("a", "b");\n' },
      { path: "src/d.ts", text: 'import * as fs from "node:fs";\nconst { promises: { rename } } = fs;\n' },
      { path: "src/runtime/runtime.ts", text: 'import * as fs from "node:fs";\nfs.realpathSync.native("x");\n' },
      { path: "src/api/operations.ts", text: 'import { realpathSync } from "node:fs";\nrealpathSync.native("x");\n' },
    ]);
    expect(rules(problems)).toEqual([
      "src/a.ts:1:file-replace", "src/b.ts:2:file-replace", "src/c.mjs:2:file-replace", "src/d.ts:2:file-replace",
      "src/runtime/runtime.ts:2:checkout-paths", "src/api/operations.ts:1:checkout-paths",
    ]);
    expect(problems[0].message).toContain(rule("file-replace").owner);
    expect(problems[4].message).toContain(rule("checkout-paths").owner);
  });

  it("covers every module of the API, where a caller's folder is resolved and looked for in its checkout", () => {
    const problems = ownedBuiltinProblems([
      { path: "src/api/mcp.ts", text: 'import { realpathSync } from "node:fs";\n' },
      { path: "src/api/mcp-bridge.ts", text: 'import { realpath } from "node:fs/promises";\n' },
    ]);
    expect(rules(problems)).toEqual(["src/api/mcp.ts:1:checkout-paths", "src/api/mcp-bridge.ts:1:checkout-paths"]);
  });

  it("flags every way a module outside the child-process helper loads node:child_process as a value", () => {
    const problems = ownedBuiltinProblems([
      { path: "src/a.ts", text: 'import { spawn } from "node:child_process";\n' },
      { path: "src/b.ts", text: 'import * as cp from "child_process";\n' },
      { path: "src/c.ts", text: 'const cp = await import("node:child_process");\n' },
      { path: "src/d.mjs", text: 'const cp = require("node:child_process");\n' },
      { path: "src/e.ts", text: 'export { execFile } from "node:child_process";\n' },
      { path: "src/f.ts", text: 'import cp = require("node:child_process");\n' },
      { path: "src/g.ts", text: '// one line\nimport { type ChildProcess, spawn } from "node:child_process";\n' },
      { path: "src/h.ts", text: 'import { createRequire } from "node:module";\nconst cp = createRequire(import.meta.url)("node:child_process");\n' },
      { path: "src/i.ts", text: 'const load = createRequire(import.meta.url);\nconst cp = load("child_process");\n' },
      { path: "src/j.ts", text: 'const cp = process.getBuiltinModule("node:child_process");\n' },
      { path: "src/k.cjs", text: 'const cp = module.require("child_process");\n' },
      { path: "src/l.ts", text: "const cp = require(`node:child_process`);\n" },
      { path: "src/m.ts", text: 'import "node:child_process";\nexport * from "child_process";\n' },
      { path: "src/n.ts", text: 'import {} from "node:child_process";\nexport {} from "child_process";\n' },
    ]);
    expect(rules(problems)).toEqual([
      "src/a.ts:1:child-process", "src/b.ts:1:child-process", "src/c.ts:1:child-process", "src/d.mjs:1:child-process",
      "src/e.ts:1:child-process", "src/f.ts:1:child-process", "src/g.ts:2:child-process", "src/h.ts:2:child-process",
      "src/i.ts:2:child-process", "src/j.ts:1:child-process", "src/k.cjs:1:child-process", "src/l.ts:1:child-process",
      "src/m.ts:1:child-process", "src/m.ts:2:child-process", "src/n.ts:1:child-process", "src/n.ts:2:child-process",
    ]);
    expect(problems[0].message).toContain(rule("child-process").owner);
  });

  it("flags an rm given retries outside file-replace.ts, however it reaches rm, and options it cannot read", () => {
    const problems = ownedBuiltinProblems([
      { path: "src/a.ts", text: 'import { rm } from "node:fs/promises";\nawait rm("x", { recursive: true, force: true, maxRetries: 10 });\n' },
      { path: "src/b.ts", text: 'import * as fs from "node:fs";\nconst retries = { maxRetries: 10 };\nfs.rm("x", { recursive: true, ...retries }, () => {});\n' },
      { path: "src/c.ts", text: 'import fs from "fs";\nawait fs.promises.rm("x", options);\n' },
      { path: "src/d.ts", text: 'import { promises } from "node:fs";\nconst remove = promises.rm;\nawait remove("x", { "maxRetries": 3 });\n' },
      { path: "src/e.ts", text: 'const { rm: remove } = await import("node:fs/promises");\nawait remove("x", { maxRetries });\nawait remove("y", { [key]: 1 });\n' },
    ]);
    expect(rules(problems)).toEqual([
      "src/a.ts:2:folder-removal", "src/b.ts:3:folder-removal", "src/c.ts:2:folder-removal", "src/d.ts:3:folder-removal",
      "src/e.ts:2:folder-removal", "src/e.ts:3:folder-removal",
    ]);
    expect(problems[0].message).toContain(rule("folder-removal").owner);
  });

  it("allows any other rm, an rmSync given retries, and the retries in file-replace.ts", () => {
    expect(ownedBuiltinProblems([
      { path: rule("folder-removal").owner, text: 'import { rm } from "node:fs/promises";\nawait rm("x", { recursive: true, maxRetries: 10 });\n' },
      { path: "src/a.ts", text: 'import { rm } from "node:fs/promises";\nawait rm("x");\nawait rm("x", { force: true });\nawait rm("x", { recursive: true, force: true, ["force"]: true });\n' },
      { path: "src/b.ts", text: 'import { rmSync } from "node:fs";\nrmSync("x", { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });\n' },
      { path: "src/c.ts", text: 'import { removeWithRetry } from "./engine/file-replace.js";\nawait removeWithRetry("x");\nconst retries = { maxRetries: 10 };\n' },
      { path: "test/d.ts", text: 'import { rm } from "node:fs/promises";\nawait rm("x", { maxRetries: 10 });\n' },
    ])).toEqual([]);
  });

  it("flags a program named by a literal Windows would look for in the current folder", () => {
    const problems = ownedBuiltinProblems([
      { path: "src/a.ts", text: 'import { spawn } from "./engine/child-process.js";\nspawn("rundll32", ["url.dll"]);\n' },
      { path: "src/b.ts", text: 'execFileSync("powershell.exe", ["-Command", "x"]);\n' },
      { path: "src/c.ts", text: 'spawnSync("/usr/bin/xcode-select", ["-p"]);\nspawn(String.raw`C:\\Windows\\System32\\taskkill.exe`, []);\nspawn(program, []);\n' },
    ]);
    expect(rules(problems)).toEqual(["src/a.ts:2:program-path", "src/b.ts:1:program-path"]);
  });

  it("allows each owner, type-only imports, other names, names kept to the module, and files a rule does not cover", () => {
    expect(ownedBuiltinProblems([
      { path: rule("child-process").owner, text: 'import { spawn } from "node:child_process";\nspawn("git", []);\n' },
      { path: rule("file-replace").owner, text: 'import { rename } from "node:fs/promises";\n' },
      { path: rule("checkout-paths").owner, text: 'import { realpathSync } from "node:fs";\n' },
      { path: rule("no-follow").owner, text: 'import { constants } from "node:fs";\nconst noFollow = constants.O_NOFOLLOW ?? 0;\n' },
      { path: "src/a.ts", text: 'import type { ChildProcess } from "node:child_process";\nimport { type Stats } from "node:fs";\n' },
      { path: "src/b.ts", text: 'type Starter = typeof import("node:child_process").spawn;\nexport type { ChildProcess } from "node:child_process";\n' },
      { path: "src/c.ts", text: 'import { renameWithRetry } from "./engine/file-replace.js";\nawait renameWithRetry("a", "b");\n' },
      { path: "src/d.ts", text: 'import * as fs from "node:fs";\nconst map = { rename: () => 1 };\nmap.rename();\nfs.readFileSync("x");\nconst { readFileSync } = fs;\n' },
      { path: "src/e.ts", text: 'import { promises, constants } from "node:fs";\nconst p = promises;\nexport { readFileSync } from "node:fs";\nexport const mode = constants.O_RDONLY;\n' },
      { path: "src/f.ts", text: 'const CODE_MODULES = new Set(["vm", "child_process"]);\n' },
      { path: "src/engine/git.ts", text: 'import { realpathSync } from "node:fs";\n' },
      { path: "src/engine/catalog/g.ts", text: 'import { checkoutRealpath, physicalPath } from "./checkout-control.js";\n' },
      { path: "test/h.ts", text: 'import { rename } from "node:fs/promises";\nimport { spawn } from "node:child_process";\n' },
      { path: "src/i.md", text: 'import { spawn } from "node:child_process";\n' },
    ])).toEqual([]);
  });

  it("flags a test that removes its temp folder directly", () => {
    const problems = ownedBuiltinProblems([{
      path: "test/a.test.ts",
      text: 'import { mkdtemp, rm } from "node:fs/promises";\nafterEach(async () => {\n  await rm(root, { recursive: true, force: true });\n});\n',
    }]);
    expect(rules(problems)).toEqual(["test/a.test.ts:3:test-folder-removal"]);
    expect(problems[0].message).toContain(REMOVE_FOLDER);
  });

  it("flags every other way a test removes a folder itself, however it reaches the removal", () => {
    expect(rules(ownedBuiltinProblems([
      { path: "test/b.test.ts", text: 'import { rmSync as remove } from "node:fs";\nremove(root, { force: true, recursive: true });\n' },
      { path: "test/c.test.ts", text: 'import * as fs from "node:fs";\nfs.rmSync(root, { recursive: true });\nfs.rmdirSync(root);\n' },
      { path: "test/d.test.mjs", text: 'import fs from "fs";\nawait fs.promises.rm(root, options);\n' },
      { path: "test/e.test.ts", text: 'import { promises as p } from "node:fs";\nawait p.rmdir(root);\nawait p.rm(root, { ...forced });\n' },
      { path: "test/support/f.ts", text: 'import { rimraf } from "rimraf";\n' },
      { path: "test/g.test.mjs", text: 'const { rmSync } = require("node:fs");\nrmSync(root, { recursive: true });\n' },
      { path: "test/h.test.ts", text: '(await import("node:fs")).rmSync(root, { recursive: true });\nconst fs = await import("node:fs/promises");\nconst alias = fs;\nawait alias.rmdir(root);\n' },
    ]))).toEqual([
      "test/b.test.ts:2:test-folder-removal", "test/c.test.ts:2:test-folder-removal", "test/c.test.ts:3:test-folder-removal",
      "test/d.test.mjs:2:test-folder-removal", "test/e.test.ts:2:test-folder-removal", "test/e.test.ts:3:test-folder-removal",
      "test/support/f.ts:1:test-folder-removal", "test/g.test.mjs:2:test-folder-removal", "test/h.test.ts:1:test-folder-removal",
      "test/h.test.ts:4:test-folder-removal",
    ]);
  });

  it("allows a test to remove one file, and leaves the helper, the exempt files and code outside test/ alone", () => {
    expect(ownedBuiltinProblems([
      { path: "test/a.test.ts", text: 'import { rm, rmSync } from "node:fs/promises";\nawait rm(file);\nrmSync(file, { force: true, recursive: false });\n' },
      { path: "test/b.test.ts", text: 'import { removeFolder } from "./support/platform/remove-folder.js";\nawait removeFolder(root);\n' },
      { path: "test/c.test.ts", text: 'import type { rm } from "node:fs/promises";\nconst fake: typeof rm = async () => {};\nawait fake(root, { recursive: true });\n' },
      { path: REMOVE_FOLDER, text: 'import { rm } from "node:fs/promises";\nawait rm(path, { recursive: true, force: true });\n' },
      ...[...TEST_EXEMPT.keys()].map(path => ({ path, text: 'import { rm } from "node:fs/promises";\nawait rm(prefix, { recursive: true, force: true });\n' })),
      { path: "src/a.ts", text: 'import { rm } from "node:fs/promises";\nawait rm(path, { recursive: true });\n' },
    ])).toEqual([]);
  });

  it("passes on this checkout's src/ and test/", () => {
    const run = spawnSync(process.execPath, [join(ROOT, "scripts/check/check-owned-builtins.mjs")], { cwd: ROOT, encoding: "utf8" });
    expect(run.stderr).toBe("");
    expect(run.status).toBe(0);
  });
});
