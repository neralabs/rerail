import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative, sep } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { samplesArchiveCachePath, samplesTree, samplesTreeDirectory } from "../../../scripts/s3/pack-fixtures.mjs";
import { buildArchive, compressArchive, lockText } from "../../../scripts/s3/test-data-archive.mjs";
import { publishSamplesDirectory, writeSampleSet } from "../../../scripts/samples/pack.js";
import { recordedLicence } from "../../../scripts/samples/licences.js";
import { publishedSamples } from "../../../scripts/samples/publish.js";
import { writeSamplesTree } from "../../support/samples.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function tempRoot(): string {
  const root = mkdtempSync(join(tmpdir(), "rerail-publish-samples-"));
  roots.push(root);
  return root;
}

const TOOL_PACK = { pack: "p", tool: "t" };

/** Every file under `directory`, by posix path relative to it, as text. */
function filesIn(directory: string): Record<string, string> {
  return Object.fromEntries(
    readdirSync(directory, { recursive: true, withFileTypes: true })
      .filter(entry => entry.isFile())
      .map(entry => [relative(directory, join(entry.parentPath, entry.name)).split(sep).join("/"), readFileSync(join(entry.parentPath, entry.name), "utf8")]),
  );
}

/** A hand-made sample set as archive entries. */
function sampleEntries(samples: Parameters<typeof writeSamplesTree>[1]): { path: string; bytes: Buffer }[] {
  const directory = tempRoot();
  writeSamplesTree(directory, samples);
  return Object.entries(filesIn(directory)).map(([path, text]) => ({ path, bytes: Buffer.from(text) }));
}

const ONE = { name: "demo", path: "src/a.js", text: "var a = null;\n", expected: { "p-no-null": [1] } };

describe("publishedSamples", () => {
  it("passes a whole staged sample and LICENSES.md, dropping REUSE.toml", () => {
    const entries = sampleEntries([ONE]);
    const paths = publishedSamples([...entries, { path: "REUSE.toml", bytes: Buffer.from("version = 1\n") }]).map(entry => entry.path);
    expect(paths.sort()).toEqual(entries.map(entry => entry.path).sort());
  });

  it("names every problem: a bad path, a sha256 mismatch, a missing file, an unlabelled sample, bad labels, a copyleft source", () => {
    const entries = sampleEntries([ONE]);
    const labels = entries.find(entry => entry.path.endsWith(".expected.json"))!;
    const id = labels.path.slice(0, labels.path.indexOf("/"));
    const parsed = JSON.parse(labels.bytes.toString("utf8"));
    const copyleftId = `gpl-${"b".repeat(12)}`;
    const files = [
      { path: "stray.txt", bytes: Buffer.from("") },
      ...entries.map(entry => (entry.path === `${id}/src/a.js` ? { ...entry, bytes: Buffer.from("edited\n") } : entry)),
      { path: `orphan-${"a".repeat(12)}/x.js`, bytes: Buffer.from("") },
      { path: `bad-${"c".repeat(12)}/x.js.expected.json`, bytes: Buffer.from("{}") },
      { path: `${copyleftId}/y.js.expected.json`, bytes: Buffer.from(JSON.stringify({ ...parsed, source: { ...parsed.source, repo: "grafana/grafana", path: "y.js" } })) },
    ];
    let message = "";
    try {
      publishedSamples(files);
    } catch (error) {
      message = error instanceof Error ? error.message : String(error);
    }
    expect(message).toContain("stray.txt: not LICENSES.md or <sample-id>/<path>");
    expect(message).toContain(`${id}/src/a.js: its sha256`);
    expect(message).toContain(`${copyleftId}/y.js.expected.json: its file ${copyleftId}/y.js is not staged`);
    expect(message).toContain(`${copyleftId}/y.js.expected.json: grafana/grafana is a copyleft source`);
    expect(message).toContain(`orphan-${"a".repeat(12)}/: no labels file`);
    expect(message).toContain(`bad-${"c".repeat(12)}/x.js.expected.json: not sample labels`);
  });
});

/** `ONE` staged with its labels' `source` changed by `edit`; the error publishedSamples throws, or "" when it passes. */
function refusalWith(edit: Record<string, string>): string {
  const entries = sampleEntries([ONE]).map(entry => {
    if (!entry.path.endsWith(".expected.json")) return entry;
    const labels = JSON.parse(entry.bytes.toString("utf8"));
    return { ...entry, bytes: Buffer.from(JSON.stringify({ ...labels, source: { ...labels.source, ...edit } })) };
  });
  try {
    publishedSamples(entries);
    return "";
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}

describe("publishedSamples on the source licence", () => {
  it("refuses a declared source.license that is not the repository's recorded licence", () => {
    expect(refusalWith({ license: "Apache-2.0" })).toContain(`its source.license "Apache-2.0" is not fastify/fastify's recorded licence "MIT"`);
  });

  it("refuses a repository with no recorded licence, whatever licence it declares", () => {
    expect(refusalWith({ repo: "someone/agpl-thing", license: "MIT" })).toContain("someone/agpl-thing has no recorded licence");
  });

  it("refuses a copyleft repository even when it declares a permissive licence", () => {
    expect(refusalWith({ repo: "Grafana/Grafana", license: "MIT" })).toContain("Grafana/Grafana is a copyleft source");
  });

  it("matches the repository case-insensitively", () => {
    expect(refusalWith({ repo: "Fastify/Fastify" })).toBe("");
  });

  it("records every repository the committed SAMPLES.md tables list, under the licence they list", () => {
    const listed = readdirSync("packs", { recursive: true, withFileTypes: true })
      .filter(entry => entry.isFile() && entry.name === "SAMPLES.md")
      .flatMap(entry => [...readFileSync(join(entry.parentPath, entry.name), "utf8").matchAll(/^\| ([\w.-]+\/[\w.-]+) \| ([^|]+?) \|/gmu)])
      .map(([, repo, licence]) => [repo!, recordedLicence(repo!) ?? "(none)", licence!]);
    expect(listed.length).toBeGreaterThan(0);
    expect(listed.filter(([, recorded, licence]) => recorded !== licence)).toEqual([]);
  });
});

describe("writeSampleSet", () => {
  it("stages the set under publish/samples/ with a REUSE.toml licensing each sample under its source's licence", () => {
    const root = tempRoot();
    const entries = sampleEntries([ONE]);
    writeSampleSet(root, TOOL_PACK, entries);
    const staged = filesIn(join(root, publishSamplesDirectory(TOOL_PACK)));
    const id = entries.find(entry => entry.path.endsWith("a.js"))!.path.split("/")[0]!;
    expect(Object.keys(staged).sort()).toEqual([...entries.map(entry => entry.path), "REUSE.toml"].sort());
    expect(staged["REUSE.toml"]).toContain(`path = "${id}/**"\nSPDX-FileCopyrightText = "fastify/fastify contributors"\nSPDX-License-Identifier = "MIT"`);
    expect(existsSync(join(root, "packs/p/upstream/t/samples.lock"))).toBe(false);
  });

  it("refuses an invalid set and stages nothing", () => {
    const root = tempRoot();
    expect(() => writeSampleSet(root, TOOL_PACK, [{ path: "stray.txt", bytes: Buffer.from("") }])).toThrow(/stray\.txt/u);
    expect(existsSync(join(root, publishSamplesDirectory(TOOL_PACK)))).toBe(false);
  });
});

describe("samplesTree with staged samples", () => {
  const noAws = (): never => {
    throw new Error("aws called");
  };

  it("serves the staged set alone when the tool-pack has no samples.lock", () => {
    const root = tempRoot();
    const entries = sampleEntries([ONE]);
    writeSampleSet(root, TOOL_PACK, entries);
    const tree = samplesTree({ root, pack: "p", tool: "t", aws: noAws })!;
    expect(tree).toContain(join(".rerail", "pack-data", "samples", "p", "t", "publish-"));
    const served = filesIn(tree);
    delete served[".tar-sha256"];
    expect(served).toEqual(Object.fromEntries(entries.map(entry => [entry.path, entry.bytes.toString("utf8")])));
  });

  it("lays the staged files over the locked set, file by file, and leaves the locked tree as it is", () => {
    const root = tempRoot();
    const locked = sampleEntries([ONE, { name: "other", path: "b.js", text: "b;\n", expected: { "p-no-null": [] } }]);
    const archive = buildArchive(locked);
    mkdirSync(join(root, "packs/p/upstream/t"), { recursive: true });
    writeFileSync(join(root, "packs/p/upstream/t/samples.lock"), lockText(archive));
    const cached = samplesArchiveCachePath(root, TOOL_PACK, archive.sha256);
    mkdirSync(join(cached, ".."), { recursive: true });
    writeFileSync(cached, compressArchive(archive.tar));
    const staged = sampleEntries([{ ...ONE, expected: { "p-no-null": [] } }]).filter(entry => entry.path !== "LICENSES.md");
    writeSampleSet(root, TOOL_PACK, staged);

    const tree = samplesTree({ root, pack: "p", tool: "t", aws: noAws })!;
    const served = filesIn(tree);
    delete served[".tar-sha256"];
    const expected = Object.fromEntries(locked.map(entry => [entry.path, entry.bytes.toString("utf8")]));
    for (const entry of staged) expected[entry.path] = entry.bytes.toString("utf8");
    expect(served).toEqual(expected);
    expect(tree).not.toBe(samplesTreeDirectory(root, TOOL_PACK, archive.sha256));
    // The same staged set is served from the same tree.
    expect(samplesTree({ root, pack: "p", tool: "t", aws: noAws })).toBe(tree);
  });
});
