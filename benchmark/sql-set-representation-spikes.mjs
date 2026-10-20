// Test-only representation experiments, with raw output and binary/source provenance.
// Build the release test executable first, then supply --binary, --output and optionally --rounds.
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { cpus, loadavg } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const { values } = parseArgs({ options: {
  binary: { type: "string" }, output: { type: "string" }, rounds: { type: "string", default: "3" },
} });
if (!values.binary || !values.output) throw new Error("Usage: --binary /path/to/release-test --output /tmp/results.json [--rounds 3]");
const rounds = Number(values.rounds);
if (!Number.isInteger(rounds) || rounds < 1 || rounds > 100) throw new Error("rounds must be 1..100");
const binary = resolve(values.binary), output = resolve(values.output);
mkdirSync(dirname(output), { recursive: true });
const hash = value => createHash("sha256").update(value).digest("hex");
const source = fileURLToPath(new URL("../native/src/code_rule_dsl/sql/spikes/", import.meta.url));
const result = {
  binary, binarySha256: hash(readFileSync(binary)),
  sourceHashes: readdirSync(source).filter(path => path.endsWith(".rs")).sort()
    .map(path => ({ path, sha256: hash(readFileSync(resolve(source, path))) })),
  machine: { cpu: cpus()[0]?.model, logicalCpus: cpus().length },
  timing: "elapsed milliseconds; shared host; no application throughput claim", records: [],
};
for (let round = 0; round < rounds; round++) {
  const load = loadavg();
  const child = spawnSync(binary, ["code_rule_dsl::sql::tests::spikes::", "--ignored", "--nocapture", "--test-threads=1"], {
    encoding: "utf8", maxBuffer: 32 * 1024 * 1024, timeout: 180_000,
  });
  const raw = child.stdout + child.stderr;
  writeFileSync(`${output}.${round}.log`, raw);
  const measurements = [...raw.matchAll(/SPIKE (\{[^\n]+\})/g)].map(match => JSON.parse(match[1]));
  result.records.push({ round, load, status: child.status, error: child.error?.message, measurements });
  writeFileSync(output, JSON.stringify(result, null, 2) + "\n");
  if (child.status !== 0 || measurements.length !== 25) throw new Error(`Spike round ${round} failed or emitted an incomplete result; see ${output}.${round}.log`);
}
console.log(JSON.stringify(result.records.map(({ round, measurements }) => ({ round, measurements })), null, 2));
