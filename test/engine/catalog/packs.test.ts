/**
 * The pack validation contract (docs/pack-format.md): every packs/<id>/ directory passes checks 1 to 8, one test per
 * check, and reports each rule's status; a rule core cannot read yet is skipped as not supported yet. Check 4 scores
 * ported rules on the tools' fixtures.
 *
 * The validator's own tests run it over the fixture packs in fixtures/packs/: `valid` passes, each negative
 * breaks one check of a copy of it, `exemplar`, for the test-only planned language, passes with its rule not supported
 * yet, and `empty`, whose ports are all candidates, passes with no rules.
 */
import { appendFileSync, cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, test, vi } from "vitest";
import { z } from "zod";
import { PACK_CHECKS, packedFileFailures, textFailures, validatePack, type PackCheckName, type PackValidation } from "../../../conformance/pack-validation.js";
import { compareDeterministicStrings } from "../../../src/engine/stable-json.js";
import { fixturesPath } from "../../support/oracles.js";

// The exemplar is a language only tests plan: the registry module lists it for this file, and the engine refuses it as a
// planned language (test/engine/support/exemplar-registry.ts, exemplar-engine.ts).
vi.mock("../../../src/engine/code-rule/code-rule-language-registry.generated.js", async () =>
  (await import("../support/exemplar-registry.js")).exemplarRegistryModule());
vi.mock("../../../src/engine/native/native.js", async importOriginal =>
  (await import("../support/exemplar-engine.js")).nativeRefusingExemplar(await importOriginal()));

const PACKS = fileURLToPath(new URL("../../../packs/", import.meta.url));
const FIXTURES = fixturesPath("packs");
const TIMEOUT = 600_000;

const packs = existsSync(PACKS)
  ? readdirSync(PACKS, { withFileTypes: true })
      .filter(entry => entry.isDirectory())
      .map(entry => entry.name)
      .sort(compareDeterministicStrings)
  : [];

/** The rule ids pack.json lists, read when tests are collected; an unreadable pack.json lists none and fails check 1. */
function listedRules(id: string): string[] {
  try {
    const parsed = z.object({ rules: z.array(z.object({ id: z.string() })) }).safeParse(JSON.parse(readFileSync(join(PACKS, id, "pack.json"), "utf8")));
    return parsed.success ? parsed.data.rules.map(rule => rule.id) : [];
  } catch {
    return [];
  }
}

describe.each(packs)("pack %s", id => {
  let validation: Promise<PackValidation> | undefined;
  const validated = (): Promise<PackValidation> => (validation ??= validatePack(join(PACKS, id)));

  it.each(PACK_CHECKS)("check $number: $name", { timeout: TIMEOUT }, async ({ name }) => {
    expect((await validated()).checks[name]).toEqual([]);
  });

  test.for(listedRules(id))("rule %s", { timeout: TIMEOUT }, async (ruleId, { skip }) => {
    const rule = (await validated()).rules.find(entry => entry.id === ruleId);
    if (rule?.status === "not-supported-yet") skip(`not supported yet: ${rule.reason}`);
    expect(rule?.status, rule?.reason).toBe("ok");
  });
});

/** Every rule document in `packs/<id>/`, shipped or candidate (any `rules/` directory outside `upstream/`), by pack-relative path. */
function ruleDocuments(id: string): string[] {
  const directory = join(PACKS, id);
  return readdirSync(directory, { recursive: true, encoding: "utf8" })
    .map(file => file.split(sep).join("/"))
    .filter(file => file.endsWith(".json") && !file.startsWith("upstream/") && file.split("/").at(-2) === "rules")
    .sort(compareDeterministicStrings);
}

describe("rule ids across packs", () => {
  it("gives every rule, shipped or candidate, an id no other document in any pack has", () => {
    const owners = new Map<string, string[]>();
    for (const id of packs) {
      for (const file of ruleDocuments(id)) {
        const ruleId = z.object({ id: z.string() }).parse(JSON.parse(readFileSync(join(PACKS, id, file), "utf8"))).id;
        owners.set(ruleId, [...(owners.get(ruleId) ?? []), `packs/${id}/${file}`]);
      }
    }
    expect([...owners].filter(([, files]) => files.length > 1)).toEqual([]);
  });
});

describe("validatePack", () => {
  const temporary: string[] = [];
  afterEach(() => {
    for (const directory of temporary.splice(0)) rmSync(directory, { recursive: true, force: true });
  });

  /** A copy of fixture pack `name`, in a directory of that name, with `change` applied to it. */
  function changed(name: string, change: (directory: string) => void): string {
    const parent = mkdtempSync(join(tmpdir(), "rerail-packs-"));
    temporary.push(parent);
    const directory = join(parent, name);
    cpSync(join(FIXTURES, name), directory, { recursive: true });
    change(directory);
    return directory;
  }

  const writeJson = (path: string, value: unknown): void => writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
  const editJson = (path: string, edit: (value: Record<string, unknown>) => unknown): void =>
    writeJson(path, edit(z.record(z.string(), z.unknown()).parse(JSON.parse(readFileSync(path, "utf8")))));
  /** The fixture pack's licenses/examplelint/UPSTREAM.md, rewritten by `edit`. */
  const editUpstream = (directory: string, edit: (text: string) => string): void => {
    const path = join(directory, "licenses", "examplelint", "UPSTREAM.md");
    writeFileSync(path, edit(readFileSync(path, "utf8")));
  };

  /** pack.json with the `pass-statement` rule's listing replaced by `edit`'s result. */
  const editPassStatement = (directory: string, edit: (rule: Record<string, unknown>) => Record<string, unknown>): void =>
    editJson(join(directory, "pack.json"), pack => ({
      ...pack,
      rules: z
        .array(z.record(z.string(), z.unknown()))
        .parse(pack.rules)
        .map(rule => (rule.id === "pass-statement" ? edit(rule) : rule)),
    }));

  /**
   * A probe manifest, `candidates/probes/conformance.json`, as the root one with its one port given `id` and `policy`, and
   * that port's document, `candidates/probes/rules/probe-assert-used.json` (id `probe-assert-used`).
   */
  const writeProbes = (directory: string, port: { id: string; policy: string }): void => {
    const probes = join(directory, "candidates", "probes");
    mkdirSync(join(probes, "rules"), { recursive: true });
    writeJson(join(probes, "rules", "probe-assert-used.json"), {
      ...z.record(z.string(), z.unknown()).parse(JSON.parse(readFileSync(join(directory, "rules", "assert-used.json"), "utf8"))),
      id: "probe-assert-used",
    });
    const root = z.looseObject({ rules: z.array(z.record(z.string(), z.unknown())) }).parse(JSON.parse(readFileSync(join(directory, "conformance.json"), "utf8")));
    writeJson(join(probes, "conformance.json"), { ...root, pack: "valid-probes", rules: root.rules.map(rule => ({ ...rule, ...port })) });
  };

  const failing = (validation: PackValidation): Partial<Record<PackCheckName, string[]>> =>
    Object.fromEntries(Object.entries(validation.checks).filter(([, failures]) => failures.length > 0));

  /** The `assert-used` port's entry in conformance.json, rewritten by `edit`. */
  const editAssertUsedPort = (directory: string, edit: (port: Record<string, unknown>) => Record<string, unknown>): void =>
    editJson(join(directory, "conformance.json"), conformance => ({
      ...conformance,
      rules: z
        .array(z.record(z.string(), z.unknown()))
        .parse(conformance.rules)
        .map(port => (port.id === "assert-used" ? edit(port) : port)),
    }));

  /** `assert-used` back to a candidate (`candidates/rules/assert-used.json`, out of pack.json and the README), its port carrying `examples`. */
  const assertUsedCandidate =
    (examples: unknown) =>
    (directory: string): void => {
      mkdirSync(join(directory, "candidates", "rules"), { recursive: true });
      renameSync(join(directory, "rules", "assert-used.json"), join(directory, "candidates", "rules", "assert-used.json"));
      editJson(join(directory, "pack.json"), pack => ({ ...pack, rules: z.array(z.looseObject({ id: z.string() })).parse(pack.rules).filter(rule => rule.id !== "assert-used") }));
      const readme = join(directory, "README.md");
      writeFileSync(readme, readFileSync(readme, "utf8").replace(/^### `assert-used`\n[\s\S]*$/mu, ""));
      editAssertUsedPort(directory, port => ({ ...port, policy: "candidates/rules/assert-used.json", examples }));
    };
  const pair = (caught: string, passes: string): unknown => ({ caught: { path: "src/a.py", text: caught }, passes: { path: "src/a.py", text: passes } });

  it("passes a candidate's example pair in conformance.json, its caught example firing and its passes example silent", async () => {
    const validation = await validatePack(changed("valid", assertUsedCandidate(pair("assert x\n", "x = 1\n"))));
    expect(failing(validation)).toEqual({});
    expect(validation.rules).toEqual([{ id: "pass-statement", document: "rules/pass-statement.json", status: "ok" }]);
  });

  it("fails a shipped port that carries examples: they belong in pack.json", async () => {
    const validation = await validatePack(changed("valid", directory => editAssertUsedPort(directory, port => ({ ...port, examples: pair("assert x\n", "x = 1\n") }))));
    // conformance.json fails its schema, so the rule has no port entry either.
    expect(failing(validation)).toEqual({
      ports: ["conformance.json: conformance.json: ✖ assert-used: ships, so its examples belong in pack.json"],
      proof: ["assert-used: no examples and no port entry in conformance.json"],
    });
  });

  it("passes a valid pack, every rule ok", async () => {
    const validation = await validatePack(join(FIXTURES, "valid"));
    expect(failing(validation)).toEqual({});
    expect(validation.pass).toBe(true);
    expect(validation.rules).toEqual([
      { id: "assert-used", document: "rules/assert-used.json", status: "ok" },
      { id: "pass-statement", document: "rules/pass-statement.json", status: "ok" },
    ]);
  });

  it("passes a pack for a planned language, its rules not supported yet and their examples skipped", async () => {
    const validation = await validatePack(join(FIXTURES, "exemplar"));
    expect(failing(validation)).toEqual({});
    expect(validation.rules).toEqual([
      { id: "no-identifier", document: "rules/no-identifier.json", status: "not-supported-yet", reason: "/clauses/0/forbid/ast/language: `exemplar` is not supported yet" },
    ]);
  });

  it("passes a pack that ships no rules while its ports are candidates", async () => {
    const validation = await validatePack(join(FIXTURES, "empty"));
    expect(failing(validation)).toEqual({});
    expect(validation.pass).toBe(true);
    expect(validation.rules).toEqual([]);
  });

  const cases: [string, (directory: string) => void, Partial<Record<PackCheckName, string[]>>][] = [
    [
      "a rule file pack.json does not list",
      directory => writeFileSync(join(directory, "rules", "extra.json"), readFileSync(join(directory, "rules", "pass-statement.json"))),
      { rules: ["rules/extra.json is not listed in pack.json"] },
    ],
    [
      "a missing document",
      directory => rmSync(join(directory, "rules", "pass-statement.json")),
      { rules: ["pass-statement: rules/pass-statement.json does not exist"] },
    ],
    [
      "a document whose id differs from the listing",
      directory => editJson(join(directory, "rules", "pass-statement.json"), document => ({ ...document, id: "pass" })),
      { rules: ["pass-statement: rules/pass-statement.json has id pass"] },
    ],
    [
      "a rule id that is not lowercase letters, digits and dashes",
      directory => {
        editPassStatement(directory, rule => ({ ...rule, id: "valid:pass-statement" }));
        editJson(join(directory, "rules", "pass-statement.json"), document => ({ ...document, id: "valid:pass-statement" }));
      },
      {
        rules: ["pack.json: ✖ must be lowercase letters, digits and dashes\n  → at rules[1].id"],
        proof: ["pack.json does not parse; see check 1"],
        examples: ["pack.json does not parse; see check 1"],
      },
    ],
    [
      "a document not named for its id",
      directory => {
        renameSync(join(directory, "rules", "pass-statement.json"), join(directory, "rules", "pass.json"));
        editPassStatement(directory, rule => ({ ...rule, document: "rules/pass.json" }));
      },
      { rules: ["pass-statement: its document is rules/pass.json; name it rules/pass-statement.json"] },
    ],
    [
      "a candidate not named for its id",
      directory => {
        mkdirSync(join(directory, "candidates", "rules"), { recursive: true });
        writeFileSync(join(directory, "candidates", "rules", "pass.json"), readFileSync(join(directory, "rules", "pass-statement.json")));
      },
      { rules: ["candidates/rules/pass.json: has id pass-statement; name it candidates/rules/pass-statement.json"] },
    ],
    [
      "a candidate whose id is not lowercase letters, digits and dashes, though its file is named for it",
      directory => {
        mkdirSync(join(directory, "candidates", "rules"), { recursive: true });
        writeJson(join(directory, "candidates", "rules", "eslint:no-alert.json"), { format: "rerail.policy/v1", id: "eslint:no-alert", description: "Avoid alert().", clauses: [] });
      },
      { rules: ["candidates/rules/eslint:no-alert.json: id eslint:no-alert is not lowercase letters, digits and dashes"] },
    ],
    [
      "a candidate with no id",
      directory => {
        mkdirSync(join(directory, "candidates", "rules"), { recursive: true });
        writeJson(join(directory, "candidates", "rules", "no-pass.json"), { format: "rerail.policy/v1", description: "Remove pass statements.", clauses: [] });
      },
      { rules: ["candidates/rules/no-pass.json: has no id"] },
    ],
    [
      "a candidate in an older document format",
      directory => {
        mkdirSync(join(directory, "candidates", "rules"), { recursive: true });
        writeJson(join(directory, "candidates", "rules", "no-pass.json"), { format: "tempo.policy/v1", id: "no-pass", description: "Remove pass statements.", checks: [] });
      },
      { rules: ["candidates/rules/no-pass.json: format is tempo.policy/v1; a rule document is rerail.policy/v1"] },
    ],
    [
      "a candidate outside a rules/ directory",
      directory => {
        mkdirSync(join(directory, "candidates"));
        writeFileSync(join(directory, "candidates", "pass-statement.json"), readFileSync(join(directory, "rules", "pass-statement.json")));
      },
      { rules: ["candidates/pass-statement.json: a candidate is in a rules/ directory; move it to candidates/rules/pass-statement.json"] },
    ],
    [
      "a probe port whose id is not its document's",
      directory => writeProbes(directory, { id: "probe-assert", policy: "rules/probe-assert-used.json" }),
      { rules: ["candidates/probes/conformance.json: port probe-assert scores candidates/probes/rules/probe-assert-used.json, whose id is probe-assert-used"] },
    ],
    [
      "a probe port whose document does not exist",
      directory => writeProbes(directory, { id: "probe-gone", policy: "rules/probe-gone.json" }),
      { rules: ["candidates/probes/conformance.json: port probe-gone scores candidates/probes/rules/probe-gone.json, which does not exist"] },
    ],
    [
      "a probe port whose id is not lowercase letters, digits and dashes",
      directory => writeProbes(directory, { id: "probe:assert-used", policy: "rules/probe-assert-used.json" }),
      { rules: [expect.stringMatching(/^candidates\/probes\/conformance\.json: ✖ rule ids are lowercase kebab-case\n {2}→ at rules\[0\]\.id$/u)] },
    ],
    [
      "a probe manifest that is not JSON",
      directory => {
        writeProbes(directory, { id: "probe-assert-used", policy: "rules/probe-assert-used.json" });
        writeFileSync(join(directory, "candidates", "probes", "conformance.json"), "{\n");
      },
      { rules: [expect.stringMatching(/^candidates\/probes\/conformance\.json: is not JSON: /u)] },
    ],
    [
      "a port whose id is not its document's",
      directory => {
        for (const file of ["conformance.json", "divergences.json"]) {
          writeFileSync(join(directory, file), readFileSync(join(directory, file), "utf8").replaceAll('"assert-used"', '"examplelint-assert-used"'));
        }
      },
      { rules: ["conformance.json: port examplelint-assert-used scores rules/assert-used.json, whose id is assert-used"] },
    ],
    [
      "a rule with neither examples nor a port entry",
      directory => editPassStatement(directory, ({ examples: _examples, ...rule }) => rule),
      { proof: ["pass-statement: no examples and no port entry in conformance.json"] },
    ],
    [
      "a caught example that does not fire",
      directory => editPassStatement(directory, rule => ({ ...rule, examples: { caught: { path: "src/a.py", text: "x = 1\n" }, passes: { path: "src/a.py", text: "x = 2\n" } } })),
      { examples: ["pass-statement: the caught example (src/a.py) produces no PASS violation"] },
    ],
    [
      "a passes example that fires",
      directory => editPassStatement(directory, rule => ({ ...rule, examples: { caught: { path: "src/a.py", text: "pass\n" }, passes: { path: "src/a.py", text: "pass\n" } } })),
      { examples: ["pass-statement: the passes example (src/a.py) produces PASS"] },
    ],
    [
      "a candidate's caught example that does not fire",
      assertUsedCandidate(pair("x = 1\n", "x = 2\n")),
      { examples: ["assert-used: the caught example (src/a.py) produces no EX101 violation"] },
    ],
    [
      "a candidate's passes example that fires",
      assertUsedCandidate(pair("assert x\n", "assert y\n")),
      { examples: ["assert-used: the passes example (src/a.py) produces EX101"] },
    ],
    [
      "a ported rule's violation the recording lacks, outside divergences.json",
      directory => writeFileSync(join(directory, "divergences.json"), "[]\n"),
      { ports: ['assert-used (fixtures): 1 unexplained FP, first {"path":"asserts.py","line":5,"column":5}'] },
    ],
    [
      "a stale divergence",
      directory => {
        const path = join(directory, "divergences.json");
        writeJson(path, [
          ...z.array(z.unknown()).parse(JSON.parse(readFileSync(path, "utf8"))),
          { rule: "assert-used", source: "fixtures", path: "asserts.py", line: 3, kind: "upstream-wrong", evidence: "documented when Rerail missed this assert" },
        ]);
      },
      { ports: ["assert-used: stale divergence asserts.py:3 (upstream-wrong) explains nothing"] },
    ],
    [
      "a declared capability with neither a ported rule nor an exemption",
      directory => editJson(join(directory, "conformance.json"), pack => ({ ...pack, capabilities: ["syntax", "scope"] })),
      { ports: ["capability scope: no ported rule exercises it and it has no exemption"] },
    ],
    [
      "dependencies in package.json",
      directory => editJson(join(directory, "package.json"), manifest => ({ ...manifest, dependencies: { "left-pad": "1.3.0" } })),
      { package: ["package.json: has dependencies (left-pad); a pack has none"] },
    ],
    [
      "a package.json with no repository",
      directory => editJson(join(directory, "package.json"), ({ repository: _repository, ...manifest }) => manifest),
      { package: ['package.json: repository is missing, not {"type":"git","url":"git+https://github.com/neralabs/rerail.git","directory":"packs/valid"}'] },
    ],
    [
      "a repository directory that is not packs/<id>",
      directory => editJson(join(directory, "package.json"), manifest => ({ ...manifest, repository: { type: "git", url: "git+https://github.com/neralabs/rerail.git", directory: "packs/other" } })),
      {
        package: [
          'package.json: repository is {"type":"git","url":"git+https://github.com/neralabs/rerail.git","directory":"packs/other"}, not {"type":"git","url":"git+https://github.com/neralabs/rerail.git","directory":"packs/valid"}',
        ],
      },
    ],
    [
      "a restricted package",
      directory => editJson(join(directory, "package.json"), manifest => ({ ...manifest, publishConfig: { access: "restricted" } })),
      { package: ["package.json: publishConfig.access is restricted; packs are public"] },
    ],
    [
      "a package.json with no publishConfig",
      directory => editJson(join(directory, "package.json"), ({ publishConfig: _publishConfig, ...manifest }) => manifest),
      { package: ["package.json: publishConfig.access is missing; packs are public"] },
    ],
    [
      "upstream/ in package.json files",
      directory => editJson(join(directory, "package.json"), manifest => ({ ...manifest, files: [...z.array(z.string()).parse(manifest.files), "upstream"] })),
      { package: ["package.json: files entry upstream publishes upstream, which is validation material"] },
    ],
    [
      "a licence that disagrees with licenses/",
      directory => editJson(join(directory, "package.json"), manifest => ({ ...manifest, license: "Apache-2.0" })),
      { package: ['package.json: license "Apache-2.0" names Apache-2.0; the pack carries Apache-2.0, MIT (Apache-2.0 and what licenses/*/UPSTREAM.md records)'] },
    ],
    [
      "README rule details for a rule pack.json does not list",
      directory => appendFileSync(join(directory, "README.md"), "\n### `assert-false`\n\nA candidate's details.\n"),
      { package: ["README.md: Rule details describes assert-false, which pack.json does not list; a candidate's details go in candidates/README.md"] },
    ],
    [
      "a README rule details heading that names no rule",
      directory => appendFileSync(join(directory, "README.md"), "\n### Shared notes\n\nDetails for no one rule.\n"),
      { package: ['README.md: the Rule details heading "Shared notes" names no rule'] },
    ],
    [
      "candidates/README.md rule details for a rule pack.json lists",
      directory => {
        mkdirSync(join(directory, "candidates"));
        writeFileSync(join(directory, "candidates", "README.md"), "# valid, candidates\n\n## Rule details\n\n### `pass-statement`\n\nA shipped rule's details.\n");
      },
      { package: ["candidates/README.md: Rule details describes pass-statement, which pack.json lists; a shipped rule's details go in README.md"] },
    ],
    [
      "a copied file once its glob is removed",
      directory => editUpstream(directory, text => text.replace(/^\| `upstream\/examplelint\/fixtures\/\*\*`.*\n/mu, "")),
      { licences: ["upstream/examplelint/fixtures/asserts.py is under upstream/<tool>/ but no licenses/*/UPSTREAM.md pattern covers it (copied) and its tool's UPSTREAM.md does not list it (Rerail's)"] },
    ],
    [
      "a glob that matches no file",
      directory => editUpstream(directory, text => `${text}| \`upstream/examplelint/tests/**\` | \`tests/\` | verbatim |\n`),
      { licences: ["licenses/examplelint/UPSTREAM.md: `upstream/examplelint/tests/**` matches no copied file"] },
    ],
    [
      // As ruff's licenses/pylint/UPSTREAM.md refuses its fixtures/pylint/: the pack mirrors upstream's layout.
      "a file under a not-copied glob",
      directory => {
        editUpstream(directory, text => `${text}\n- Not copied: \`tests/pylint/\` as \`upstream/examplelint/fixtures/pylint/**\` — a copyleft project, GPL-2.0-or-later\n`);
        mkdirSync(join(directory, "upstream", "examplelint", "fixtures", "pylint"));
        writeFileSync(join(directory, "upstream", "examplelint", "fixtures", "pylint", "a.py"), "assert e\n");
      },
      { licences: ["upstream/examplelint/fixtures/pylint/a.py matches `upstream/examplelint/fixtures/pylint/**`, which licenses/examplelint/UPSTREAM.md lists as not copied (tests/pylint/)"] },
    ],
    [
      "a not-copied line with no pack-path glob",
      directory => editUpstream(directory, text => `${text}\n- Not copied: \`tests/pylint/\` — a copyleft project, GPL-2.0-or-later\n`),
      { licences: ['licenses/examplelint/UPSTREAM.md: "Not copied: `tests/pylint/`" names no pack-path glob (`… as `<glob>``)'] },
    ],
    [
      "a not-copied line spelled otherwise",
      directory => editUpstream(directory, text => `${text}\n* Not Copied: \`tests/pylint/\` as \`upstream/examplelint/fixtures/pylint/**\` — a copyleft project, GPL-2.0-or-later\n`),
      { licences: ['licenses/examplelint/UPSTREAM.md: "* Not Copied: `tests/pylint/` as `upstream/examplelint/fixtures/pylint/**` — a copyleft project, GPL-2.0-or-later" is not a `- Not copied: `<prefix>` as `<glob>`` line'] },
    ],
    [
      // clean.py is the tool's authored file, so only the copied asserts.py is judged.
      "equal-length longest globs that disagree on copy kind",
      directory => editUpstream(directory, text => `${text}| \`upstream/examplelint/f*xtures/**\` | \`tests/*.py\` | adapted |\n`),
      {
        licences: [
          "upstream/examplelint/fixtures/asserts.py is covered by equal-length globs that disagree: licenses/examplelint/UPSTREAM.md: `upstream/examplelint/fixtures/**`, MIT, verbatim; licenses/examplelint/UPSTREAM.md: `upstream/examplelint/f*xtures/**`, MIT, adapted",
        ],
      },
    ],
    [
      "a glob with more than two **",
      directory => editUpstream(directory, text => `${text}| \`upstream/**/examplelint/**/fixtures/**\` | \`tests/*.py\` | verbatim |\n`),
      { licences: ["licenses/examplelint/UPSTREAM.md: glob `upstream/**/examplelint/**/fixtures/**` holds 3 `**`; a glob may hold at most 2"] },
    ],
    [
      "an UPSTREAM.md with no Source row",
      directory => editUpstream(directory, text => text.replace(/^\| https:.*\n/mu, "")),
      { licences: ["licenses/examplelint/UPSTREAM.md: no Source row (| <repository URL> | <tag> | <commit> | <licence> |)"] },
    ],
    [
      "an UPSTREAM.md whose Source row is truncated",
      directory => editUpstream(directory, text => text.replace(/^\| https:.*\n/mu, "| https://example.com/examplelint | `v1.0.0` |\n")),
      { licences: ["licenses/examplelint/UPSTREAM.md: Source row https://example.com/examplelint lacks a 40-hex commit and a licence (| <repository URL> | <tag> | <commit> | <licence> |)"] },
    ],
    [
      "an UPSTREAM.md whose Source row has an empty tag and a short commit",
      directory => editUpstream(directory, text => text.replace("| `v1.0.0` | `0000000000000000000000000000000000000000` |", "|  | `0000000` |")),
      { licences: ["licenses/examplelint/UPSTREAM.md: Source row https://example.com/examplelint lacks a tag and a 40-hex commit (| <repository URL> | <tag> | <commit> | <licence> |)"] },
    ],
    [
      "an UPSTREAM.md whose only repository-URL row is outside its Source section",
      directory => editUpstream(directory, text => text.replace("## Source", "## Elsewhere")),
      { licences: ["licenses/examplelint/UPSTREAM.md: no Source row (| <repository URL> | <tag> | <commit> | <licence> |)"] },
    ],
    [
      "a License: line that ANDs a licence off the allowlist",
      directory => editUpstream(directory, text => text.replace("License: MIT;", "License: MIT AND GPL-3.0-only;")),
      {
        package: ['package.json: license "Apache-2.0 AND MIT" names Apache-2.0, MIT; the pack carries Apache-2.0, GPL-3.0-only, MIT (Apache-2.0 and what licenses/*/UPSTREAM.md records)'],
        licences: ["licenses/examplelint/UPSTREAM.md: MIT AND GPL-3.0-only is not on the repository licence allowlist"],
      },
    ],
    [
      "a malformed License: expression",
      directory => editUpstream(directory, text => text.replace("License: MIT;", "License: (MIT OR Apache-2.0;")),
      {
        package: ['package.json: license "Apache-2.0 AND MIT" names Apache-2.0, MIT; licenses/examplelint/UPSTREAM.md records (MIT OR Apache-2.0, which is no SPDX expression'],
        licences: ["licenses/examplelint/UPSTREAM.md: (MIT OR Apache-2.0 is no SPDX expression"],
      },
    ],
    [
      "an authored file the tool's UPSTREAM.md does not list, outside every glob",
      directory => {
        writeFileSync(join(directory, "upstream", "examplelint", "UPSTREAM.md"), "# examplelint: files Rerail wrote\n");
        editUpstream(directory, text => text.replace("`upstream/examplelint/fixtures/**`", "`upstream/examplelint/fixtures/asserts.py`"));
      },
      { licences: ["upstream/examplelint/fixtures/clean.py is under upstream/<tool>/ but no licenses/*/UPSTREAM.md pattern covers it (copied) and its tool's UPSTREAM.md does not list it (Rerail's)"] },
    ],
    [
      "a description that names its tool and holds code",
      directory => editJson(join(directory, "rules", "pass-statement.json"), document => ({ ...document, description: "Port of Ruff's `PIE790`." })),
      {
        text: [
          "rules/pass-statement.json: description names an upstream tool: Port of Ruff's `PIE790`.",
          "rules/pass-statement.json: description holds code (backticks): Port of Ruff's `PIE790`.",
        ],
      },
    ],
    [
      "a rule with no description",
      directory => editJson(join(directory, "rules", "pass-statement.json"), ({ description: _description, ...document }) => document),
      { text: ["rules/pass-statement.json: has no description"] },
    ],
  ];

  it.each(cases)("fails %s", async (_name, change, expected) => {
    const validation = await validatePack(changed("valid", change));
    expect(failing(validation)).toEqual(expected);
    expect(validation.pass).toBe(false);
  });

  it("leaves a tool's publish/ to the pulled layout it is laid over: a staged file no pattern covers fails nothing", async () => {
    const validation = await validatePack(
      changed("valid", directory => {
        mkdirSync(join(directory, "upstream", "examplelint", "publish", "fixtures"), { recursive: true });
        writeFileSync(join(directory, "upstream", "examplelint", "publish", "fixtures", "staged.py"), "assert staged\n");
      }),
    );
    expect(failing(validation)).toEqual({});
    expect(validation.pass).toBe(true);
  });

  it.each([
    ["MIT OR Apache-2.0", "Apache-2.0 AND MIT"],
    ["GPL-3.0-only OR MIT", "Apache-2.0 AND MIT"],
    ["(MIT OR GPL-3.0-only) AND Apache-2.0", "Apache-2.0 AND MIT"],
  ])("passes License: %s with the package's %s: it elects an alternative of the OR", async (licence, packageLicence) => {
    const validation = await validatePack(
      changed("valid", directory => {
        editUpstream(directory, text => text.replace("License: MIT;", `License: ${licence};`));
        editJson(join(directory, "package.json"), manifest => ({ ...manifest, license: packageLicence }));
      }),
    );
    expect(failing(validation)).toEqual({});
    expect(validation.pass).toBe(true);
  });

  it("passes a pack with a publish request staged: check 6 judges pack data where a pull lays it, not under publish/", async () => {
    const validation = await validatePack(
      changed("valid", directory => {
        const publish = join(directory, "upstream", "examplelint", "publish");
        mkdirSync(join(publish, "tests"), { recursive: true });
        writeFileSync(join(publish, "tests", "staged.py"), "pass\n");
        mkdirSync(join(publish, "samples", "repo-src-a.py-0123456789ab", "src"), { recursive: true });
        writeFileSync(join(publish, "samples", "repo-src-a.py-0123456789ab", "src", "a.py"), "pass\n");
      }),
    );
    expect(failing(validation)).toEqual({});
    expect(validation.pass).toBe(true);
  });

  it("passes a rule whatever its message says: messages are left to review", async () => {
    const message = `NEVER keep this pass (pass-statement); Rerail flags it.${" Remove it.".repeat(20)}`;
    const validation = await validatePack(
      changed("valid", directory => editJson(join(directory, "rules", "pass-statement.json"), document => ({
        ...document,
        clauses: (document.clauses as { violation: object }[]).map(clause => ({ ...clause, violation: { ...clause.violation, message } })),
      }))),
    );
    expect(failing(validation)).toEqual({});
    expect(validation.pass).toBe(true);
    expect(validation.rules.find(rule => rule.id === "pass-statement")?.status).toBe("ok");
  });

  describe("check 8: text", () => {
    it("reads no sentence end in an abbreviation or quoted text, and no tool name in a quoted value", () => {
      const document = { description: "Use field init shorthand, e.g. Point { x } for Point { x: x }, not 'eslint' style.", clauses: [] };
      expect(textFailures("rules/x.json", JSON.stringify(document))).toEqual([]);
    });
  });

  describe("check 7: manifest", () => {
    it("fails a pack.json without match, as a manifest failure besides check 1's schema failure", async () => {
      const validation = await validatePack(changed("valid", directory => editJson(join(directory, "pack.json"), ({ match: _match, ...pack }) => pack)));
      expect(failing(validation)).toEqual({
        rules: ["pack.json: ✖ Invalid input: expected object, received undefined\n  → at match"],
        proof: ["pack.json does not parse; see check 1"],
        examples: ["pack.json does not parse; see check 1"],
        manifest: ["pack.json has no match: set-up could never recommend this pack"],
      });
    });

    it("fails a pack core's generated pack list does not name, when it must be listed", async () => {
      const validation = await validatePack(join(FIXTURES, "valid"), { listed: true });
      expect(failing(validation)).toEqual({ manifest: ["src/engine/catalog/pack-list.generated.ts does not list valid; run npm run generate:packs"] });
    });
  });

  it("marks the rule a failure belongs to", async () => {
    const validation = await validatePack(
      changed("valid", directory => writeFileSync(join(directory, "divergences.json"), "[]\n")),
    );
    expect(validation.rules.map(({ id, status }) => ({ id, status }))).toEqual([
      { id: "assert-used", status: "failed" },
      { id: "pass-statement", status: "ok" },
    ]);
  });

  it("passes a pack whose ESLint port honours ESLint's suppression comments", async () => {
    const validation = await validatePack(join(FIXTURES, "eslint-port"));
    expect(failing(validation)).toEqual({});
    expect(validation.rules).toEqual([{ id: "no-debugger", document: "rules/no-debugger.json", status: "ok" }]);
  });

  /** The eslint-port pack with its port's `suppress` removed, scored on a fixture with no directive to honour. */
  const withoutSuppress = (directory: string): void => {
    editJson(join(directory, "rules", "no-debugger.json"), document => ({
      ...document,
      clauses: z
        .array(z.looseObject({ violation: z.record(z.string(), z.unknown()) }))
        .parse(document.clauses)
        .map(({ violation: { suppress: _suppress, ...violation }, ...clause }) => ({ ...clause, violation })),
    }));
    writeFileSync(join(directory, "upstream", "eslint", "fixtures", "debugger.js"), "debugger;\n");
  };
  const exempted = (directory: string): void =>
    editJson(join(directory, "conformance.json"), pack => ({ ...pack, suppressionExemptions: [{ rule: "no-debugger", reason: "recorded before the port modelled eslint-disable comments" }] }));

  it("fails an ESLint port whose violation does not suppress its upstream rule", async () => {
    const validation = await validatePack(changed("eslint-port", withoutSuppress));
    expect(failing(validation)).toEqual({
      ports: [
        "no-debugger: rules/no-debugger.json reports eslint's no-debugger without suppress.eslint.rules naming it (clause debugger-statement), so the comments that silence no-debugger are ignored; add it, or exempt the port in suppressionExemptions with a reason",
      ],
    });
    expect(validation.rules.map(({ id, status }) => ({ id, status }))).toEqual([{ id: "no-debugger", status: "failed" }]);
  });

  it("passes a port without suppress that conformance.json exempts with a reason", async () => {
    const validation = await validatePack(
      changed("eslint-port", directory => {
        withoutSuppress(directory);
        exempted(directory);
      }),
    );
    expect(failing(validation)).toEqual({});
  });

  it("fails a stale suppression exemption, for a port that suppresses its upstream rule", async () => {
    const validation = await validatePack(changed("eslint-port", exempted));
    expect(failing(validation)).toEqual({ ports: ["no-debugger: stale suppression exemption: every violation of rules/no-debugger.json names no-debugger in suppress.eslint.rules"] });
  });

  it("fails a port whose violation carries suppress.oxlint without suppress.eslint", async () => {
    const validation = await validatePack(
      changed("eslint-port", directory =>
        editJson(join(directory, "rules", "no-debugger.json"), document => ({
          ...document,
          clauses: z
            .array(z.looseObject({ violation: z.record(z.string(), z.unknown()) }))
            .parse(document.clauses)
            .map(clause => ({ ...clause, violation: { ...clause.violation, suppress: { oxlint: { rules: ["no-debugger"] } } } })),
        })),
      ),
    );
    expect(failing(validation)).toEqual({
      ports: [
        "no-debugger: rules/no-debugger.json reports eslint's no-debugger without suppress.eslint.rules naming it (clause debugger-statement), so the comments that silence no-debugger are ignored; add it, or exempt the port in suppressionExemptions with a reason",
        "no-debugger: rules/no-debugger.json carries suppress.eslint and suppress.oxlint on different violations (clause debugger-statement); Oxlint reads eslint-disable comments too, so both go on the same violations",
      ],
    });
  });

  it("fails a port whose suppress.eslint lacks the Biome rule the same as its ESLint rule, or that carries suppress.biome alone", async () => {
    const suppressing = (suppress: Record<string, unknown>) => (directory: string) =>
      editJson(join(directory, "rules", "no-debugger.json"), document => ({
        ...document,
        clauses: z
          .array(z.looseObject({ violation: z.record(z.string(), z.unknown()) }))
          .parse(document.clauses)
          .map(clause => ({ ...clause, violation: { ...clause.violation, suppress } })),
      }));
    const eslint = { rules: ["no-debugger"] };
    const oxlint = { rules: ["no-debugger"] };
    const missing = {
      ports: [
        "no-debugger: rules/no-debugger.json carries suppress.eslint without suppress.biome naming lint/suspicious/noDebugger (clause debugger-statement), the Biome 2.5.15 rule the same as the ESLint rule it names, so the biome-ignore comments that silence it are ignored; run scripts/generate/packs/biome-suppression.mjs",
      ],
    };
    expect(failing(await validatePack(changed("eslint-port", suppressing({ eslint, oxlint }))))).toEqual(missing);
    const otherRule = suppressing({ eslint, oxlint, biome: { rules: ["lint/suspicious/noConsole"] } });
    expect(failing(await validatePack(changed("eslint-port", otherRule)))).toEqual(missing);
    const alone = await validatePack(changed("eslint-port", suppressing({ biome: { rules: ["lint/suspicious/noDebugger"] } })));
    expect(failing(alone)).toEqual({
      ports: [
        // The fixture's eslint-disable comments no longer silence the port: its recording disagrees.
        'no-debugger (fixtures): 2 unexplained FP, first {"path":"debugger.js","line":2,"column":1}',
        "no-debugger: rules/no-debugger.json reports eslint's no-debugger without suppress.eslint.rules naming it (clause debugger-statement), so the comments that silence no-debugger are ignored; add it, or exempt the port in suppressionExemptions with a reason",
        "no-debugger: rules/no-debugger.json carries suppress.biome without suppress.eslint (clause debugger-statement); a port names the Biome rules the same as the ESLint rule it names",
      ],
    });
  });

  it("asks no suppress.biome of a port whose query language does not honour Biome's comments", async () => {
    // Vue's registry entry honours eslint alone: a Vue query, over the component's script program
    // too, refuses suppress.biome.
    const vue = (directory: string) =>
      editJson(join(directory, "rules", "no-debugger.json"), document => ({
        ...document,
        clauses: z
          .array(z.looseObject({ forbid: z.looseObject({ ast: z.looseObject({}) }), violation: z.record(z.string(), z.unknown()) }))
          .parse(document.clauses)
          .map(clause => ({
            ...clause,
            forbid: { ...clause.forbid, ast: { ...clause.forbid.ast, language: "vue", embedded: "script-block" } },
            violation: { ...clause.violation, suppress: { eslint: { rules: ["no-debugger"] } } },
          })),
      }));
    const validation = await validatePack(changed("eslint-port", vue));
    // The fixtures are .js files, which a Vue query does not read; nothing asks for suppress.biome.
    expect(failing(validation)).toEqual({
      ports: [
        'no-debugger (fixtures): 1 unexplained FN, first {"path":"debugger.js","line":1,"column":1}',
        "no-debugger (fixtures): 0 TP and no examples in pack.json; nothing shows it catches anything",
      ],
    });
  });
});

describe("packedFileFailures", () => {
  const files = ["pack.json", "rules", "licenses", "LICENSE", "NOTICE", "README.md"];

  it("passes package.json and everything files publishes, nested directories included", () => {
    expect(packedFileFailures(files, ["package.json", "pack.json", "rules/a.json", "licenses/tool/UPSTREAM.md", "LICENSE", "NOTICE", "README.md"])).toEqual([]);
  });

  it("fails an entry files does not publish", () => {
    expect(packedFileFailures(files, ["pack.json", "CHANGELOG.md", "docs/notes.md"])).toEqual([
      "CHANGELOG.md: package.json files does not publish it",
      "docs/notes.md: package.json files does not publish it",
    ]);
  });

  it("passes a directory a pack adds to files, such as docs/", () => {
    expect(packedFileFailures([...files, "docs"], ["docs/notes.md"])).toEqual([]);
  });

  it("passes files under a folder a glob entry matches, which npm publishes whole", () => {
    expect(packedFileFailures(["pack.json", "rules/*"], ["pack.json", "rules/a.json", "rules/group/b.json"])).toEqual([]);
  });

  it("fails validation material even when files publishes it", () => {
    expect(packedFileFailures(["*", "upstream", "candidates"], ["conformance.json", "divergences.json", "upstream/tool/fixtures/a.py", "candidates/rules/x.json", "pack.json"])).toEqual([
      "candidates/rules/x.json: validation material, never published",
      "conformance.json: validation material, never published",
      "divergences.json: validation material, never published",
      "upstream/tool/fixtures/a.py: validation material, never published",
    ]);
  });
});
