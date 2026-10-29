// Records lizard 1.24.0's cyclomatic complexity for the complexity rule pack's parity corpus
// (fixtures/ beside this file) into lizard-1.24.0.tsv beside this file, which
// test/engine/code-rule/complexity-rules.test.ts reads, then writes provenance.json and LICENSES.md (every file is
// Rerail's: UPSTREAM.md lists the corpus and the TSV is recorded). `npm run fixtures:derive -- --pack complexity` runs
// it after the corpus changes; it needs uv on PATH. lizard is never a runtime dependency; CI runs it only to derive the
// TSV. The corpus is authored pack test data, in S3; the TSV is derived, never committed (CONTRIBUTING.md, "Pack test
// data").
import { spawnSync } from "node:child_process";
import { writeFile } from "node:fs/promises";
import { relative } from "node:path";
import { fileURLToPath } from "node:url";
import { writeProvenance } from "../../../../scripts/pack-data/provenance.mjs";
import { packFixturesWritten, requirePackFixture } from "../../../../scripts/s3/pack-fixtures.mjs";

const LIZARD = "lizard==1.24.0";
const corpus = requirePackFixture(fileURLToPath(new URL("fixtures/", import.meta.url)));
const target = new URL("lizard-1.24.0.tsv", import.meta.url);

/** lizard's CSV over the corpus; `-m` is its modified count, where a whole switch or match adds 1 instead of 1 per case. */
function lizard(...flags) {
  const run = spawnSync("uvx", ["--from", LIZARD, "lizard", ...flags, "--csv", corpus], { encoding: "utf8" }); // NOSONAR - a manual generator that runs uvx from PATH
  if (run.error !== undefined) throw new Error(`uvx not found on PATH (${run.error.message}); install uv to record lizard scores`);
  if (run.status !== 0) throw new Error(`lizard failed with status ${run.status}: ${run.stderr}`);
  return run.stdout;
}

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

// lizard --csv columns: NLOC, CCN, token, PARAM, length, location, file, function, long name, start, end.
const scores = csv => csv
  .split("\n")
  .filter(line => line.trim() !== "")
  .map(csvFields)
  .map(fields => ({ path: relative(corpus, fields[6]).replaceAll("\\", "/"), name: fields[7], line: Number(fields[9]), ccn: Number(fields[1]) }));
// The lower of the standard and modified counts, as slapkit scores a function: a switch with at least one case, or a
// match, adds 1 however many cases it has. min() keeps a switch with no case (default only) at the standard count, 0.
const modified = new Map(scores(lizard("-m")).map(row => [`${row.path}:${row.line}`, row.ccn]));
const rows = scores(lizard())
  .map(row => {
    const ccn = modified.get(`${row.path}:${row.line}`);
    if (ccn === undefined) throw new Error(`lizard -m did not score ${row.path}:${row.line}`);
    return { ...row, ccn: Math.min(row.ccn, ccn) };
  })
  // By path in code-unit order (not locale order, so the TSV is the same on every machine), then by line.
  .sort((a, b) => Number(a.path > b.path) - Number(a.path < b.path) || a.line - b.line);
if (rows.length === 0) throw new Error("lizard scored no functions in the corpus");
const header = `# Recorded by packs/complexity/upstream/lizard/generate.mjs with ${LIZARD}, ccn = min(standard, -m); paths are relative to fixtures/ beside this file.\npath\tfunction\tline\tccn\n`;
await writeFile(target, header + rows.map(row => `${row.path}\t${row.name}\t${row.line}\t${row.ccn}\n`).join(""));
console.log(`lizard: ${rows.length} functions -> ${relative(process.cwd(), fileURLToPath(target))}`);
console.log(packFixturesWritten([fileURLToPath(target)]));
writeProvenance({ pack: "complexity", tool: "lizard", entries: {}, commits: {} });
