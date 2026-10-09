/**
 * The pack validation contract (docs/pack-format.md): every packs/<id>/ directory passes checks 1 to 6, one test per
 * check, and reports each rule's status; a rule core cannot read yet is skipped as not supported yet. Check 4 scores
 * ported rules on the tools' fixtures, and on the pinned corpora that are checked out (RERAIL_PACK_CORPORA=1: every
 * corpus, a missing checkout failing; 0: none).
 *
 * The validator's own tests run it over the fixture packs in fixtures/packs/: `valid` passes, each negative
 * breaks one check of a copy of it, and `vue`, for a language core plans, passes with its rule not supported yet.
 */
import { cpSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, test } from "vitest";
import { z } from "zod";
import { PACK_CHECKS, validatePack, type PackCheckName, type PackValidation, type ValidatePackOptions } from "../conformance/pack-validation.js";
import { compareDeterministicStrings } from "../src/stable-json.js";
import { fixturesPath } from "./oracles.js";

const PACKS = fileURLToPath(new URL("../packs/", import.meta.url));
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
        .map(rule => (rule.id === "valid:pass-statement" ? edit(rule) : rule)),
    }));

  const failing = (validation: PackValidation): Partial<Record<PackCheckName, string[]>> =>
    Object.fromEntries(Object.entries(validation.checks).filter(([, failures]) => failures.length > 0));

  it("passes a valid pack, every rule ok", async () => {
    const validation = await validatePack(join(FIXTURES, "valid"), { corpora: false });
    expect(failing(validation)).toEqual({});
    expect(validation.pass).toBe(true);
    expect(validation.rules).toEqual([
      { id: "valid:assert-used", document: "rules/assert-used.json", status: "ok" },
      { id: "valid:pass-statement", document: "rules/pass-statement.json", status: "ok" },
    ]);
  });

  it("passes a pack for a planned language, its rules not supported yet and their examples skipped", async () => {
    const validation = await validatePack(join(FIXTURES, "vue"), { corpora: false });
    expect(failing(validation)).toEqual({});
    expect(validation.rules).toEqual([
      { id: "vue:no-v-html", document: "rules/no-v-html.json", status: "not-supported-yet", reason: "/clauses/0/forbid/ast/language: `vue` is not supported yet" },
    ]);
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
      { rules: ["valid:pass-statement: rules/pass-statement.json does not exist"] },
    ],
    [
      "a document whose id differs from the listing",
      directory => editJson(join(directory, "rules", "pass-statement.json"), document => ({ ...document, id: "valid:pass" })),
      { rules: ["valid:pass-statement: rules/pass-statement.json has id valid:pass"] },
    ],
    [
      "a rule with neither examples nor a port entry",
      directory => editPassStatement(directory, ({ examples: _examples, ...rule }) => rule),
      { proof: ["valid:pass-statement: no examples and no port entry in conformance.json"] },
    ],
    [
      "a caught example that does not fire",
      directory => editPassStatement(directory, rule => ({ ...rule, examples: { caught: { path: "src/a.py", text: "x = 1\n" }, passes: { path: "src/a.py", text: "x = 2\n" } } })),
      { examples: ["valid:pass-statement: the caught example (src/a.py) produces no PASS violation"] },
    ],
    [
      "a passes example that fires",
      directory => editPassStatement(directory, rule => ({ ...rule, examples: { caught: { path: "src/a.py", text: "pass\n" }, passes: { path: "src/a.py", text: "pass\n" } } })),
      { examples: ["valid:pass-statement: the passes example (src/a.py) produces PASS"] },
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
      { id: "valid:assert-used", status: "failed" },
      { id: "valid:pass-statement", status: "ok" },
    ]);
  });
});
