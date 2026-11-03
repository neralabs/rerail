#!/usr/bin/env node
// Writes, removes, or checks the npm-shrinkwrap.json core's tarball ships (shrinkwrap.mjs). npm runs `write` as
// prepack and `remove` as postpack, so the tarball carries it while the checkout never does. A pack that fails after
// prepack skips postpack and leaves the file behind, which npm would then prefer to package-lock.json:
// `node scripts/release/shrinkwrap-pack.mjs remove` (idempotent) cleans it up. release.yml runs `check` on the tarball
// before publishing, and fails the release when the tarball has no shrinkwrap or one package-lock.json does not give.
//
//   node scripts/release/shrinkwrap-pack.mjs write [package-root]
//   node scripts/release/shrinkwrap-pack.mjs remove [package-root]
//   node scripts/release/shrinkwrap-pack.mjs check <tarball> [package-root]
import { execFileSync } from "node:child_process";
import { rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { SHRINKWRAP_FILE, shrinkwrapProblem, shrinkwrapText } from "./shrinkwrap.mjs";

/** The text of the shrinkwrap packed in `tarball`, or undefined when it has none. */
function packedShrinkwrap(tarball) {
  try {
    return execFileSync("tar", ["-xzOf", tarball, `package/${SHRINKWRAP_FILE}`], {
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch {
    return undefined;
  }
}

const [mode, ...rest] = process.argv.slice(2);
if (mode === "write") {
  const root = resolve(rest[0] ?? process.cwd());
  writeFileSync(join(root, SHRINKWRAP_FILE), shrinkwrapText(root));
} else if (mode === "remove") {
  rmSync(join(resolve(rest[0] ?? process.cwd()), SHRINKWRAP_FILE), { force: true });
} else if (mode === "check" && rest[0] !== undefined) {
  const [tarball, root = process.cwd()] = rest;
  const problem = shrinkwrapProblem(packedShrinkwrap(tarball), shrinkwrapText(resolve(root)));
  if (problem !== undefined) {
    process.stderr.write(`${tarball}: ${problem}\n`);
    process.exit(1);
  }
  console.log(`${tarball} ships the ${SHRINKWRAP_FILE} package-lock.json gives.`);
} else {
  process.stderr.write("usage: shrinkwrap-pack.mjs write|remove [package-root] | check <tarball> [package-root]\n");
  process.exit(2);
}
