#!/usr/bin/env node
// Every tracked path checks out on Windows. CI runs on Linux, so this is what keeps the release workflow's Windows
// jobs from failing at checkout.
//
// Rules, over every tracked path (fixtures and upstream data included: a checkout fails on any path):
//   length      At most MAX_LENGTH characters (UTF-16 code units, as Windows counts them). Windows refuses a path
//               over 260 characters, the drive and clone directory included, unless Git is told to use long paths;
//               MAX_LENGTH leaves 60 for the clone directory, enough for the release runner's `D:\a\rerail\rerail`
//               and a typical `C:\Users\<name>\source\repos\rerail`.
//   characters  No segment holds a character Windows forbids in a file name: `< > : " \ | ? *` or a control character.
//   reserved    No segment is a reserved device name (`CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, `COM0`-`COM9`,
//               `LPT0`-`LPT9`, superscript digits too, any case), alone or before an extension, trailing spaces
//               included (`aux.js`, `nul .js`).
//   trailing    No segment ends with a dot or a space, which Windows drops.
//   case        No two paths, or directories along them, differ only by case: Windows folds them into one. Compared
//               upper-cased, as NTFS's upcase table does; each clash is reported once.
// No exemptions. A fixture that names the recording machine's own directories is a recorder's to refuse
// (`assertPortableCase` in conformance/recorders/eslint-record.mjs), not this check's: a short leak passes every rule here.
//
//   node scripts/check/check-windows-paths.mjs   (npm run check:windows-paths; part of npm run check)
import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const MAX_LENGTH = 200;

const FORBIDDEN = /[<>:"\\|?*\u0000-\u001f]/u;
const RESERVED = /^(con|prn|aux|nul|conin\$|conout\$|com[0-9¹²³]|lpt[0-9¹²³])$/iu;

/** A segment's name as Windows matches it against device names: before the first dot, trailing spaces dropped. */
const deviceStem = (segment) => segment.split(".")[0].replace(/ +$/u, "");

function segmentProblem(segment) {
  if (FORBIDDEN.test(segment)) return `${JSON.stringify(segment)} holds a character Windows forbids in a file name`;
  if (RESERVED.test(deviceStem(segment))) return `${segment} is a reserved Windows device name`;
  if (/[. ]$/u.test(segment)) return `${JSON.stringify(segment)} ends with a dot or a space, which Windows drops`;
  return null;
}

/** Every problem `files` (repository-relative, `/`-separated) would have on Windows, as `{ path, message }`. */
export function windowsPathProblems(files) {
  const problems = [];
  const spelled = new Map();
  const reported = new Set();
  for (const path of [...files].sort()) {
    if (path.length > MAX_LENGTH) {
      problems.push({ path, message: `${path.length} characters; Windows checkouts allow ${MAX_LENGTH} (its 260 less the clone directory)` });
    }
    const segments = path.split("/");
    const bad = segments.map(segmentProblem).find((problem) => problem !== null);
    if (bad !== undefined) problems.push({ path, message: bad });
    let prefix = "";
    for (const segment of segments) {
      prefix = prefix === "" ? segment : `${prefix}/${segment}`;
      const folded = prefix.toUpperCase();
      const first = spelled.get(folded);
      if (first === undefined) spelled.set(folded, prefix);
      else if (first !== prefix) {
        if (!reported.has(prefix)) problems.push({ path, message: `${prefix} and ${first} differ only by case; Windows sees one path` });
        reported.add(prefix);
        break;
      }
    }
  }
  return problems;
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  const files = execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 })
    .split("\0")
    .filter((path) => path !== "");
  const problems = windowsPathProblems(files);
  for (const { path, message } of problems) console.error(`${path}: ${message}`);
  console.log(
    problems.length === 0
      ? `check-windows-paths: ${files.length} tracked paths check out on Windows`
      : `check-windows-paths: ${problems.length} problem(s); Windows cannot check this tree out`,
  );
  process.exit(problems.length === 0 ? 0 : 1);
}
