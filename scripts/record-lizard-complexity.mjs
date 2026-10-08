// Records lizard 1.24.0's cyclomatic complexity for the complexity rule pack's parity corpus
// (test/fixtures/complexity/) into test/fixtures/complexity/lizard-1.24.0.tsv, which
// test/complexity-rules.test.ts reads. Run by hand after changing the corpus:
//   node scripts/record-lizard-complexity.mjs
// It needs uv on PATH. lizard is never a runtime or CI dependency: the TSV is checked in.
import { spawnSync } from "node:child_process";
import { writeFile } from "node:fs/promises";
import { relative } from "node:path";
import { fileURLToPath } from "node:url";

const LIZARD = "lizard==1.24.0";
const corpus = fileURLToPath(new URL("../test/fixtures/complexity/", import.meta.url));
const target = new URL("../test/fixtures/complexity/lizard-1.24.0.tsv", import.meta.url);

const run = spawnSync("uvx", ["--from", LIZARD, "lizard", "--csv", corpus], { encoding: "utf8" }); // NOSONAR - a manual recorder that runs uvx from PATH
if (run.error !== undefined) throw new Error(`uvx not found on PATH (${run.error.message}); install uv to record lizard scores`);
if (run.status !== 0) throw new Error(`lizard failed with status ${run.status}: ${run.stderr}`);

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
const rows = run.stdout
  .split("\n")
  .filter(line => line.trim() !== "")
  .map(csvFields)
  .map(fields => ({ path: relative(corpus, fields[6]).replaceAll("\\", "/"), name: fields[7], line: Number(fields[9]), ccn: Number(fields[1]) }))
  // By path in code-unit order (not locale order, so the TSV is the same on every machine), then by line.
  .sort((a, b) => Number(a.path > b.path) - Number(a.path < b.path) || a.line - b.line);
if (rows.length === 0) throw new Error("lizard scored no functions in the corpus");
const header = `# Recorded by scripts/record-lizard-complexity.mjs with ${LIZARD}; paths are relative to test/fixtures/complexity/.\npath\tfunction\tline\tccn\n`;
await writeFile(target, header + rows.map(row => `${row.path}\t${row.name}\t${row.line}\t${row.ccn}\n`).join(""));
console.log(`record-lizard-complexity: ${rows.length} functions -> ${relative(process.cwd(), fileURLToPath(target))}`);
