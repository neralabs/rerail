#!/usr/bin/env node
// The packaged Rerail runs a `.rerail` rule file's tests (package.yml's smoke of the installed release package): the
// tests read the engine's typing programs and the `.rerail` rule library, which the package ships beside the engine
// (package.json `files`), so a file left out of the package fails here, at packaging, not at a user's.
//
//   node scripts/check/ci/installed-rr-tests.mjs <installed package directory>
//
// The rule (fixtures/host/package-smoke/) is copied to a new directory outside any checkout, and the installed CLI's
// `rerail rules test` runs there with no `RERAIL_SOURCE_ROOT` or `RERAIL_RR_LIB`: the engine finds only what the
// package holds. Exits 1 unless every test the file carries passes.
import { spawnSync } from "node:child_process";
import { cpSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const RULE = "no-os-system.rerail";
const FIXTURE = fileURLToPath(new URL("../../../fixtures/host/package-smoke/", import.meta.url));

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
    cpSync(FIXTURE, join(work, "rule"), { recursive: true });
    const run = spawnSync(process.execPath, [join(installed, "dist", "cli", "cli.js"), "rules", "test", join(work, "rule", RULE), "--json"], {
      cwd: work,
      env: packageOnlyEnvironment(join(work, "home")),
      encoding: "utf8",
    });
    const results = report(run.stdout);
    const why = failure(run.status, results);
    if (why !== null) {
      console.error(`The installed package could not run ${RULE}'s tests:\n${why}\n${run.stdout}${run.stderr}`);
      return 1;
    }
    console.log(`installed package ran ${RULE}'s tests: ${results.map((result) => `${result.status} ${result.test}`).join(", ")}`);
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
