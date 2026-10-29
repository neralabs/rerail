import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import type { PackConformance } from "../../../scripts/samples/conformance.js";
import { phpTestSetSamples } from "../../../scripts/samples/convert-php-test-set.js";

const COMMIT = "0".repeat(40);

const temporary: string[] = [];
afterEach(() => {
  for (const directory of temporary.splice(0)) rmSync(directory, { recursive: true, force: true });
});

const conformance: PackConformance = {
  pack: "php",
  family: "php",
  rules: [
    { id: "phpcs-squiz-php-eval", policy: "rules/phpcs-squiz-php-eval.json", upstream: { tool: "phpcs", rule: "Squiz.PHP.Eval" } },
    { id: "phpstan-argument-sscanf", policy: "rules/phpstan-argument-sscanf.json", upstream: { tool: "phpstan", rule: "argument.sscanf" } },
  ],
};

function testSet(entries: unknown[], files: Record<string, unknown>): string {
  const directory = mkdtempSync(join(tmpdir(), "php-test-set-"));
  temporary.push(directory);
  const write = (path: string, text: string): void => {
    mkdirSync(join(directory, path, ".."), { recursive: true });
    writeFileSync(join(directory, path), text);
  };
  write("manifest.json", JSON.stringify({ tools: { "squizlabs/php_codesniffer": "4.0.4", "phpstan/phpstan": "2.2.16", "vimeo/psalm": "6.19.1" }, entries }));
  for (const [path, content] of Object.entries(files)) write(path, typeof content === "string" ? content : JSON.stringify(content));
  return directory;
}

describe("the PHP test set", () => {
  it("labels each original with the tool's findings and each fixed copy clean, under the tool-pack porting the rule", () => {
    const directory = testSet(
      [
        { file: "phpcs-squiz-php-eval/repo-a-0123456789ab.php", fixed: "phpcs-squiz-php-eval/repo-a-0123456789ab-fixed.php", rule: "Squiz.PHP.Eval", rerailRule: "phpcs-squiz-php-eval", pack: "php", tool: "phpcs" },
        { file: "phpstan-argument-sscanf/repo-b-0123456789ab.php", fixed: null, rule: "argument.sscanf", rerailRule: "phpstan-argument-sscanf", pack: "php", tool: "phpstan" },
      ],
      {
        "phpcs-squiz-php-eval/repo-a-0123456789ab.php": "<?php\neval($x);\n",
        "phpcs-squiz-php-eval/repo-a-0123456789ab-fixed.php": "<?php\nrun($x);\n",
        "phpcs-squiz-php-eval/repo-a-0123456789ab.php.phpcs.json": { rule: "Squiz.PHP.Eval", rerailRule: "phpcs-squiz-php-eval", source: { repo: "owner/repo", sha: COMMIT, path: "src/a.php", license: "MIT" }, findings: [{ line: 2, column: 1 }] },
        "phpcs-squiz-php-eval/repo-a-0123456789ab-fixed.php.phpcs.json": { fixedOf: "phpcs-squiz-php-eval/repo-a-0123456789ab.php", rule: "Squiz.PHP.Eval", findings: [] },
        "phpstan-argument-sscanf/repo-b-0123456789ab.php": "<?php\nsscanf($s, '%d %d', $a);\n",
        "phpstan-argument-sscanf/repo-b-0123456789ab.php.phpstan.json": { rule: "argument.sscanf", rerailRule: "phpstan-argument-sscanf", source: { repo: "owner/repo", sha: COMMIT, path: "src/b.php", license: "MIT" }, findings: [{ line: 2 }] },
      },
    );
    const { samples, originals, fixed } = phpTestSetSamples(directory, new Map([["php", conformance]]));
    expect([...originals]).toEqual([["php/phpcs", 1], ["php/phpstan", 1]]);
    expect([...fixed]).toEqual([["php/phpcs", 1]]);
    expect(samples.map((entry) => [entry.short, entry.toolPack.tool, entry.source.path, Object.fromEntries(entry.expected), entry.pinned])).toEqual([
      ["repo", "phpcs", "src/a.php", { "phpcs-squiz-php-eval": [{ line: 2, column: 1 }] }, true],
      ["repo-fixed", "phpcs", "src/a.php", { "phpcs-squiz-php-eval": [] }, true],
      ["repo", "phpstan", "src/b.php", { "phpstan-argument-sscanf": [{ line: 2 }] }, true],
    ]);
    expect(samples[0]!.labelledBy).toEqual({ tool: "phpcs", version: "4.0.4", how: "php test set" });
    expect(samples[2]!.labelledBy).toEqual({ tool: "phpstan", version: "2.2.16", how: "php test set" });
  });

  it("refuses an entry whose rule the pack does not port with that tool", () => {
    const directory = testSet(
      [{ file: "x/repo-a-0123456789ab.php", fixed: null, rule: "Squiz.PHP.Eval", rerailRule: "phpcs-squiz-php-eval", pack: "php", tool: "psalm" }],
      { "x/repo-a-0123456789ab.php": "<?php\n" },
    );
    expect(() => phpTestSetSamples(directory, new Map([["php", conformance]]))).toThrow(/not a rule of pack php porting psalm Squiz.PHP.Eval/u);
  });

  it("refuses a fixed copy that still has findings", () => {
    const directory = testSet(
      [{ file: "x/repo-a-0123456789ab.php", fixed: "x/repo-a-0123456789ab-fixed.php", rule: "Squiz.PHP.Eval", rerailRule: "phpcs-squiz-php-eval", pack: "php", tool: "phpcs" }],
      {
        "x/repo-a-0123456789ab.php": "<?php\neval($x);\n",
        "x/repo-a-0123456789ab-fixed.php": "<?php\neval($y);\n",
        "x/repo-a-0123456789ab.php.phpcs.json": { rule: "Squiz.PHP.Eval", rerailRule: "phpcs-squiz-php-eval", source: { repo: "owner/repo", sha: COMMIT, path: "src/a.php", license: "MIT" }, findings: [{ line: 2, column: 1 }] },
        "x/repo-a-0123456789ab-fixed.php.phpcs.json": { fixedOf: "x/repo-a-0123456789ab.php", rule: "Squiz.PHP.Eval", findings: [{ line: 2, column: 1 }] },
      },
    );
    expect(() => phpTestSetSamples(directory, new Map([["php", conformance]]))).toThrow(/not a clean fixed copy/u);
  });
});
