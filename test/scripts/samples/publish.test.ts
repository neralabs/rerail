import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join, relative, sep } from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";

import { samplesArchiveCachePath, samplesTree, samplesTreeDirectory } from "../../../scripts/s3/pack-fixtures.mjs";
import { buildArchive, compressArchive, lockText } from "../../../scripts/s3/test-data-archive.mjs";
import { publishSamplesDirectory, writeSampleSet } from "../../../scripts/samples/pack.js";
import { recordedLicences } from "../../../scripts/samples/licences.js";
import { checkLicenceTexts, licenceCache, publishedSamples } from "../../../scripts/samples/publish.js";
import { cacheLicence, writeSamplesTree } from "../../support/samples.js";

const roots: string[] = [];
afterEach(() => {
  vi.unstubAllEnvs();
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

/** `ONE`'s entries with its labels' `source` changed by `edit`. */
function sampleWith(edit: Record<string, string>, sample = ONE): { path: string; bytes: Buffer }[] {
  return sampleEntries([sample]).map(entry => {
    if (!entry.path.endsWith(".expected.json")) return entry;
    const labels = JSON.parse(entry.bytes.toString("utf8"));
    return { ...entry, bytes: Buffer.from(JSON.stringify({ ...labels, source: { ...labels.source, ...edit } })) };
  });
}

/** `ONE` staged with its labels' `source` changed by `edit`; the error publishedSamples throws, or "" when it passes. */
function refusalWith(edit: Record<string, string>): string {
  try {
    publishedSamples(sampleWith(edit));
    return "";
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}

describe("publishedSamples on the source licence", () => {
  it("refuses a declared source.license that is not the repository's recorded licence", () => {
    expect(refusalWith({ license: "Apache-2.0" })).toMatch(/its source\.license "Apache-2\.0" is not fastify\/fastify's recorded licence at [0-9a-f]{12}, "MIT"/u);
  });

  it("judges a sample by the licence recorded at its own commit, where that differs from the repository's", () => {
    // jest was BSD-3-Clause with a patent grant until 2018, then MIT.
    const bsd = "242fb35459b387e360f52394b0e30f42101cc51d";
    const later = "f".repeat(40);
    expect(refusalWith({ repo: "jestjs/jest", commit: bsd, license: "BSD-3-Clause" })).toBe("");
    expect(refusalWith({ repo: "jestjs/jest", commit: bsd, license: "MIT" })).toContain(`recorded licence at 242fb35459b3, "BSD-3-Clause"`);
    expect(refusalWith({ repo: "jestjs/jest", commit: later, license: "MIT" })).toBe("");
    expect(refusalWith({ repo: "jestjs/jest", commit: later, license: "BSD-3-Clause" })).toContain(`recorded licence at ffffffffffff, "MIT"`);
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

  it("records every repository the committed SAMPLES.md tables list, under each licence they list", () => {
    // SAMPLES.md sits at packs/<pack>/upstream/<tool>/; walking all of packs/ would read every pulled fixture.
    const listed = readdirSync("packs")
      .flatMap(pack => (existsSync(join("packs", pack, "upstream")) ? readdirSync(join("packs", pack, "upstream")).map(tool => join("packs", pack, "upstream", tool, "SAMPLES.md")) : []))
      .filter(path => existsSync(path))
      .flatMap(path => [...readFileSync(path, "utf8").matchAll(/^\| ([\w.-]+\/[\w.-]+) \| ([^|]+?) \|/gmu)])
      .flatMap(([, repo, licences]) => licences!.split(", ").map(licence => [repo!, recordedLicences(repo!).join(", ") || "(none)", licence]));
    expect(listed.length).toBeGreaterThan(0);
    expect(listed.filter(([, recorded, licence]) => !recorded!.split(", ").includes(licence!))).toEqual([]);
  });
});

describe("checkLicenceTexts", () => {
  const BSD = "BSD License\n\nRedistribution and use in source and binary forms ... Neither the name Facebook\n";
  // jest at 242fb35: BSD-3-Clause with a patent grant (LICENSE and PATENTS).
  const JEST_BSD = "242fb35459b387e360f52394b0e30f42101cc51d";
  // sequelize in September 2010, which nothing at the commit gives a licence.
  const SEQUELIZE_NONE = "155381bd5ddfc8d4b7a6f1cd8385bfca60921ada";

  /** checkLicenceTexts's error over `entries` with the cache under `root`, or "" when it passes. */
  function failure(root: string, entries: readonly { path: string; bytes: Buffer }[]): string {
    try {
      checkLicenceTexts(entries, licenceCache(root));
      return "";
    } catch (error) {
      return error instanceof Error ? error.message : String(error);
    }
  }

  it("passes samples whose licence files read, at each sample's commit, as the licence recorded for it", () => {
    const root = tempRoot();
    cacheLicence(root);
    cacheLicence(root, { repo: "jestjs/jest", commit: JEST_BSD, text: BSD });
    cacheLicence(root, { repo: "jestjs/jest", commit: JEST_BSD, file: "PATENTS", text: "Additional Grant of Patent Rights Version 2\n" });
    const jest = sampleWith({ repo: "jestjs/jest", commit: JEST_BSD, license: "BSD-3-Clause" }, { ...ONE, name: "jest" });
    expect(failure(root, [...sampleEntries([ONE]), ...jest])).toBe("");
  });

  it("names each commit whose licence does not hold, with one of its samples: a mismatch, a copyleft text, no licence, an unrecorded repository", () => {
    const root = tempRoot();
    // A 2017 jest commit the table gives no entry of its own, so it is held to jest's MIT, whose LICENSE reads BSD.
    const eslintJest = "27b53ca4fe0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e";
    cacheLicence(root, { repo: "jestjs/jest", commit: eslintJest, text: BSD });
    cacheLicence(root, { repo: "django/django", text: "GNU GENERAL PUBLIC LICENSE\nVersion 2\n" });
    const entries = [
      ...sampleWith({ repo: "jestjs/jest", commit: eslintJest }, { ...ONE, name: "jest" }),
      ...sampleWith({ repo: "django/django", license: "BSD-3-Clause" }, { ...ONE, name: "django" }),
      ...sampleWith({ repo: "sequelize/sequelize", commit: SEQUELIZE_NONE, license: "NONE" }, { ...ONE, name: "sequelize" }),
      ...sampleWith({ repo: "someone/unknown" }, { ...ONE, name: "unknown" }),
    ];
    const message = failure(root, entries);
    expect(message).toContain("publish/samples/ holds samples whose licence does not hold at their commit:");
    expect(message).toMatch(new RegExp(`jestjs/jest at ${eslintJest}: LICENSE do not read as MIT \\(1 sample\\(s\\), such as jest-src-a-js-[0-9a-f]{12}/src/a\\.js\\)`, "u"));
    expect(message).toContain(`django/django at ${"0".repeat(40)}: its licence files name a copyleft licence`);
    expect(message).toContain(`sequelize/sequelize at ${SEQUELIZE_NONE}: nothing at the commit states a licence, so no file of it may be copied`);
    expect(message).toContain("someone/unknown: scripts/samples/licences.ts records no licence files for it");
  });

  it("says which recorded licence file GitHub does not have at the commit", () => {
    const root = tempRoot();
    const bin = join(root, "bin");
    mkdirSync(bin);
    writeFileSync(join(bin, "gh"), "#!/bin/sh\necho 'gh: Not Found (HTTP 404)' >&2\nexit 1\n");
    chmodSync(join(bin, "gh"), 0o755);
    vi.stubEnv("PATH", `${bin}${delimiter}${process.env.PATH ?? ""}`);
    expect(failure(root, sampleEntries([ONE]))).toContain(
      `fastify/fastify at ${"0".repeat(40)}: LICENSE, which scripts/samples/licences.ts records for this commit, could not be read from GitHub (gh: Not Found (HTTP 404))`,
    );
  });
});

describe("writeSampleSet", () => {
  it("stages the set under publish/samples/ with a REUSE.toml licensing each sample under its source's licence", () => {
    const root = tempRoot();
    const entries = sampleEntries([ONE]);
    cacheLicence(root);
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

describe("writeSampleSet on the licence at each sample's commit", () => {
  it("refuses a set whose repository's licence file does not read as recorded at a sample's commit, and stages nothing", () => {
    const root = tempRoot();
    cacheLicence(root, { text: "Apache License, Version 2.0\n" });
    expect(() => writeSampleSet(root, TOOL_PACK, sampleEntries([ONE]))).toThrow(/fastify\/fastify at 0{40}: LICENSE do not read as MIT/u);
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
    cacheLicence(root);
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
    cacheLicence(root);
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
