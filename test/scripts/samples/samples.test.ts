import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { buildArchive, readArchive } from "../../../scripts/s3/test-data-archive.mjs";
import { eslintTestSetSamples, fixedCopyPath } from "../../../scripts/samples/convert-eslint-test-set.js";
import { packRuleIds, type PackConformance } from "../../../scripts/samples/conformance.js";
import { LABELS_SUFFIX, sampleLabelsSchema } from "../../../scripts/samples/format.js";
import { copyleftReason, licenceTexts, licensesMarkdown, samplesMarkdown } from "../../../scripts/samples/licences.js";
import { archiveEntries, dedupe, labelsText, sampleId, sha256, type Sample } from "../../../scripts/samples/samples.js";

const COMMIT = "0".repeat(40);

function sample(overrides: Partial<Sample> & { path?: string; text?: string } = {}): Sample {
  const bytes = Buffer.from(overrides.text ?? "x = 1\n", "utf8");
  const { path = "pkg/a.py", text: _text, ...rest } = overrides;
  return {
    toolPack: { pack: "python", tool: "ruff" },
    short: "demo",
    source: { repo: "owner/demo", commit: COMMIT, path, license: "MIT", sha256: sha256(bytes) },
    bytes,
    labelledBy: { tool: "ruff", version: "0.16.9", how: "port-readiness recording" },
    expected: new Map(),
    context: [],
    pinned: false,
    origins: ["golden"],
    ...rest,
  };
}

const conformance = (rules: PackConformance["rules"]): PackConformance => ({ pack: "rust", family: "rust", rules });

const temporary: string[] = [];
afterEach(() => {
  for (const directory of temporary.splice(0)) rmSync(directory, { recursive: true, force: true });
});
const scratch = (): string => {
  const directory = mkdtempSync(join(tmpdir(), "samples-test-"));
  temporary.push(directory);
  return directory;
};

describe("sample ids", () => {
  it("are the source, the path slug and the sha256's first 12 digits, the same on every build", () => {
    const digest = sha256("x");
    expect(sampleId("vscode", "src/vs/Base/Common.ts", digest)).toBe(`vscode-src-vs-base-common.ts-${digest.slice(0, 12)}`);
    expect(sampleId("vscode", "src/vs/Base/Common.ts", digest)).toBe(sampleId("vscode", "src/vs/Base/Common.ts", digest));
  });

  it("tell an original from its fixed copy, and keep a long path's end", () => {
    const digest = sha256("x");
    expect(sampleId("gatsby-fixed", "a.ts", digest)).not.toBe(sampleId("gatsby", "a.ts", digest));
    const long = sampleId("repo", `${"deep/".repeat(40)}leaf-file.ts`, digest);
    expect(long.length).toBeLessThan(110);
    expect(long).toContain("leaf-file.ts");
  });
});

describe("labels", () => {
  it("give every pack rule porting one upstream rule the same key (D4)", () => {
    const rules = conformance([
      { id: "clippy-unreadable-literal", policy: "rules/a.json", upstream: { tool: "clippy", rule: "unreadable_literal" } },
      { id: "clippy-unreadable-literal-no-fractions", policy: "rules/b.json", upstream: { tool: "clippy", rule: "unreadable_literal" } },
      { id: "clippy-other", policy: "rules/c.json", upstream: { tool: "clippy", rule: "other" } },
    ]);
    expect(packRuleIds(rules, "clippy", "unreadable_literal")).toEqual(["clippy-unreadable-literal", "clippy-unreadable-literal-no-fractions"]);
    expect(packRuleIds(rules, "clippy", "other")).toEqual(["clippy-other"]);
    expect(packRuleIds(rules, "eslint", "other")).toEqual([]);
  });

  it("are written as the format's labels file, rule ids and labels sorted", () => {
    const text = labelsText(sample({ expected: new Map([["b", [{ line: 9 }, { line: 2, column: 4 }]], ["a", []]]) }));
    const parsed = sampleLabelsSchema.parse(JSON.parse(text));
    expect(Object.keys(parsed.expected)).toEqual(["a", "b"]);
    expect(parsed.expected.b).toEqual([{ line: 2, column: 4 }, { line: 9 }]);
    expect(text.endsWith("}\n")).toBe(true);
  });
});

describe("deduplication", () => {
  it("merges the same repository, path and sha256 into one sample, the earlier source winning a disagreement", () => {
    const golden = sample({ expected: new Map([["r1", [{ line: 1 }]], ["r2", []]]), context: [{ path: "pkg/__init__.py", bytes: Buffer.from("") }] });
    const corpus = sample({ expected: new Map([["r2", [{ line: 4 }]], ["r3", []]]), origins: ["corpus"], labelledBy: { tool: "ruff", version: "0.16.9", how: "corpus recording" } });
    const other = sample({ path: "pkg/b.py" });
    const { samples, conflicts, merged } = dedupe([golden, corpus, other]);
    expect(samples).toHaveLength(2);
    expect(merged).toBe(1);
    const one = samples.find((entry) => entry.source.path === "pkg/a.py")!;
    expect(Object.fromEntries(one.expected)).toEqual({ r1: [{ line: 1 }], r2: [], r3: [] });
    expect(one.labelledBy.how).toBe("port-readiness recording");
    expect(one.context.map((file) => file.path)).toEqual(["pkg/__init__.py"]);
    expect(conflicts).toEqual([{ key: "owner/demo\0pkg/a.py\0" + golden.source.sha256, rule: "r2", kept: "golden", dropped: "corpus" }]);
  });

  it("keeps the same path with other bytes apart", () => {
    expect(dedupe([sample(), sample({ text: "y = 2\n" })]).samples).toHaveLength(2);
  });
});

describe("licences", () => {
  it("exclude the copyleft sources, whatever the case of their name", () => {
    expect(copyleftReason("Automattic/wp-calypso")).toMatch(/GPL-2\.0/u);
    expect(copyleftReason("mozilla/application-services")).toMatch(/MPL-2\.0/u);
    expect(copyleftReason("django/django")).toBeUndefined();
  });

  it("are read from the repository's licence file and checked against the recorded licence", () => {
    const cache = scratch();
    const write = (repo: string, file: string, text: string) => {
      const path = join(cache, "licences", repo, COMMIT, file);
      mkdirSync(join(path, ".."), { recursive: true });
      writeFileSync(path, text);
    };
    write("tokio-rs/tokio", "LICENSE", "MIT License\n\nPermission is hereby granted, free of charge, to any person");
    expect(licenceTexts(cache, "tokio-rs/tokio", COMMIT).spdx).toBe("MIT");
    write("django/django", "LICENSE", "GNU GENERAL PUBLIC LICENSE\nVersion 2");
    expect(() => licenceTexts(cache, "django/django", COMMIT)).toThrow(/copyleft/u);
    write("dask/dask", "LICENSE.txt", "Permission is hereby granted, free of charge");
    expect(() => licenceTexts(cache, "dask/dask", COMMIT)).toThrow(/do not read as BSD-3-Clause/u);
    expect(() => licenceTexts(cache, "someone/unknown", COMMIT)).toThrow(/no licence is recorded/u);
  });

  it("are read at each commit from the files the table records for it, a package.json's `license` standing in for none", () => {
    const cache = scratch();
    const write = (repo: string, commit: string, file: string, text: string) => {
      const path = join(cache, "licences", repo, commit, file);
      mkdirSync(join(path, ".."), { recursive: true });
      writeFileSync(path, text);
    };
    const bsdText = "BSD License\n\nRedistribution and use in source and binary forms ... Neither the name Facebook";
    // jest: BSD-3-Clause with a patent grant at 242fb35 (LICENSE and PATENTS), MIT later (LICENSE alone).
    const bsd = "242fb35459b387e360f52394b0e30f42101cc51d";
    write("jestjs/jest", bsd, "LICENSE", bsdText);
    write("jestjs/jest", bsd, "PATENTS", "Additional Grant of Patent Rights Version 2");
    expect(licenceTexts(cache, "jestjs/jest", bsd)).toMatchObject({ spdx: "BSD-3-Clause", texts: [{ file: "LICENSE" }, { file: "PATENTS" }] });
    write("jestjs/jest", COMMIT, "LICENSE", bsdText);
    expect(() => licenceTexts(cache, "jestjs/jest", COMMIT)).toThrow(/LICENSE do not read as MIT/u);
    // sails at c39c5e3 has no licence file; its package.json declares MIT.
    const sails = "c39c5e38970058c7ddd6b43e04ba795856c5ab07";
    write("balderdashy/sails", sails, "package.json", JSON.stringify({ name: "waterline", license: "MIT" }));
    expect(licenceTexts(cache, "balderdashy/sails", sails).spdx).toBe("MIT");
    write("balderdashy/sails", sails, "package.json", JSON.stringify({ name: "waterline", license: "ISC" }));
    expect(() => licenceTexts(cache, "balderdashy/sails", sails)).toThrow(/package\.json do not read as MIT/u);
    // sequelize in September 2010: nothing states a licence, which the table records as NONE and reads nothing for.
    expect(licenceTexts(cache, "sequelize/sequelize", "155381bd5ddfc8d4b7a6f1cd8385bfca60921ada")).toEqual({ spdx: "NONE", texts: [] });
  });

  it("are claimed in SAMPLES.md and LICENSES.md as texts only for the repositories whose texts the archive holds", () => {
    const samples = [sample({ source: { ...sample().source, repo: "a/x" } }), sample({ path: "b.py", source: { ...sample().source, repo: "b/y", path: "b.py" } })];
    const lock = { sha256: "f".repeat(64), files: 4, bytes: 10 };
    const text = [{ file: "LICENSE", text: "MIT License" }];
    const claims = (texts: (repo: string) => typeof text | undefined) => {
      const samplesMd = samplesMarkdown("p/t", lock, samples, texts);
      return { samplesMd, intro: licensesMarkdown(samples, texts).split("\n")[2]! };
    };

    const all = claims(() => text);
    expect(all.samplesMd).toContain("The archive's `LICENSES.md` lists every file with its repository's licence text.");
    expect(all.intro).toContain("and its licence text as read at those commits.");

    const none = claims(() => undefined);
    expect(none.samplesMd).toContain("It holds no licence texts.");
    expect(none.samplesMd).not.toContain("with its repository's licence text");
    expect(none.intro).toContain("where one was read (otherwise the licence the ESLint test set recorded)");

    const some = claims((repo) => (repo === "a/x" ? text : []));
    expect(some.samplesMd).toContain("It holds the licence text of a/x only; for every other repository it holds the licence id the ESLint test set recorded");
    expect(some.samplesMd).not.toContain("with its repository's licence text");
  });
});

describe("archives", () => {
  it("are canonical: the same samples in any order make the same tar", () => {
    const samples = [
      sample({ expected: new Map([["r1", [{ line: 1 }]]]), context: [{ path: "pkg/__init__.py", bytes: Buffer.from("") }] }),
      sample({ path: "pkg/b.py", text: "b = 1\n", expected: new Map([["r1", []]]) }),
    ];
    const first = buildArchive(archiveEntries(samples, "# Sources\n"));
    const second = buildArchive(archiveEntries([...samples].reverse(), "# Sources\n"));
    expect(second.sha256).toBe(first.sha256);
    const paths = readArchive(first.tar).map((entry: { path: string }) => entry.path);
    const id = sampleId("demo", "pkg/a.py", samples[0]!.source.sha256);
    expect(paths).toEqual(expect.arrayContaining(["LICENSES.md", `${id}/pkg/a.py`, `${id}/pkg/a.py${LABELS_SUFFIX}`, `${id}/pkg/__init__.py`]));
    expect(paths).toHaveLength(1 + 2 + 1 + 2);
  });

  it("refuse two samples with one id", () => {
    const one = sample();
    expect(() => archiveEntries([one, { ...one, origins: ["corpus"] }], "")).toThrow(/same id/u);
  });
});

describe("the ESLint test set", () => {
  it("puts a fixed copy at its original's path, or at the renamed path where the fix renamed the file", () => {
    expect(fixedCopyPath("lib/a.js", "r/repo-a-0123456789ab.js", "r/repo-a-0123456789ab-fixed.js")).toBe("lib/a.js");
    expect(fixedCopyPath("lib/helpers/getDiscriminatorValueFromFilter.js", "unicorn-filename-case/mongoose-getDiscriminatorValueFromFilter-54e47adf2259.js", "unicorn-filename-case/mongoose-get-discriminator-value-from-filter-54e47adf2259-fixed.js"))
      .toBe("lib/helpers/get-discriminator-value-from-filter.js");
  });

  it("labels each original with its findings and each fixed copy clean, under the tool-pack porting the rule", () => {
    const directory = scratch();
    const write = (path: string, text: string) => {
      mkdirSync(join(directory, path, ".."), { recursive: true });
      writeFileSync(join(directory, path), text);
    };
    write("manifest.json", JSON.stringify({ tools: { eslint: "10.11.0" }, entries: [{ file: "eslint-no-x/repo-a-0123456789ab.js", fixed: "eslint-no-x/repo-a-0123456789ab-fixed.js", rule: "no-x", rerailRule: "eslint-no-x", pack: "javascript" }] }));
    write("eslint-no-x/repo-a-0123456789ab.js", "x();\n");
    write("eslint-no-x/repo-a-0123456789ab-fixed.js", "y();\n");
    write("eslint-no-x/repo-a-0123456789ab.js.eslint.json", JSON.stringify({ rule: "no-x", rerailRule: "eslint-no-x", source: { repo: "owner/repo", sha: COMMIT, path: "src/a.js", license: "MIT" }, eslint: { findings: [{ line: 1, column: 1 }] } }));
    write("eslint-no-x/repo-a-0123456789ab-fixed.js.eslint.json", JSON.stringify({ fixedOf: "eslint-no-x/repo-a-0123456789ab.js", rule: "no-x", eslint: { findings: [] } }));
    const rules = new Map([["javascript", { ...conformance([{ id: "eslint-no-x", policy: "rules/eslint-no-x.json", upstream: { tool: "eslint", rule: "no-x" } }]), pack: "javascript" }]]);
    const { samples, originals, fixed } = eslintTestSetSamples(directory, rules);
    expect([...originals]).toEqual([["javascript/eslint", 1]]);
    expect([...fixed]).toEqual([["javascript/eslint", 1]]);
    expect(samples.map((entry) => [entry.short, entry.source.path, Object.fromEntries(entry.expected), entry.pinned])).toEqual([
      ["repo", "src/a.js", { "eslint-no-x": [{ line: 1, column: 1 }] }, true],
      ["repo-fixed", "src/a.js", { "eslint-no-x": [] }, true],
    ]);
    expect(samples[0]!.labelledBy).toEqual({ tool: "eslint", version: "10.11.0", how: "eslint test set" });
  });
});
