// Structure gate, contract metrics (#8-#11): each predicate on a tiny in-memory tree.

import { describe, expect, it } from "vitest";
import { memoryRepo } from "../../scripts/check/check-structure/repo.mjs";
import { check as packFormat } from "../../scripts/check/check-structure/metrics/08-pack-format.mjs";
import { check as packsImportEngine } from "../../scripts/check/check-structure/metrics/09-packs-import-engine.mjs";
import { check as featureModules } from "../../scripts/check/check-structure/metrics/10-fixture-feature-modules.mjs";
import { check as orphans } from "../../scripts/check/check-structure/metrics/11-orphan-families.mjs";

const rule = (id) => JSON.stringify({ format: "rerail.policy/v1", id, clauses: [] });
const paths = (result) => result.findings.map((item) => item.path);

function validPack(id, overrides = {}) {
  return {
    [`packs/${id}/pack.json`]: JSON.stringify({
      format: "rerail.rule-pack/v1",
      id,
      name: "PHP",
      rules: [{ id: `${id}:a`, document: "rules/a.json" }],
    }),
    [`packs/${id}/package.json`]: JSON.stringify({
      name: `@neralabs/rerail-pack-${id}`,
      license: "Apache-2.0",
      files: ["pack.json", "rules", "LICENSE", "NOTICE", "README.md"],
    }),
    [`packs/${id}/README.md`]: "# Rerail rule pack: PHP\n\nBody.\n",
    [`packs/${id}/LICENSE`]: "",
    [`packs/${id}/NOTICE`]: "",
    [`packs/${id}/rules/a.json`]: rule(`${id}:a`),
    "docs/rule-packs.md": "| Pack | Category |\n|---|---|\n| PHP | language |\n",
    ...overrides,
  };
}

/** The pack named `name`, its README and the docs table following it, so only what a test overrides disagrees. */
function packNamed(name, overrides = {}) {
  const files = validPack("php", {
    "packs/php/README.md": `# Rerail rule pack: ${name}\n`,
    "docs/rule-packs.md": `| Pack | Category |\n|---|---|\n| ${name} | language |\n`,
    ...overrides,
  });
  files["packs/php/pack.json"] = JSON.stringify({ ...JSON.parse(files["packs/php/pack.json"]), name });
  return files;
}

const messages = (files) => packFormat(memoryRepo(files)).findings.map((item) => `${item.path}: ${item.message}`);

describe("#8 pack format", () => {
  it("passes a complete pack", () => {
    expect(packFormat(memoryRepo(validPack("php")))).toEqual({ actual: "100% (1/1)", findings: [] });
  });

  it("names each missing file and contract breach", () => {
    const files = validPack("php", {
      "packs/php/package.json": JSON.stringify({ name: "php-pack", license: "MIT", files: ["pack.json"] }),
      "packs/php/rules/unlisted.json": rule("php:b"),
    });
    delete files["packs/php/NOTICE"];
    const result = packFormat(memoryRepo(files));
    expect(result.actual).toBe("0% (0/1)");
    expect(result.findings.map((item) => `${item.path}: ${item.message}`)).toEqual([
      "packs/php/NOTICE: missing",
      "packs/php/package.json: files must publish LICENSE",
      "packs/php/package.json: files must publish rules",
      "packs/php/package.json: name must be @neralabs/rerail-pack-php",
      "packs/php/rules/unlisted.json: in rules/ but not listed in pack.json",
    ]);
  });

  it("rejects a pack.json whose id is not its directory, or of another format", () => {
    const files = validPack("php", { "packs/php/pack.json": JSON.stringify({ pack: "php", family: "php", rules: [] }) });
    delete files["packs/php/rules/a.json"];
    expect(packFormat(memoryRepo(files)).findings.map((item) => item.message)).toEqual([
      "format must be rerail.rule-pack/v1",
      'id must be the directory name "php"',
    ]);
  });

  it("refuses a name holding brackets or digits", () => {
    expect(messages(packNamed("PHP (strict)"))).toEqual([
      'packs/php/pack.json: name "PHP (strict)" has brackets or digits: a name says what the pack is about, and limits and versions go in covers',
    ]);
    expect(messages(packNamed("Complexity limit 6"))).toEqual([
      'packs/php/pack.json: name "Complexity limit 6" has brackets or digits: a name says what the pack is about, and limits and versions go in covers',
    ]);
    for (const name of ["PHP [strict]", "PHP {strict}"]) {
      expect(messages(packNamed(name))).toEqual([
        `packs/php/pack.json: name "${name}" has brackets or digits: a name says what the pack is about, and limits and versions go in covers`,
      ]);
    }
  });

  it("refuses a name holding a tool the pack ports, whatever its case", () => {
    const conformance = JSON.stringify({ tools: [{ name: "phpstan" }, { name: "psalm" }] });
    expect(messages(packNamed("PHP and PHPStan", { "packs/php/conformance.json": conformance }))).toEqual([
      'packs/php/pack.json: name "PHP and PHPStan" names the tool phpstan: tools go in covers',
    ]);
    expect(messages(packNamed("PHP", { "packs/php/conformance.json": conformance }))).toEqual([]);
  });

  it("requires the README heading, and the candidates README's, to carry the name", () => {
    expect(messages(validPack("php", { "packs/php/README.md": "# PHP opt-in rule pack\n" }))).toEqual([
      'packs/php/README.md: first line must be "# Rerail rule pack: PHP"',
    ]);
    expect(messages(validPack("php", { "packs/php/candidates/README.md": "# Candidates\n" }))).toEqual([
      'packs/php/candidates/README.md: first line must be "# Rerail rule pack: PHP, candidates"',
    ]);
    expect(messages(validPack("php", { "packs/php/candidates/README.md": "# Rerail rule pack: PHP, candidates\n" }))).toEqual([]);
  });

  it("requires a published pack's row in the docs table, by its exact name", () => {
    expect(messages(validPack("php", { "docs/rule-packs.md": "| Pack |\n| PHP (old) | language |\n" }))).toEqual([
      'docs/rule-packs.md: no table row names the pack "PHP"',
    ]);
    expect(messages(validPack("php", { "docs/rule-packs.md": "| Pack |\n| PHP | language |\n" }))).toEqual([]);
  });

  it("does not ask a private pack for a docs row", () => {
    const files = validPack("php", { "docs/rule-packs.md": "| Pack | Category |\n" });
    files["packs/php/package.json"] = JSON.stringify({ ...JSON.parse(files["packs/php/package.json"]), private: true });
    expect(messages(files)).toEqual([]);
  });
});

describe("#9 packs import the engine", () => {
  it("flags a pack script importing src/ or the package, not its own modules", () => {
    const result = packsImportEngine(
      memoryRepo({
        "packs/typescript/t1.ts": 'import { run } from "../../src/native.js";\nimport { x } from "./local.js";',
        "packs/react/build.mjs": 'const core = await import("@neralabs/rerail/engine");',
        "packs/php/upstream/phpcs/generate.mjs": 'import x from "./lib/src/x.js";',
      }),
    );
    expect(result.findings).toEqual([
      { path: "packs/react/build.mjs", line: 1, message: "imports the engine: @neralabs/rerail/engine" },
      { path: "packs/typescript/t1.ts", line: 1, message: "imports the engine: ../../src/native.js" },
    ]);
  });
});

describe("#10 fixture features map to engine modules", () => {
  it("maps features to native/src/code_rule_dsl modules and documented groups", () => {
    const result = featureModules(
      memoryRepo({
        "native/src/code_rule_dsl/scope/mod.rs": "",
        "native/src/code_rule_dsl/value.rs": "",
        "fixtures/php/scope/a.json": "{}",
        "fixtures/php/value/b.json": "{}",
        "fixtures/php/upstream/phpcs/c.json": "{}",
        "fixtures/php/t1/d.json": "{}",
      }),
    );
    expect(paths(result)).toEqual(["fixtures/php/t1"]);
  });
});

describe("#11 orphan families and packs", () => {
  it("accepts families with a language, a pack or a shared declaration, and packs whose rules are all candidates; flags the rest and rule-less packs", () => {
    const result = orphans(
      memoryRepo({
        "languages/php.json": "{}",
        "fixtures/php/scope/a.json": "{}",
        "fixtures/react/value/b.json": "{}",
        "packs/react/rules/r.json": rule("r"),
        "fixtures/ecma/scope/c.json": "{}",
        "fixtures/cobol/scope/d.json": "{}",
        "packs/empty/pack.json": "{}",
        "packs/vue/pack.json": "{}",
        "packs/vue/candidates/rules/c.json": rule("c"),
        "packs/probes/pack.json": "{}",
        "packs/probes/candidates/probes/rules/p.json": rule("p"),
        "packs/notes/pack.json": "{}",
        "packs/notes/candidates/rules/notes.json": JSON.stringify({ note: "not a rule document" }),
      }),
    );
    expect(paths(result)).toEqual(["fixtures/cobol", "packs/empty", "packs/notes", "packs/probes"]);
  });
});
