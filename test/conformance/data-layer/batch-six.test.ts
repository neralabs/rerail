import { closeSync, existsSync, mkdtempSync, openSync, readFileSync } from "node:fs";
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";
import { fixturesPath } from "../../support/oracles.js";

const paths = ["batch-06-recording.json", "batch-06-probes.json"]
  .map(name => fixturesPath("host", "data-layer-port", name));

// The recording is not in git (30 MB of captured source): `data_layer_record.py --batch 6` writes it.
it.skipIf(!existsSync(paths[0]))("matches positive and clean source observations for all ten sixth-batch candidates", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-batch-six-"));
  const output = join(directory, "result.json"), log = join(directory, "replay.log");
  // Profiling builds emit large matcher descriptions. A file keeps their writes
  // off the fork runner's nonblocking stderr pipe.
  const descriptor = openSync(log, "w");
  try {
    await new Promise<void>((resolve, reject) => {
      const child = spawn(process.execPath, ["--import", "tsx", fileURLToPath(new URL("../../../conformance/data-layer/replay.ts", import.meta.url)),
        ...paths.flatMap(path => ["--recording", path]), "--output", output], { stdio: ["ignore", descriptor, descriptor] });
      child.once("error", reject);
      child.once("close", (code, signal) => code === 0 ? resolve()
        : reject(new Error(`Source replay exited ${code ?? signal}: ${readFileSync(log, "utf8").slice(-4000)}`)));
    });
  } finally { closeSync(descriptor); }
  expect(JSON.parse(readFileSync(output, "utf8"))).toMatchObject({
    nativeCalls: 1259, positiveCalls: 417, cleanCalls: 842,
    falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0, missingCoverage: [],
  });
}, 600_000);
