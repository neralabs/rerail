/**
 * Hand-made real-code samples trees (scripts/samples/format.ts) for the conformance runner's tests, which reach them
 * through `RunOptions.samplesDirectory` instead of a samples.lock.
 */
import { createHash } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { LABELS_SUFFIX, LICENSES_FILE, type SampleLabels } from "../../scripts/samples/format.js";

export interface HandMadeSample {
  /** The sample id's source short name. */
  name: string;
  /** The file's path in its repository. */
  path: string;
  text: string;
  /** Pack rule id → labelled lines; `[]` is clean for the rule. */
  expected: Record<string, number[]>;
  /** Files beside it, by path in the repository: context, never judged. */
  context?: Record<string, string>;
}

/** Writes `samples` as a samples tree under `directory`; returns each sample's evaluation path, in order. */
export function writeSamplesTree(directory: string, samples: readonly HandMadeSample[]): string[] {
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, LICENSES_FILE), "# Licences\n\nexample/repo: MIT\n");
  return samples.map(sample => {
    const sha256 = createHash("sha256").update(sample.text).digest("hex");
    const id = `${sample.name}-${sample.path.replace(/[^a-z0-9]+/giu, "-").toLowerCase()}-${sha256.slice(0, 12)}`;
    const write = (path: string, text: string): void => {
      mkdirSync(dirname(join(directory, id, path)), { recursive: true });
      writeFileSync(join(directory, id, path), text);
    };
    const labels: SampleLabels = {
      source: { repo: "example/repo", commit: "0".repeat(40), path: sample.path, license: "MIT", sha256 },
      labelledBy: { tool: "examplelint", version: "1.0.0", how: "hand-labelled" },
      expected: Object.fromEntries(Object.entries(sample.expected).map(([rule, lines]) => [rule, lines.map(line => ({ line }))])),
    };
    write(sample.path, sample.text);
    write(`${sample.path}${LABELS_SUFFIX}`, `${JSON.stringify(labels, null, 2)}\n`);
    for (const [path, text] of Object.entries(sample.context ?? {})) write(path, text);
    return `${id}/${sample.path}`;
  });
}
