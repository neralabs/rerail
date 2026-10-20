// Checks a converted policy directory (convert.mts `--out`) against what the loop was selected with; `setup.sh` runs it.
//
//   node --import tsx benchmark/data-layer/setup/verify.mts --policies <dir> --nera-landing <clone> --commit <sha>
//
// - Each oracle file (`manifest.json`, `tree.expected.json`, `unsupported.json`, `mutations/`, and `generate.py`) is
//   the nera-landing commit's `tempo-oracles/data-layer/` blob, byte for byte (git blob ids), with none missing or extra.
// - `conversion-report.json` beside the directory converted every linter from that commit.
// - Each `<linter>/policy.json` hashes (sha256 of the file's bytes, as `loop.ts select` records it) to the
//   `policies` entry of `benchmark/data-layer/loop-subset.json`, and the linters are the same set.
//
// Prints one line per problem and exits 1 when there is any, 0 otherwise.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";

import { rerailRoot, sha256 } from "../engine.js";

const args = new Map<string, string>();
for (let index = 2; index < process.argv.length; index += 2) args.set(process.argv[index]!.replace(/^--/, ""), process.argv[index + 1]!);
const required = (name: string) => {
  const value = args.get(name);
  if (value === undefined) throw new Error(`--${name} is required`);
  return value;
};
const policies = resolve(required("policies"));
const neraLanding = resolve(required("nera-landing"));
const commit = required("commit");
const oracleRoot = "tempo-oracles/data-layer/";

const problems: string[] = [];
const subset = JSON.parse(readFileSync(resolve(rerailRoot, "benchmark/data-layer/loop-subset.json"), "utf8")) as { policies: Record<string, string> };
const linters = Object.keys(subset.policies).sort();

if (!existsSync(policies)) {
  console.log(`no policies at ${policies}`);
  process.exit(1);
}

// The oracle files: the commit's blobs under each linter directory, and the recorder.
const tree = spawnSync("git", ["-C", neraLanding, "ls-tree", "-r", "-z", commit, "--", oracleRoot], { maxBuffer: 64 * 1024 * 1024 });
if (tree.status !== 0) throw new Error(`git ls-tree ${commit} failed: ${tree.stderr.toString().trim()}`);
const expected = new Map<string, string>();
for (const entry of tree.stdout.toString("utf8").split("\0")) {
  if (entry.length === 0) continue;
  const [meta, path] = entry.split("\t") as [string, string];
  const name = path.slice(oracleRoot.length);
  if (name === "generate.py" || (name.includes("/") && !name.endsWith("/policy.json"))) expected.set(name, meta.split(" ")[2]!);
}
const blobId = (bytes: Buffer) => createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
const present = new Set<string>();
const walk = (directory: string) => {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) walk(path);
    else present.add(relative(policies, path));
  }
};
walk(policies);
let oracleMismatches = 0;
for (const [name, blob] of expected) {
  if (!present.has(name)) problems.push(`missing oracle file ${name}`);
  else if (blobId(readFileSync(resolve(policies, name))) !== blob) problems.push(`oracle file ${name} differs from ${commit.slice(0, 9)}`);
  else continue;
  oracleMismatches += 1;
}
const policyFiles = new Set(linters.map(linter => `${linter}/policy.json`));
for (const name of present) if (!expected.has(name) && !policyFiles.has(name)) problems.push(`unexpected file ${name}`);

// The conversion report.
const reportPath = resolve(dirname(policies), "conversion-report.json");
if (!existsSync(reportPath)) problems.push(`no conversion report at ${reportPath}`);
else {
  const report = JSON.parse(readFileSync(reportPath, "utf8")) as { source: { commit: string }; converted: number; failed: number };
  if (report.source.commit !== commit) problems.push(`converted from ${report.source.commit}, not ${commit}`);
  if (report.converted !== linters.length || report.failed !== 0) problems.push(`${report.converted} converted and ${report.failed} not; loop-subset.json has ${linters.length} policies`);
}

// The policies, against the hashes the subset was selected with.
const converted = readdirSync(policies, { withFileTypes: true }).filter(entry => entry.isDirectory()).map(entry => entry.name).sort();
for (const linter of converted) if (!(linter in subset.policies)) problems.push(`policy ${linter} is not in loop-subset.json`);
let hashMismatches = 0;
for (const linter of linters) {
  const path = resolve(policies, linter, "policy.json");
  if (!existsSync(path)) problems.push(`no policy ${linter}/policy.json`);
  else if (sha256(readFileSync(path)) !== subset.policies[linter]) problems.push(`${linter}/policy.json hashes to ${sha256(readFileSync(path))}, loop-subset.json records ${subset.policies[linter]}`);
  else continue;
  hashMismatches += 1;
}

for (const problem of problems) console.log(problem);
if (problems.length === 0) console.log(`${linters.length} policies match loop-subset.json; ${expected.size} oracle files match ${commit.slice(0, 9)}`);
else console.log(`${problems.length} problem${problems.length === 1 ? "" : "s"} (${oracleMismatches} oracle files, ${hashMismatches} policy hashes)`);
process.exitCode = problems.length === 0 ? 0 : 1;
