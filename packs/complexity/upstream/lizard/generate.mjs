// Records lizard's cyclomatic complexity for the complexity rule pack's parity corpus (fixtures/ beside this file), into
// two files beside it:
// - lizard-<VERSION>.tsv, every function's score, which test/engine/code-rule/complexity-rules.test.ts reads;
// - fixtures.expected.json, lizard's findings as the conformance check reads a tool's (conformance/README.md): one
//   CYCLOMATIC_COMPLEXITY finding, at the function's first line, per function scored above the pack's limit of 6, on
//   the corpus files some rule's scope covers. The test files the scopes leave out are complexity-rules.test.ts's.
// Then it writes provenance.json and LICENSES.md (every file is Rerail's: UPSTREAM.md lists the corpus, and the rest is
// recorded). `npm run fixtures:derive -- --pack complexity` runs it after the corpus changes, with the lizard of
// scripts/pack-data/runtimes/lizard-<VERSION> (scripts/pack-data/recipes/complexity.ts). lizard is never a runtime
// dependency. The corpus is authored pack test data, in S3; what this writes is derived, never committed
// (CONTRIBUTING.md, "Pack test data").
import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { matchesGlob, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { writeProvenance } from "../../../../scripts/pack-data/provenance.mjs";
import { packFixturesWritten, requirePackFixture } from "../../../../scripts/s3/pack-fixtures.mjs";

const VERSION = "1.24.0";
/** The release the scores are lizard's at: its tag's commit. */
const ORIGIN = "https://github.com/terryyin/lizard@308b1c3efd8c1c69bcc3eb82deeaec64fd3662ec";
/** The rules' violation code, which conformance.json names as the upstream rule each port ports. */
const RULE = "CYCLOMATIC_COMPLEXITY";
/** The pack's ceiling: a function scored above it is a finding. */
const LIMIT = 6;

const [executable] = process.argv.slice(2);
if (executable === undefined) throw new Error(`usage: node generate.mjs <lizard ${VERSION} executable>; npm run fixtures:derive -- --pack complexity installs it`);
const corpus = requirePackFixture(fileURLToPath(new URL("fixtures/", import.meta.url)));
const scoresTarget = fileURLToPath(new URL(`lizard-${VERSION}.tsv`, import.meta.url));
const findingsTarget = fileURLToPath(new URL("fixtures.expected.json", import.meta.url));

/** lizard's output for `flags`; `-m` is its modified count, where a whole switch or match adds 1 instead of 1 per case. */
function lizard(...flags) {
  const run = spawnSync(executable, flags, { encoding: "utf8" });
  if (run.error !== undefined) throw new Error(`${executable} cannot run (${run.error.message}); npm run fixtures:derive -- --pack complexity installs it`);
  if (run.status !== 0) throw new Error(`lizard failed with status ${run.status}: ${run.stderr}`);
  return run.stdout;
}

const installed = lizard("--version").trim();
if (installed !== VERSION) throw new Error(`${executable} is lizard ${installed}, not ${VERSION}`);

/** One RFC 4180 CSV line: lizard quotes fields that hold commas, such as a long-name parameter list. */
function csvFields(line) {
  const fields = [];
  let field = "";
  let quoted = false;
  for (let index = 0; index < line.length; index++) {
    const char = line[index];
    if (quoted && char === '"' && line[index + 1] === '"') {
      field += '"';
      index++;
    } else if (char === '"') {
      quoted = !quoted;
    } else if (char === "," && !quoted) {
      fields.push(field);
      field = "";
    } else {
      field += char;
    }
  }
  fields.push(field);
  return fields;
}

const byPath = (a, b) => Number(a > b) - Number(a < b);

// lizard --csv columns: NLOC, CCN, token, PARAM, length, location, file, function, long name, start, end.
const scores = csv => csv
  .split("\n")
  .filter(line => line.trim() !== "")
  .map(csvFields)
  .map(fields => ({ path: relative(corpus, fields[6]).replaceAll("\\", "/"), name: fields[7], line: Number(fields[9]), ccn: Number(fields[1]) }));
// The lower of the standard and modified counts, as slapkit scores a function: a switch with at least one case, or a
// match, adds 1 however many cases it has. min() keeps a switch with no case (default only) at the standard count, 0.
const modified = new Map(scores(lizard("-m", "--csv", corpus)).map(row => [`${row.path}:${row.line}`, row.ccn]));
const rows = scores(lizard("--csv", corpus))
  .map(row => {
    const ccn = modified.get(`${row.path}:${row.line}`);
    if (ccn === undefined) throw new Error(`lizard -m did not score ${row.path}:${row.line}`);
    return { ...row, ccn: Math.min(row.ccn, ccn) };
  })
  // By path in code-unit order (not locale order, so the files are the same on every machine), then by line.
  .sort((a, b) => byPath(a.path, b.path) || a.line - b.line);
if (rows.length === 0) throw new Error("lizard scored no functions in the corpus");
const header = `# Recorded by packs/complexity/upstream/lizard/generate.mjs with lizard ${VERSION}, ccn = min(standard, -m); paths are relative to fixtures/ beside this file.\npath\tfunction\tline\tccn\n`;
await writeFile(scoresTarget, header + rows.map(row => `${row.path}\t${row.name}\t${row.line}\t${row.ccn}\n`).join(""));

// The units: every corpus file a rule's scope covers, as the engine reads a rule document's scope globs.
const scopes = readdirSync(fileURLToPath(new URL("../../rules/", import.meta.url)))
  .filter(name => name.endsWith(".json"))
  .map(name => JSON.parse(readFileSync(fileURLToPath(new URL(`../../rules/${name}`, import.meta.url)), "utf8")).scope);
const inScope = path => scopes.some(scope => scope.include.some(glob => matchesGlob(path, glob)) && !scope.exclude.some(glob => matchesGlob(path, glob)));
const files = readdirSync(corpus, { recursive: true, withFileTypes: true })
  .filter(entry => entry.isFile())
  .map(entry => relative(corpus, `${entry.parentPath}/${entry.name}`).replaceAll("\\", "/"))
  .filter(inScope)
  .sort(byPath);
const expected = {
  tool: "lizard",
  version: VERSION,
  source: {
    kind: "fixtures",
    origin: ORIGIN,
    rules: [RULE],
    files,
    units: Object.fromEntries(files.map(path => [path, { rules: [RULE] }])),
  },
  findings: rows.filter(row => row.ccn > LIMIT && files.includes(row.path)).map(row => ({ rule: RULE, path: row.path, line: row.line })),
};
await writeFile(findingsTarget, `${JSON.stringify(expected, null, 1)}\n`);

console.log(`lizard: ${rows.length} functions, ${expected.findings.length} over ${LIMIT} in ${files.length} in-scope files -> ${relative(process.cwd(), scoresTarget)}, ${relative(process.cwd(), findingsTarget)}`);
console.log(packFixturesWritten([scoresTarget, findingsTarget]));
writeProvenance({ pack: "complexity", tool: "lizard", entries: {}, commits: {} });
