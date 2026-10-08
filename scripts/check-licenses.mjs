#!/usr/bin/env node
// Licence gate: Rerail is permissively licensed throughout.
//
// 1. Repository: every licence text in LICENSES/ must be on the allowlist. `reuse lint` (run beside this
//    script in CI) proves every file is annotated and every licence it names has its text in LICENSES/, so a
//    file under any other licence cannot be committed without failing one of the two checks.
// 2. Distribution: no production dependency of any published package (everything npm installs with it)
//    may carry a copyleft licence. Development-only dependencies are never distributed and are not checked.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/** SPDX ids allowed for files committed to this repository. */
const ALLOWED_LICENSES = new Set([
  "0BSD",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "CC0-1.0",
  "ISC",
  "MIT",
  "Unicode-3.0",
  "Zlib",
]);

/** Licence families that impose terms on the work they are combined with. */
const COPYLEFT = /\b(A?GPL|LGPL|MPL|EPL|EUPL|CDDL|CPAL|OSL|SSPL|RPL|QPL|Sleepycat|CC-BY-SA|CC-BY-NC)\b/iu;

const failures = [];

for (const file of readdirSync(join(root, "LICENSES"))) {
  const id = file.replace(/\.txt$/u, "");
  if (!ALLOWED_LICENSES.has(id)) failures.push(`LICENSES/${file}: ${id} is not on the repository licence allowlist`);
}

const lock = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));
for (const [path, entry] of Object.entries(lock.packages)) {
  // The root and workspace packages are this repository's own code, covered by the repository check.
  if (path === "" || !path.includes("node_modules/") || entry.dev === true || entry.link === true) continue;
  const license = typeof entry.license === "string" ? entry.license : JSON.stringify(entry.license ?? "UNKNOWN");
  if (COPYLEFT.test(license)) failures.push(`${path}: production dependency licensed ${license}`);
}

if (failures.length > 0) {
  process.stderr.write(`Licence check failed:\n${failures.map(failure => `  ${failure}`).join("\n")}\n`);
  process.exit(1);
}
process.stdout.write("Licence check passed: repository licences are on the allowlist and no production dependency is copyleft.\n");
