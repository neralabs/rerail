// Structure gate, location metrics (#1-#7): each predicate on a tiny in-memory tree.

import { describe, expect, it } from "vitest";
import { memoryRepo } from "../../scripts/check/check-structure/repo.mjs";
import { check as packJsonLocation } from "../../scripts/check/check-structure/metrics/01-pack-json-location.mjs";
import { check as ruleDocLocation } from "../../scripts/check/check-structure/metrics/02-rule-doc-location.mjs";
import { check as fixtureDataLocation } from "../../scripts/check/check-structure/metrics/03-fixture-data-location.mjs";
import { check as engineRoots } from "../../scripts/check/check-structure/metrics/04-engine-source-roots.mjs";
import { check as testRoots } from "../../scripts/check/check-structure/metrics/05-test-roots.mjs";
import { check as generatorLocations } from "../../scripts/check/check-structure/metrics/06-generator-locations.mjs";
import { check as packLicences } from "../../scripts/check/check-structure/metrics/07-pack-licence-annotations.mjs";

const rule = (id) => JSON.stringify({ format: "rerail.policy/v1", id, clauses: [] });
const paths = (result) => result.findings.map((item) => item.path);

describe("#1 pack.json location", () => {
  it("accepts pack manifests and validator inputs, rejects nested and stray pack.json", () => {
    const result = packJsonLocation(
      memoryRepo({
        "packs/php/pack.json": "{}",
        "fixtures/packs/valid/pack.json": "{}",
        "packs/next/candidates/pack.json": "{}",
        "test/fixtures/packs/vue/pack.json": "{}",
      }),
    );
    expect(paths(result)).toEqual(["packs/next/candidates/pack.json", "test/fixtures/packs/vue/pack.json"]);
  });
});

describe("#2 rule document location", () => {
  it("allows pack rules/ and candidates/ and fixtures test inputs, flags rule documents anywhere else", () => {
    const result = ruleDocLocation(
      memoryRepo({
        "packs/php/rules/a.json": rule("php:a"),
        "packs/example/candidates/b/c.json": rule("example:b"),
        "fixtures/php/compile/d.json": rule("d"),
        "fixtures/host/steer/e.json": rule("e"),
        "benchmark/usage/with/f.json": rule("f"),
        "benchmark/usage/with/other.json": JSON.stringify({ format: "something/v1" }),
      }),
    );
    expect(paths(result)).toEqual(["benchmark/usage/with/f.json"]);
    expect(result.actual).toBe("1 in 1 dirs");
  });

  it("rejects rule sets parked under fixtures/ (rules, candidates, probes, policies dirs) except validator packs", () => {
    const result = ruleDocLocation(
      memoryRepo({
        "fixtures/next/probes/a.json": rule("a"),
        "fixtures/node/candidates/x/b.json": rule("b"),
        "fixtures/php/rules/c.json": rule("c"),
        "fixtures/react/policies/d.json": rule("d"),
        "fixtures/packs/valid/rules/e.json": rule("e"),
      }),
    );
    expect(paths(result)).toEqual([
      "fixtures/next/probes/a.json",
      "fixtures/node/candidates/x/b.json",
      "fixtures/php/rules/c.json",
      "fixtures/react/policies/d.json",
    ]);
  });
});

describe("#3 fixture data location", () => {
  it("flags recordings outside fixtures/ and packs/<id>/upstream/", () => {
    const result = fixtureDataLocation(
      memoryRepo({
        "fixtures/php/scope/a.expected.json": "{}",
        "packs/php/upstream/phpcs/fixtures.expected.json": "{}",
        "test/data/b.expected.json": "{}",
      }),
    );
    expect(paths(result)).toEqual(["test/data/b.expected.json"]);
  });
});

describe("#4 engine source roots", () => {
  const engine = (source) => memoryRepo({ "native/Cargo.toml": "", "native/src/lib.rs": source, "fixtures/php/x.json": "" });

  it("counts an include_str! that leaves native/ as a second root", () => {
    const result = engineRoots(engine('const C: &str = include_str!("../../code-rule-dsl/capabilities.json");'));
    expect(result.actual).toBe("2");
    expect(result.findings).toEqual([
      { path: "native/src/lib.rs", line: 1, message: "include_str! reaches outside native/: code-rule-dsl/capabilities.json" },
    ]);
  });

  it("resolves CARGO_MANIFEST_DIR includes against the crate", () => {
    const result = engineRoots(engine('include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../model/m.bin"));'));
    expect(paths(result)).toEqual(["native/src/lib.rs"]);
  });

  it("lets test-only code read fixtures/ but not production code", () => {
    const include = 'include_str!("../../fixtures/php/x.json")';
    expect(engineRoots(engine(`fn a() {}\n#[cfg(test)]\nmod tests { const X: &str = ${include}; }`)).findings).toEqual([]);
    expect(engineRoots(engine(`const X: &str = ${include};\n#[cfg(test)]\nmod tests {}`)).findings).toHaveLength(1);
  });

  it("exempts only the item #[cfg(test)] applies to, not the rest of the file", () => {
    const repo = (source) =>
      memoryRepo({ "native/Cargo.toml": "", "native/src/lib.rs": source, "packs/example/rules/quotemark.json": "{}" });
    const declared = '#[cfg(test)]\nmod tests;\n\npub const RULES: &str = include_str!("../../packs/example/rules/quotemark.json");\n';
    expect(engineRoots(repo(declared)).findings).toEqual([
      { path: "native/src/lib.rs", line: 4, message: "include_str! reaches outside native/: packs/example/rules/quotemark.json" },
    ]);
    const afterBody = [
      "#[cfg(test)]",
      "#[allow(dead_code)]",
      "mod tests {",
      '    const OPEN: &str = "{"; // }',
      "    const C: char = '{';",
      "    fn f<'a>(x: &'a str) -> &'a str { x }",
      "}",
      'pub const RULES: &str = include_str!("../../packs/example/rules/quotemark.json");',
    ].join("\n");
    expect(paths(engineRoots(repo(afterBody)))).toEqual(["native/src/lib.rs"]);
    const inBody = '#[cfg(test)]\nmod tests {\n    const R: &str = include_str!("../../packs/example/rules/quotemark.json");\n}\n';
    expect(engineRoots(repo(inBody)).findings).toEqual([]);
    const item = '#[cfg(test)]\nconst R: [&str; 1] = [include_str!("../../packs/example/rules/quotemark.json")];\n';
    expect(engineRoots(repo(item)).findings).toEqual([]);
  });

  it("keeps file-based test exemptions", () => {
    const include = 'const X: &str = include_str!("../../../fixtures/php/x.json");';
    const files = ["native/src/m/tests.rs", "native/src/m/parse_tests.rs", "native/src/m/test_support.rs"];
    for (const path of files) {
      expect(engineRoots(memoryRepo({ "native/Cargo.toml": "", [path]: include, "fixtures/php/x.json": "" })).findings).toEqual([]);
    }
    const integration = memoryRepo({ "native/Cargo.toml": "", "native/tests/a.rs": 'include_str!("../../fixtures/php/x.json")' });
    expect(engineRoots(integration).findings).toEqual([]);
    expect(engineRoots(memoryRepo({ "native/Cargo.toml": "", "native/src/m/prod.rs": include })).findings).toHaveLength(1);
  });

  it("flags any tracked file in the retired code-rule-dsl/", () => {
    expect(paths(engineRoots(memoryRepo({ "code-rule-dsl/capabilities.json": "{}" })))).toEqual(["code-rule-dsl/capabilities.json"]);
  });
});

describe("#5 test roots", () => {
  it("allows test/, integration/, playground/ and data roots; flags tests elsewhere", () => {
    const result = testRoots(
      memoryRepo({
        "test/a.test.ts": "",
        "integration/b.test.ts": "",
        "playground/ui/c.test.tsx": "",
        "fixtures/python/value/test_d.py": "",
        "packs/php/upstream/phpcs/fixtures/FooTest.php": "",
        "scripts/e.test.mjs": "",
        "scripts/test_f.py": "",
        "conformance/example/g.spec.ts": "",
      }),
    );
    expect(paths(result)).toEqual(["conformance/example/g.spec.ts", "scripts/e.test.mjs", "scripts/test_f.py"]);
  });
});

describe("#6 generator locations", () => {
  it("allows scripts/generate/, fixtures/ and pack recorders; flags generators in the engine source", () => {
    const result = generatorLocations(
      memoryRepo({
        "scripts/generate/value/ecma-builtins.mjs": "",
        "scripts/generate/languages/generate-registry.ts": "",
        "fixtures/php/value/generate_php.php": "",
        "packs/php/upstream/phpcs/generate.py": "",
        "native/src/code_rule_dsl/value/ecma/generate_builtins.mjs": "",
        "scripts/generate-schema-sql.ts": "",
        "native/src/code_rule_dsl/value/generated.rs": "",
        "scripts/check-structure/generated.mjs": "",
      }),
    );
    expect(paths(result)).toEqual(["native/src/code_rule_dsl/value/ecma/generate_builtins.mjs", "scripts/generate-schema-sql.ts"]);
  });

  it("flags a manifest row's or header's generator outside scripts/generate/, whatever its name", () => {
    const manifest = [
      "| Generated file | Generator | Command |",
      "|---|---|---|",
      "| `src/engine/storage/schema-sql.generated.ts` | `src/engine/storage/schema-sql.ts` | `npm run generate:schema` |",
      "| `src/engine/names.generated.ts` | `scripts/generate/value/names.py` | `python3 scripts/generate/value/names.py` |",
      "| `native/src/x/oracle.json` | `fixtures/php/value/record.php` | `php fixtures/php/value/record.php` |",
    ].join("\n");
    const result = generatorLocations(
      memoryRepo({
        "scripts/generate/README.md": manifest,
        "src/engine/storage/schema-sql.ts": "",
        "src/engine/storage/schema-sql.generated.ts": "// Generated by src/engine/storage/schema-sql.ts; do not edit.\n",
        "scripts/generate/value/names.py": "",
        "src/engine/names.generated.ts": "// Generated by scripts/generate/value/names.py; do not edit.\n",
        "src/hosts/omp/tokens.ts": "/* Generated by src/hosts/omp/tokens.mjs; do not edit. */\n",
        "fixtures/php/value/record.php": "",
        "fixtures/php/value/x.generated.php": "<?php // Generated by fixtures/php/value/record.php; do not edit.\n",
        "native/src/x/oracle.json": "{}",
      }),
    );
    expect(result.findings).toEqual([
      {
        path: "scripts/generate/README.md",
        line: 3,
        message: "names generator src/engine/storage/schema-sql.ts, outside scripts/generate/ (engine/host/pack data) or fixtures/, packs/<id>/upstream/ (recordings)",
      },
      {
        path: "src/engine/storage/schema-sql.generated.ts",
        line: 1,
        message: "names generator src/engine/storage/schema-sql.ts, outside scripts/generate/ (engine/host/pack data) or fixtures/, packs/<id>/upstream/ (recordings)",
      },
      {
        path: "src/hosts/omp/tokens.ts",
        line: 1,
        message: "names generator src/hosts/omp/tokens.mjs, outside scripts/generate/ (engine/host/pack data) or fixtures/, packs/<id>/upstream/ (recordings)",
      },
    ]);
    expect(result.actual).toBe("2");
  });
});

describe("#7 pack licences", () => {
  const reuse = (extra) => `version = 1

[[annotations]]
path = "**"
SPDX-License-Identifier = "Apache-2.0"
${extra}`;
  const pack = (license) => JSON.stringify({ license });

  it("requires shipped rules to resolve to the pack's licence; the catch-all default does not suffice", () => {
    const result = packLicences(
      memoryRepo({
        "REUSE.toml": reuse(""),
        "packs/php/package.json": pack("Apache-2.0 AND MIT"),
        "packs/php/rules/a.json": rule("php:a"),
      }),
    );
    expect(result.findings).toEqual([
      { path: "packs/php/rules/a.json", message: "REUSE resolves Apache-2.0; the pack declares Apache-2.0 AND MIT" },
    ]);
  });

  it("applies the last matching annotation and compares AND terms as a set", () => {
    const result = packLicences(
      memoryRepo({
        "REUSE.toml": reuse(`
[[annotations]]
path = ["packs/php/rules/**"]
precedence = "override"
SPDX-License-Identifier = "MIT AND Apache-2.0"
`),
        "packs/php/package.json": pack("Apache-2.0 AND MIT"),
        "packs/php/rules/a.json": rule("php:a"),
        "packs/php/rules/README.md": "not a rule document",
      }),
    );
    expect(result.findings).toEqual([]);
    expect(result.actual).toBe("100% (1/1)");
  });

  it("lets candidates resolve to a subset of the pack's terms but not to a licence it does not declare", () => {
    const files = {
      "REUSE.toml": reuse(`
[[annotations]]
path = "packs/php/candidates/bsd/**"
precedence = "override"
SPDX-License-Identifier = "BSD-3-Clause"
`),
      "packs/php/package.json": pack("Apache-2.0 AND MIT"),
      "packs/php/candidates/own.json": rule("php:own"),
      "packs/php/candidates/bsd/port.json": rule("php:port"),
    };
    expect(paths(packLicences(memoryRepo(files)))).toEqual(["packs/php/candidates/bsd/port.json"]);
  });

  it("lets a .license sidecar win under closest precedence, but not over an override annotation", () => {
    const tag = `${["SPDX", "License", "Identifier"].join("-")}: Apache-2.0 AND MIT`;
    const files = (precedence) => ({
      "REUSE.toml": reuse(`
[[annotations]]
path = "packs/php/rules/**"
precedence = "${precedence}"
SPDX-License-Identifier = "Apache-2.0"
`),
      "packs/php/package.json": pack("Apache-2.0 AND MIT"),
      "packs/php/rules/a.json": rule("php:a"),
      "packs/php/rules/a.json.license": tag,
    });
    expect(packLicences(memoryRepo(files("closest"))).findings).toEqual([]);
    expect(paths(packLicences(memoryRepo(files("override"))))).toEqual(["packs/php/rules/a.json"]);
  });
});
