/**
 * The pack validation contract (docs/pack-format.md): every packs/<id>/ directory passes checks 1 to 6, one test per
 * check, and reports each rule's status; a rule core cannot read yet is skipped as not supported yet. Check 4 scores
 * ported rules on the tools' fixtures, and on the pinned corpora that are checked out (RERAIL_PACK_CORPORA=1: every
 * corpus, a missing checkout failing; 0: none).
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
import { PACK_CHECKS, packedFileFailures, validatePack, type PackCheckName, type PackValidation, type ValidatePackOptions } from "../../../conformance/pack-validation.js";
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

const CORPORA: ValidatePackOptions = process.env.RERAIL_PACK_CORPORA === "1" ? { corpora: true } : process.env.RERAIL_PACK_CORPORA === "0" ? { corpora: false } : {};

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
  const validated = (): Promise<PackValidation> => (validation ??= validatePack(join(PACKS, id), CORPORA));

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

  it("passes a valid pack, every rule ok", async () => {
    const validation = await validatePack(join(FIXTURES, "valid"), { corpora: false });
    expect(failing(validation)).toEqual({});
    expect(validation.pass).toBe(true);
    expect(validation.rules).toEqual([
      { id: "assert-used", document: "rules/assert-used.json", status: "ok" },
      { id: "pass-statement", document: "rules/pass-statement.json", status: "ok" },
    ]);
  });

  it("passes a pack for a planned language, its rules not supported yet and their examples skipped", async () => {
    const validation = await validatePack(join(FIXTURES, "exemplar"), { corpora: false });
    expect(failing(validation)).toEqual({});
    expect(validation.rules).toEqual([
      { id: "no-identifier", document: "rules/no-identifier.json", status: "not-supported-yet", reason: "/clauses/0/forbid/ast/language: `exemplar` is not supported yet" },
    ]);
  });

  it("passes a pack that ships no rules while its ports are candidates", async () => {
    const validation = await validatePack(join(FIXTURES, "empty"), { corpora: false });
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
        writeJson(join(directory, "candidates", "rules", "eslint:no-alert.json"), { format: "rerail.policy/v1", id: "eslint:no-alert", clauses: [] });
      },
      { rules: ["candidates/rules/eslint:no-alert.json: id eslint:no-alert is not lowercase letters, digits and dashes"] },
    ],
    [
      "a candidate with no id",
      directory => {
        mkdirSync(join(directory, "candidates", "rules"), { recursive: true });
        writeJson(join(directory, "candidates", "rules", "no-pass.json"), { format: "rerail.policy/v1", clauses: [] });
      },
      { rules: ["candidates/rules/no-pass.json: has no id"] },
    ],
    [
      "a candidate in an older document format",
      directory => {
        mkdirSync(join(directory, "candidates", "rules"), { recursive: true });
        writeJson(join(directory, "candidates", "rules", "no-pass.json"), { format: "tempo.policy/v1", id: "no-pass", checks: [] });
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
      "a public package",
      directory => editJson(join(directory, "package.json"), manifest => ({ ...manifest, publishConfig: { access: "public" } })),
      { package: ["package.json: publishConfig.access is public; packs stay restricted until launch"] },
    ],
    [
      "a package.json with no publishConfig",
      directory => editJson(join(directory, "package.json"), ({ publishConfig: _publishConfig, ...manifest }) => manifest),
      { package: ["package.json: publishConfig.access is missing; packs stay restricted until launch"] },
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
      "a copied file no UPSTREAM.md lists",
      directory => writeFileSync(join(directory, "upstream", "examplelint", "fixtures", "more.py"), "assert d\n"),
      { licences: ["upstream/examplelint/fixtures/more.py is under upstream/<tool>/ but neither licenses/*/UPSTREAM.md (copied) nor its tool's UPSTREAM.md (Rerail's) lists it"] },
    ],
    [
      "a file copied from an upstream path its UPSTREAM.md lists as not copied",
      directory => appendFileSync(join(directory, "licenses", "examplelint", "UPSTREAM.md"), "\n- Not copied: `tests/` — a copyleft project, GPL-3.0-only\n"),
      { licences: ["licenses/examplelint/UPSTREAM.md lists upstream/examplelint/fixtures/asserts.py as copied from tests/asserts.py, under tests/, which it lists as not copied"] },
    ],
    [
      "an authored file the tool's UPSTREAM.md does not list",
      directory => writeFileSync(join(directory, "upstream", "examplelint", "UPSTREAM.md"), "# examplelint: files Rerail wrote\n"),
      { licences: ["upstream/examplelint/fixtures/clean.py is under upstream/<tool>/ but neither licenses/*/UPSTREAM.md (copied) nor its tool's UPSTREAM.md (Rerail's) lists it"] },
    ],
  ];

  it.each(cases)("fails %s", async (_name, change, expected) => {
    const validation = await validatePack(changed("valid", change), { corpora: false });
    expect(failing(validation)).toEqual(expected);
    expect(validation.pass).toBe(false);
  });

  it("marks the rule a failure belongs to", async () => {
    const validation = await validatePack(
      changed("valid", directory => writeFileSync(join(directory, "divergences.json"), "[]\n")),
      { corpora: false },
    );
    expect(validation.rules.map(({ id, status }) => ({ id, status }))).toEqual([
      { id: "assert-used", status: "failed" },
      { id: "pass-statement", status: "ok" },
    ]);
  });

  it("passes a pack whose ESLint port honours ESLint's suppression comments", async () => {
    const validation = await validatePack(join(FIXTURES, "eslint-port"), { corpora: false });
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
    const validation = await validatePack(changed("eslint-port", withoutSuppress), { corpora: false });
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
      { corpora: false },
    );
    expect(failing(validation)).toEqual({});
  });

  it("fails a stale suppression exemption, for a port that suppresses its upstream rule", async () => {
    const validation = await validatePack(changed("eslint-port", exempted), { corpora: false });
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
      { corpora: false },
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
    expect(failing(await validatePack(changed("eslint-port", suppressing({ eslint, oxlint })), { corpora: false }))).toEqual(missing);
    const otherRule = suppressing({ eslint, oxlint, biome: { rules: ["lint/suspicious/noConsole"] } });
    expect(failing(await validatePack(changed("eslint-port", otherRule), { corpora: false }))).toEqual(missing);
    const alone = await validatePack(changed("eslint-port", suppressing({ biome: { rules: ["lint/suspicious/noDebugger"] } })), { corpora: false });
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
    // Vue's registry entry honours eslint alone: a Vue query refuses suppress.biome.
    const vue = (directory: string) =>
      editJson(join(directory, "rules", "no-debugger.json"), document => ({
        ...document,
        clauses: z
          .array(z.looseObject({ forbid: z.looseObject({ ast: z.looseObject({}) }), violation: z.record(z.string(), z.unknown()) }))
          .parse(document.clauses)
          .map(clause => ({
            ...clause,
            forbid: { ...clause.forbid, ast: { ...clause.forbid.ast, language: "vue" } },
            violation: { ...clause.violation, suppress: { eslint: { rules: ["no-debugger"] } } },
          })),
      }));
    const validation = await validatePack(changed("eslint-port", vue), { corpora: false });
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
    expect(packedFileFailures(files, ["pack.json", "CHANGELOG.md", "checks/match.mjs"])).toEqual([
      "CHANGELOG.md: package.json files does not publish it",
      "checks/match.mjs: package.json files does not publish it",
    ]);
  });

  it("passes a directory a pack adds to files, such as checks/", () => {
    expect(packedFileFailures([...files, "checks"], ["checks/match.mjs"])).toEqual([]);
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
