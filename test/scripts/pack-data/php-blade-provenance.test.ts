/**
 * The provenance entries the PHP pack's (phpcs, phpstan, psalm) and the Blade pack's (blade-parser, bladestan, tlint)
 * generators hand the writer: each detail form as the snapshot provenance.json records it. The generators need PHP to
 * run; their `provenance` functions don't.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { join, resolve } from "node:path";
import { describe, test } from "vitest";

const PACKS = resolve(import.meta.dirname, "..", "..", "..", "packs");

/** `provenance(input)` of packs/<pack>/upstream/<tool>/generate.py, in a Python of its own (each pack has a `common`). */
function provenance(pack: string, tool: string, input: unknown): unknown {
  const script = [
    "import importlib.util, json, sys",
    `spec = importlib.util.spec_from_file_location("generate", ${JSON.stringify(join(PACKS, pack, "upstream", tool, "generate.py"))})`,
    "generate = importlib.util.module_from_spec(spec)",
    "spec.loader.exec_module(generate)",
    "print(json.dumps(generate.provenance(json.loads(sys.stdin.read()))))",
  ].join("\n");
  const run = spawnSync("python3", ["-B", "-c", script], { input: JSON.stringify(input), encoding: "utf8" });
  assert.equal(run.status, 0, run.stderr);
  return JSON.parse(run.stdout);
}

describe("php", () => {
  test("phpcs: a sniff's test file, verbatim with a `.php` suffix", () => {
    assert.deepEqual(provenance("php", "phpcs", { "Generic/PHP/LowerCaseConstantUnitTest.1.inc.php": "src/Standards/Generic/Tests/PHP/LowerCaseConstantUnitTest.1.inc" }), {
      "fixtures/Generic/PHP/LowerCaseConstantUnitTest.1.inc.php": {
        source: "php_codesniffer",
        from: "src/Standards/Generic/Tests/PHP/LowerCaseConstantUnitTest.1.inc",
        how: "verbatim",
        detail: "verbatim, renamed with a `.php` suffix so the host routes it to PHP",
      },
    });
  });

  test("phpstan: every file of a unit, a multi-file unit's too, verbatim from tests/PHPStan", () => {
    const verbatim = (path: string) => ({ source: "phpstan", from: `tests/PHPStan/${path}`, how: "verbatim" });
    assert.deepEqual(
      provenance("php", "phpstan", [
        "Rules/Variables/data/defined-variables.php",
        ["Rules/Comparison/data/call-condition-cross-file-a.php", "Rules/Comparison/data/call-condition-cross-file-b.php"],
      ]),
      {
        "fixtures/Rules/Variables/data/defined-variables.php": verbatim("Rules/Variables/data/defined-variables.php"),
        "fixtures/Rules/Comparison/data/call-condition-cross-file-a.php": verbatim("Rules/Comparison/data/call-condition-cross-file-a.php"),
        "fixtures/Rules/Comparison/data/call-condition-cross-file-b.php": verbatim("Rules/Comparison/data/call-condition-cross-file-b.php"),
      },
    );
  });

  test("psalm: a valid and an invalid case, the `code` of its provider's case", () => {
    const cases = [
      { path: "ForbiddenCodeTest/invalid/execTicks.php", test: "ForbiddenCodeTest", provider: "invalid", name: "execTicks" },
      { path: "UnusedVariableTest/valid/variableUsedIndirectly.php", test: "UnusedVariableTest", provider: "valid", name: "variableUsedIndirectly" },
    ];
    assert.deepEqual(provenance("php", "psalm", cases), {
      "fixtures/ForbiddenCodeTest/invalid/execTicks.php": {
        source: "psalm",
        from: "tests/ForbiddenCodeTest.php",
        how: "extracted",
        detail: "the `code` of providerInvalidCodeParse() case `execTicks`",
      },
      "fixtures/UnusedVariableTest/valid/variableUsedIndirectly.php": {
        source: "psalm",
        from: "tests/UnusedVariableTest.php",
        how: "extracted",
        detail: "the `code` of providerValidCodeParse() case `variableUsedIndirectly`",
      },
    });
  });
});

describe("blade", () => {
  test("blade-parser: a template a test passes the parser, with and without a dataset row", () => {
    const copies = [
      ["ComponentShorthandVariableParameterTest/2.blade.php", "tests/Validation/ComponentShorthandVariableParameterTest.php", "component shorthand validator does not detect issues", null],
      ["DebugDirectiveTest/10.blade.php", "tests/Validation/DebugDirectiveTest.php", "debug directive validator does not detect issues", "('elsecannot')"],
    ];
    assert.deepEqual(provenance("blade", "blade-parser", copies), {
      "fixtures/ComponentShorthandVariableParameterTest/2.blade.php": {
        source: "blade-parser",
        from: "tests/Validation/ComponentShorthandVariableParameterTest.php",
        how: "extracted",
        detail: "the template test `component shorthand validator does not detect issues` passes to `Document::fromText()`, captured verbatim",
      },
      "fixtures/DebugDirectiveTest/10.blade.php": {
        source: "blade-parser",
        from: "tests/Validation/DebugDirectiveTest.php",
        how: "extracted",
        detail: "the template test `debug directive validator does not detect issues` passes to `Document::fromText()` (dataset `('elsecannot')`), captured verbatim",
      },
    });
  });

  test("bladestan: a skeleton template, verbatim from tests/", () => {
    assert.deepEqual(provenance("blade", "bladestan", ["skeleton/resources/views/view-render/some_int.blade.php"]), {
      "fixtures/skeleton/resources/views/view-render/some_int.blade.php": {
        source: "bladestan",
        from: "tests/skeleton/resources/views/view-render/some_int.blade.php",
        how: "verbatim",
      },
    });
  });

  test("tlint: a template a test method passes the lint", () => {
    const cases = [["SpacesAroundBladeRenderContentTest/9.blade.php", "SpacesAroundBladeRenderContent", "tests/Linting/Linters/SpacesAroundBladeRenderContentTest.php", "does_not_trigger_on_escaped_blade", [{ equals: 1 }]]];
    assert.deepEqual(provenance("blade", "tlint", cases), {
      "fixtures/SpacesAroundBladeRenderContentTest/9.blade.php": {
        source: "tlint",
        from: "tests/Linting/Linters/SpacesAroundBladeRenderContentTest.php",
        how: "extracted",
        detail: "the template test `does_not_trigger_on_escaped_blade` passes to `new SpacesAroundBladeRenderContent(...)`, captured verbatim",
      },
    });
  });
});
