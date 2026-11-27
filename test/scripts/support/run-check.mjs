// A check of scripts/check/ run as `npm run check` starts it, as a process of its own, over a Git repository of a few
// files a test writes. A check reads the repository its own file sits in, two folders up, so the test runs a copy of it
// in a new repository: its cost is a Node start, TypeScript's load and those few files, however large this checkout grows.

import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, lstatSync, mkdirSync, mkdtempSync, readdirSync, realpathSync, unlinkSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { linkFolder } from "../../support/platform/links.ts";
import { removeFolderSync } from "../../support/platform/remove-folder.ts";

const CHECKS = fileURLToPath(new URL("../../../scripts/check/", import.meta.url));
/** The node_modules folder this checkout loads TypeScript from, which the checks import. */
const MODULES = dirname(dirname(createRequire(import.meta.url).resolve("typescript/package.json")));

/** Git with none of an enclosing hook's index or directory, so it reads the new repository. */
const ENV = { ...process.env };
for (const key of ["GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE"]) delete ENV[key];

/**
 * Runs `node scripts/check/<script>` in a new Git repository that holds `files` (repository path to text, untracked as a
 * new file is), a copy of each module of scripts/check/ and a link to this checkout's node_modules, then removes the
 * repository. Returns the process's exit status, standard output and standard error.
 */
export function runCheck(script, files) {
  // Node runs a script from its real path and the check runs only when that is the path it was given: on macOS the
  // temp folder is under /var, a link to /private/var.
  const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-check-")));
  const modules = join(root, "node_modules");
  try {
    mkdirSync(join(root, "scripts", "check"), { recursive: true });
    for (const name of readdirSync(CHECKS).filter(name => name.endsWith(".mjs"))) {
      copyFileSync(join(CHECKS, name), join(root, "scripts", "check", name));
    }
    for (const [path, text] of Object.entries(files)) {
      mkdirSync(dirname(join(root, path)), { recursive: true });
      writeFileSync(join(root, path), text);
    }
    linkFolder(MODULES, modules);
    execFileSync("git", ["init", "-q"], { cwd: root, env: ENV, stdio: "pipe" });
    const run = spawnSync(process.execPath, [join(root, "scripts", "check", script)], { cwd: root, env: ENV, encoding: "utf8" });
    return { status: run.status, stdout: run.stdout, stderr: run.stderr };
  } finally {
    // The link goes first, so the removal cannot reach this checkout's node_modules through it.
    if (lstatSync(modules, { throwIfNoEntry: false })?.isSymbolicLink()) unlinkSync(modules);
    removeFolderSync(root);
  }
}

/** The `path:line` of each problem a check printed to standard error (`path:line: [rule] message`). */
export const printedPlaces = stderr => stderr.split("\n").filter(Boolean).map(line => line.slice(0, line.indexOf(": ")));
