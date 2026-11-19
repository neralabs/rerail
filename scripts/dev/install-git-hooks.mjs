#!/usr/bin/env node
// `npm install` in a checkout (`prepare`) installs Git's pre-commit hook, which runs the checked-out branch's
// scripts/check/pre-commit.sh: ci.yml's lint and type-check job, before each commit. It writes only `pre-commit` in the
// repository's hooks folder, so Git LFS's hooks there stay, and leaves a pre-commit hook it did not write alone. In CI,
// or outside a Git checkout (a packed or installed package), it does nothing.
import { execFileSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const MARK = "# Installed by scripts/dev/install-git-hooks.mjs";
const HOOK = `#!/bin/sh
${MARK}: runs the checked-out branch's own pre-commit check, when it has one.
check="$(git rev-parse --show-toplevel)/scripts/check/pre-commit.sh"
[ -f "$check" ] || exit 0
exec sh "$check"
`;

/** The hooks folder Git runs, shared by every worktree of the repository; undefined outside a Git checkout. */
function hooksFolder() {
  try {
    return resolve(execFileSync("git", ["rev-parse", "--git-path", "hooks"], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim());
  } catch {
    return undefined;
  }
}

const hooks = process.env.CI ? undefined : hooksFolder();
if (hooks !== undefined) {
  const path = join(hooks, "pre-commit");
  if (existsSync(path) && !readFileSync(path, "utf8").includes(MARK)) {
    console.log(`install-git-hooks: ${path} is not Rerail's, so it is left as it is; scripts/check/pre-commit.sh runs the same checks`);
  } else {
    mkdirSync(hooks, { recursive: true });
    writeFileSync(path, HOOK);
    chmodSync(path, 0o755);
  }
}
