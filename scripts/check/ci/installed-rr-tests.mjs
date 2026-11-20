#!/usr/bin/env node
// The packaged Rerail runs a `.rerail` rule file's tests (package.yml's smoke of the installed release package): the
// tests read the engine's typing programs and the `.rerail` rule library, which the package ships beside the engine
// (package.json `files`), so a file left out of the package fails here, at packaging, not at a user's.
//
//   node scripts/check/ci/installed-rr-tests.mjs <installed package directory>
//
// The rule below is written to a new directory outside any checkout, and the installed CLI's `rerail rules test` runs
// there with no `RERAIL_SOURCE_ROOT` or `RERAIL_RR_LIB`: the engine finds only what the package holds. Exits 1 unless
// every test the file carries passes.
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const RULE_FILE = "no-os-system.rerail";

/**
 * Name resolution through imports (`sym_of`, python/names.rerail) needs the library well past core.rerail, and phase 1
 * (the typing program sources.rerail) reads the imports.
 */
const RULE = `.policy "package-smoke-no-os-system" revision 1
.description "os.system runs its argument through a shell."
.language python
.mode steer
.scope include "**/*.py"

#include "python/calls.rerail"

.violation OsSystem(at: node) code "OS001" "os.system runs its argument through a shell; call subprocess.run with a list instead."
OsSystem(c) :- kind(c, "call"), field(c, "function", f), sym_of(f, "os.system").

.test imported_and_aliased_calls_are_reported exactly
    .fixture "app/run.py" \`\`\`
        import os as o
        from os import system
        o.system("ls")
        system("ls")
        print("ls")
        \`\`\`
    .expect OS001 at "app/run.py":3
    .expect OS001 at "app/run.py":4
.end

.test a_local_function_of_that_name_is_not_reported exactly
    .fixture "app/local.py" \`\`\`
        def system(command):
            return command
        system("ls")
        \`\`\`
.end
`;

/** The environment of the run: this one, less what points the engine at files the package does not hold. */
function packageOnlyEnvironment(home) {
  const environment = { ...process.env, RERAIL_HOME: home };
  delete environment.RERAIL_SOURCE_ROOT;
  delete environment.RERAIL_RR_LIB;
  return environment;
}

/** The CLI's `--json` report, or null when it printed none. */
function report(stdout) {
  try {
    return JSON.parse(stdout);
  } catch {
    return null;
  }
}

/** Why the run (its exit status and its report) is not a pass of every test, or null. */
function failure(status, results) {
  if (!Array.isArray(results) || results.length === 0) return `rerail rules test ran no test (exit ${status})`;
  const failed = results.filter((result) => result.status !== "pass");
  if (failed.length > 0) return failed.map((result) => `${result.test}: ${result.failures.join("; ")}`).join("\n");
  return status === 0 ? null : `rerail rules test exited ${status} though every test passed`;
}

function main(installed) {
  const work = mkdtempSync(join(tmpdir(), "rerail-installed-rr-"));
  try {
    writeFileSync(join(work, RULE_FILE), RULE);
    const run = spawnSync(process.execPath, [join(installed, "dist", "cli", "cli.js"), "rules", "test", join(work, RULE_FILE), "--json"], {
      cwd: work,
      env: packageOnlyEnvironment(join(work, "home")),
      encoding: "utf8",
    });
    const results = report(run.stdout);
    const why = failure(run.status, results);
    if (why !== null) {
      console.error(`The installed package could not run ${RULE_FILE}'s tests:\n${why}\n${run.stdout}${run.stderr}`);
      return 1;
    }
    console.log(`installed package ran ${RULE_FILE}'s tests: ${results.map((result) => `${result.status} ${result.test}`).join(", ")}`);
    return 0;
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const installed = process.argv[2];
  if (installed === undefined) {
    console.error("usage: node scripts/check/ci/installed-rr-tests.mjs <installed package directory>");
    process.exit(2);
  }
  process.exit(main(resolve(installed)));
}
