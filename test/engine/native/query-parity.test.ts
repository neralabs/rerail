import { execFile } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { afterEach, describe, expect, it } from "vitest";
import { compareSides, coverage, parseOptions } from "../../../benchmark/query-parity.js";
import { resolveNativeBindingPath } from "../../../src/engine/native/native.js";

const execute = promisify(execFile);
const rerailRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const script = join(rerailRoot, "benchmark", "query-parity.ts");

const temporaryRoots: string[] = [];
afterEach(async () => {
  for (const root of temporaryRoots.splice(0)) await rm(root, { recursive: true, force: true });
});

/** A corpus directory holding `files`, with a manifest listing them. */
async function corpus(files: Record<string, string>): Promise<{ root: string; manifest: string }> {
  const root = await mkdtemp(join(tmpdir(), "rerail-query-parity-"));
  temporaryRoots.push(root);
  for (const [path, source] of Object.entries(files)) {
    await mkdir(dirname(join(root, "corpus", path)), { recursive: true });
    await writeFile(join(root, "corpus", path), source);
  }
  const manifest = join(root, "manifest.json");
  await writeFile(manifest, JSON.stringify(Object.keys(files)));
  return { root: join(root, "corpus"), manifest };
}

describe("query parity comparison", () => {
  it("reports different incremental edit applies", () => {
    const differences = compareSides([
      {
        side: { name: "reference", addon: "/reference.node", environment: {} },
        workloads: { edits: { applies: [{ findings: { value: [{ id: "before" }] } }] } },
      },
      {
        side: { name: "candidate", addon: "/candidate.node", environment: {} },
        workloads: { edits: { applies: [{ findings: { value: [{ id: "after" }] } }] } },
      },
    ]);

    expect(differences).toEqual([expect.objectContaining({ side: "candidate", workload: "edits", section: "applies" })]);
  });

  it("ignores raw edit hashes and violation order when applies are semantically equal", () => {
    const differences = compareSides([
      {
        side: { name: "reference", addon: "/reference.node", environment: {} },
        workloads: {
          edits: {
            applies: [{
              status: "completed",
              findings: { value: [{ id: "a", message: "first" }, { id: "b", message: "second" }] },
              failures: { value: [] },
              coverage: { value: [] },
              rawFindingSha256: "sha256:reference-order",
            }],
          },
        },
      },
      {
        side: { name: "candidate", addon: "/candidate.node", environment: {} },
        workloads: {
          edits: {
            applies: [{
              status: "completed",
              findings: { value: [{ message: "second", id: "b" }, { message: "first", id: "a" }] },
              failures: { value: [] },
              coverage: { value: [] },
              rawFindingSha256: "sha256:candidate-order",
            }],
          },
        },
      },
    ]);

    expect(differences).toEqual([]);
  });
});

describe("query parity coverage", () => {
  it("counts failures for their code rule and clause", () => {
    const records = coverage([
      { path: "policy.json", source: JSON.stringify({ id: "code-rule-a" }), clauses: [{ id: "clause-a" }, { id: "clause-b" }] },
    ], [
      { ruleId: "code-rule-a", clauseId: "clause-a", code: "source-partial", message: "incomplete source" },
      { ruleId: "code-rule-a", clauseId: "clause-a", code: "budget-exceeded", message: "budget exhausted" },
    ]);

    expect(records).toEqual([
      { policyId: "code-rule-a", checkId: "clause-a", status: "partial", failureCount: 2 },
      { policyId: "code-rule-a", checkId: "clause-b", status: "complete", failureCount: 0 },
    ]);
  });
});

describe("query parity options", () => {
  const sides = ["--side", "reference=/reference.node", "--side", "candidate=/candidate.node"];

  it("requires a corpus and manifest for every corpus workload", () => {
    expect(() => parseOptions(sides, {})).toThrow(/A corpus is required for corpus, resolves, usage, edits/);
    expect(() => parseOptions([...sides, "--corpus", "/corpus"], {})).toThrow(/--manifest <paths.json>.* go together/);
    expect(parseOptions([...sides, "--workload", "select-when"], {}).corpus).toBeUndefined();
    expect(parseOptions(sides, { RERAIL_QUERY_PARITY_CORPUS: "/corpus", RERAIL_QUERY_PARITY_MANIFEST: "/manifest.json" }).corpus)
      .toEqual({ root: "/corpus", manifest: "/manifest.json" });
    expect(parseOptions([...sides, "--corpus", "/flag", "--manifest", "/flag.json"], { RERAIL_QUERY_PARITY_CORPUS: "/env", RERAIL_QUERY_PARITY_MANIFEST: "/env.json" }).corpus)
      .toEqual({ root: "/flag", manifest: "/flag.json" });
  });

  it("requires a reference addon unless sides are explicit", () => {
    const corpusOptions = ["--workload", "select-when"];
    expect(() => parseOptions(corpusOptions, {})).toThrow(/RERAIL_QUERY_PARITY_REFERENCE_ADDON/);
    expect(parseOptions(corpusOptions, { RERAIL_QUERY_PARITY_REFERENCE_ADDON: "/reference.node" }).sides.map(side => side.name))
      .toEqual(["reference", "candidate"]);
  });
});

describe("query parity command", () => {
  it("prints its usage and fails when no corpus is given", async () => {
    const failure = await execute(process.execPath, ["--import", "tsx", script], { cwd: rerailRoot, env: { ...process.env, RERAIL_QUERY_PARITY_REFERENCE_ADDON: "" } })
      .then(() => undefined, (error: unknown) => error);
    expect(failure).toMatchObject({
      code: 2,
      stderr: expect.stringMatching(/A corpus is required[\s\S]*Usage: just rerail-query-parity --corpus <dir> --manifest <paths\.json>/u),
    });
  });

  it("finds two sides running the same engine equal over a temporary corpus", async () => {
    const { root, manifest } = await corpus({
      // Cyclomatic complexity 7, above the bundled complexity rule's limit of 6.
      "src/app.ts": `export function grade(a: number): number {\n${[1, 2, 3, 4, 5, 6].map(limit => `  if (a > ${limit}) return ${limit};\n`).join("")}  return 0;\n}\n`,
      "src/util.ts": "export const value = 1;\n",
    });
    const output = join(dirname(root), "report");
    const addon = resolveNativeBindingPath();
    const { stderr } = await execute(process.execPath, [
      "--import", "tsx", script,
      "--side", `reference=${addon}`, "--side", `candidate=${addon}`,
      "--corpus", root, "--manifest", manifest, "--output", output,
      "--workload", "corpus", "--workload", "edits", "--workload", "select-when",
    ], { cwd: rerailRoot, maxBuffer: 64 * 1024 * 1024 });
    expect(stderr).toContain("rerail-query-parity: equal across 2 sides");

    const report: unknown = JSON.parse(await readFile(join(output, "report.json"), "utf8"));
    const complexityFinding = expect.objectContaining({ ruleId: "complexity:cyclomatic-typescript", path: "src/app.ts" });
    const workloads = {
      corpus: { files: 2, findings: { value: expect.arrayContaining([complexityFinding]) } },
      // The initial apply, then one per scripted edit.
      edits: { files: 2, applies: Array.from({ length: 9 }, () => expect.objectContaining({ findings: expect.anything() })) },
    };
    expect(report).toMatchObject({ equal: true, differences: [], results: [{ workloads }, { workloads }] });
  }, 120_000);
});
