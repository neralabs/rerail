// #24 Every test root and harness runs in CI.
//
// What CI runs: every `run:` script in .github/workflows/ci.yml, split into simple commands, expanded
// through npm scripts transitively (`npm run X`, `npm test`, `npm --prefix <dir> …` for the package in
// <dir>, `cd <dir>` within a script). From those commands the gate collects what executes:
//   - `vitest [run] [--dir D] [--config C] [filters…]` in package P runs P's `*.test.*`/`*.spec.*` files under
//     D (else under C's directory, else all of P) whose path contains a filter (when given), minus the
//     string globs in P's `vitest.config.*` `exclude` array;
//   - `node --test files…` runs those files; `pytest paths…` runs the test files under those paths;
//   - `node [--import X] file`, `tsx file`, `python[3] file`, `bash|sh file` run `file` as an entry point.
// Predicate:
//   - every tracked test file (`*.test.*`/`*.spec.*` scripts, pytest `test_*.py`/`*_test.py`, PHPUnit
//     `*Test.php`) is run by CI;
//   - every entry point (harness CLI) that an npm script named `test`, `test:*`, `conformance` or
//     `conformance:*` runs, in any package, is also run by CI. Test files those scripts run are covered by
//     the first part, so watch-mode and single-file variants of a CI-run suite pass.
// Exemption `data-roots`: test files under `fixtures/` and `packs/<id>/upstream/` are recorded data.
// Rust tests run through `cargo test`, which is outside this metric.

import { posix } from "node:path";
import { basename, dirname, finding, globToRegExp, isDataPath, isTestFile, readJson, shellCommands, sortFindings, workflowRunScripts } from "../lib.mjs";

export const id = 24;
export const title = "Every test root and harness runs in CI";
export const target = "100%";

export const WORKFLOW = ".github/workflows/ci.yml";

function join(dir, path) {
  return posix.normalize(dir === "" ? path : `${dir}/${path}`).replace(/^\.\//, "").replace(/\/$/, "");
}

/** Records of what a command runs: { kind: "vitest", pkg, dir, filters, config } | { kind: "file", path } | { kind: "tests", path }. */
export function analyse(repo) {
  const scriptsOf = (pkg) => readJson(repo, join(pkg, "package.json"))?.scripts ?? {};
  const memo = new Map();

  function runScript(pkg, name, extra, stack) {
    const key = `${pkg}\0${name}`;
    if (stack.has(key)) return [];
    const command = scriptsOf(pkg)[name];
    if (typeof command !== "string") return [];
    if (extra.length === 0 && memo.has(key)) return memo.get(key);
    const records = runShell(pkg, `${command}${extra.length > 0 ? ` ${extra.join(" ")}` : ""}`, new Set([...stack, key]));
    if (extra.length === 0) memo.set(key, records);
    return records;
  }

  function runShell(pkg, script, stack) {
    const records = [];
    let cwd = pkg;
    for (const tokens of shellCommands(script)) {
      if (tokens[0] === "cd" && tokens[1]) {
        cwd = join(cwd, tokens[1]);
        continue;
      }
      records.push(...runCommand(cwd, tokens, stack));
    }
    return records;
  }

  function runCommand(cwd, tokens, stack) {
    let [program, ...args] = tokens;
    if (program === "npx" || program === "exec") [program, ...args] = args;
    if (program === undefined) return [];
    if (program === "npm") return runNpm(cwd, args, stack);
    if (program === "vitest") return [vitest(cwd, args)];
    if (program === "node" || program === "tsx") return runNode(cwd, args);
    if (program === "pytest" || ((program === "python" || program === "python3") && args[0] === "-m" && args[1] === "pytest")) {
      const paths = program === "pytest" ? args : args.slice(2);
      return paths.filter((arg) => !arg.startsWith("-")).map((arg) => ({ kind: "tests", path: join(cwd, arg) }));
    }
    if (program === "python" || program === "python3" || program === "bash" || program === "sh") {
      const file = args.find((arg) => !arg.startsWith("-"));
      return file ? [{ kind: "file", path: join(cwd, file) }] : [];
    }
    if (program.startsWith("./") || program.includes("/")) return [{ kind: "file", path: join(cwd, program) }];
    return [];
  }

  function runNpm(cwd, args, stack) {
    let pkg = cwd;
    const rest = [];
    for (let i = 0; i < args.length; i += 1) {
      if ((args[i] === "--prefix" || args[i] === "-C") && args[i + 1]) pkg = join(cwd, args[(i += 1)]);
      else if (args[i].startsWith("--prefix=")) pkg = join(cwd, args[i].slice("--prefix=".length));
      else rest.push(args[i]);
    }
    const [verb, ...more] = rest;
    const dash = more.indexOf("--");
    const extra = dash >= 0 ? more.slice(dash + 1) : [];
    if (verb === "test" || verb === "t") return runScript(pkg, "test", extra, stack);
    if (verb === "run" || verb === "run-script") return more[0] ? runScript(pkg, more[0], extra, stack) : [];
    return [];
  }

  function vitest(cwd, args) {
    const record = { kind: "vitest", pkg: cwd, dir: null, config: null, filters: [] };
    const valued = new Set(["--dir", "--config", "-c", "--root", "-r", "--project", "--shard", "--reporter", "--pool"]);
    for (let i = 0; i < args.length; i += 1) {
      const arg = args[i];
      const [flag, inline] = arg.includes("=") ? [arg.slice(0, arg.indexOf("=")), arg.slice(arg.indexOf("=") + 1)] : [arg, null];
      if (valued.has(flag)) {
        const value = inline ?? args[(i += 1)];
        if (flag === "--dir" || flag === "--root" || flag === "-r") record.dir = join(cwd, value);
        if (flag === "--config" || flag === "-c") record.config = join(cwd, value);
      } else if (arg === "run" || arg === "watch" || arg.startsWith("-")) {
        continue;
      } else {
        record.filters.push(arg);
      }
    }
    return record;
  }

  function runNode(cwd, args) {
    const records = [];
    let test = false;
    for (let i = 0; i < args.length; i += 1) {
      const arg = args[i];
      if (arg === "--test") test = true;
      else if (arg === "--import" || arg === "--require" || arg === "-r" || arg === "--loader") i += 1;
      else if (arg === "-e" || arg === "--eval" || arg === "-p" || arg === "--print") return [];
      else if (arg.startsWith("-")) continue;
      else if (test) records.push({ kind: "tests", path: join(cwd, arg) });
      else return [{ kind: "file", path: join(cwd, arg) }];
    }
    return records;
  }

  const ci = repo.has(WORKFLOW) ? workflowRunScripts(repo.text(WORKFLOW)) : [];
  const reached = ci.flatMap(({ script }) => runShell("", script, new Set()));
  const packages = repo.files.filter((path) => basename(path) === "package.json" && !isDataPath(path) && !path.startsWith("packs/"));
  const harnessScripts = [];
  for (const manifest of packages) {
    const pkg = dirname(manifest);
    for (const name of Object.keys(scriptsOf(pkg)).sort()) {
      if (/^(test|conformance)(:|$)/.test(name)) harnessScripts.push({ pkg, name, records: runScript(pkg, name, [], new Set()) });
    }
  }
  return { reached, harnessScripts };
}

function vitestExcludes(repo, pkg) {
  for (const name of ["vitest.config.ts", "vitest.config.mts", "vitest.config.js", "vitest.config.mjs"]) {
    const text = repo.text(join(pkg, name));
    if (text === null) continue;
    const block = /exclude\s*:\s*\[([\s\S]*?)\]/.exec(text);
    if (!block) return [];
    return [...block[1].matchAll(/["']([^"']+)["']/g)].map((match) => globToRegExp(match[1]));
  }
  return [];
}

/** The tracked test files a run record executes. */
export function filesRun(repo, record, testFiles) {
  if (record.kind === "file") return testFiles.includes(record.path) ? [record.path] : [];
  if (record.kind === "tests") return testFiles.filter((path) => path === record.path || path.startsWith(`${record.path}/`));
  const root = record.dir ?? (record.config ? dirname(record.config) : record.pkg);
  const excludes = vitestExcludes(repo, record.pkg);
  return testFiles.filter((path) => {
    if (!/\.(test|spec)\.[cm]?[jt]sx?$/.test(path)) return false;
    if (root !== "" && !path.startsWith(`${root}/`)) return false;
    const relative = record.pkg === "" ? path : path.slice(record.pkg.length + 1);
    if (record.pkg !== "" && !path.startsWith(`${record.pkg}/`)) return false;
    if (excludes.some((exclude) => exclude.test(relative))) return false;
    return record.filters.length === 0 || record.filters.some((filter) => relative.includes(filter));
  });
}

export function check(repo) {
  const testFiles = repo.files.filter((path) => isTestFile(path) && !isDataPath(path));
  const { reached, harnessScripts } = analyse(repo);
  const ranTests = new Set(reached.flatMap((record) => filesRun(repo, record, testFiles)));
  const ranEntries = new Set(reached.filter((record) => record.kind === "file").map((record) => record.path));
  const findings = [];
  for (const path of testFiles) {
    if (!ranTests.has(path)) findings.push(finding(path, "test file no CI step runs"));
  }
  let harnessOk = 0;
  for (const { pkg, name, records } of harnessScripts) {
    const missing = new Set(records.filter((record) => record.kind === "file" && !ranEntries.has(record.path)).map((record) => record.path));
    if (missing.size === 0) harnessOk += 1;
    for (const path of [...missing].sort()) {
      findings.push(finding(join(pkg, "package.json"), `script "${name}" runs ${path}, which no CI step runs`));
    }
  }
  const total = testFiles.length + harnessScripts.length;
  const passing = testFiles.filter((path) => ranTests.has(path)).length + harnessOk;
  const percent = total === 0 ? 100 : Math.floor((passing * 100) / total);
  return { actual: `${percent}% (${passing}/${total})`, findings: sortFindings(findings) };
}
