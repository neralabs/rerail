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
      rules: [{ id: `${id}:a`, document: "rules/a.json" }],
    }),
    [`packs/${id}/package.json`]: JSON.stringify({
      name: `@neralabs/rerail-pack-${id}`,
      license: "Apache-2.0",
      files: ["pack.json", "rules", "LICENSE", "NOTICE", "README.md"],
    }),
    [`packs/${id}/README.md`]: "",
    [`packs/${id}/LICENSE`]: "",
    [`packs/${id}/NOTICE`]: "",
    [`packs/${id}/rules/a.json`]: rule(`${id}:a`),
    ...overrides,
  };
}

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
  it("accepts families with a language, a pack or a shared declaration; flags the rest and rule-less packs", () => {
    const result = orphans(
      memoryRepo({
        "languages/php.json": "{}",
        "fixtures/php/scope/a.json": "{}",
        "fixtures/react/value/b.json": "{}",
        "packs/react/rules/r.json": rule("react:r"),
        "fixtures/ecma/scope/c.json": "{}",
        "fixtures/cobol/scope/d.json": "{}",
        "packs/empty/pack.json": "{}",
      }),
    );
    expect(paths(result)).toEqual(["fixtures/cobol", "packs/empty"]);
  });
});
