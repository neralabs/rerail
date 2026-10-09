import { existsSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { FIXTURE_RULES, loadScoredRules, loadVariants } from "./fixtures.js";
import { goldenRules } from "./golden.js";
import { literalTokens } from "./literal-scan.js";
import { renameCheck } from "./poison.js";
import { globalProtectedNames, optionPatterns, protectionFor, renameIdentifiers, syntaxDiagnostics, type Protection } from "./rename.js";

const bare: Protection = { names: globalProtectedNames(), patterns: [] };

/**
 * The rename is a same-length substitution of whole words by the map: outside words nothing changed, and every changed
 * word is a renamed name (private names without their `#`). This is what keeps TSLint's expected spans on their tokens.
 */
function isMappedRename(original: string, renamed: string, renames: Map<string, string>): boolean {
  if (original.length !== renamed.length) return false;
  const bareMap = new Map([...renames].map(([from, to]) => [from.replace(/^#/, ""), to.replace(/^#/, "")]));
  let cursor = 0;
  for (const match of original.matchAll(/[\p{L}_$][\p{L}\p{N}_$]*/gu)) {
    if (original.slice(cursor, match.index) !== renamed.slice(cursor, match.index)) return false;
    const word = match[0];
    const now = renamed.slice(match.index, match.index + word.length);
    if (now !== word && bareMap.get(word) !== now) return false;
    cursor = match.index + word.length;
  }
  return original.slice(cursor) === renamed.slice(cursor);
}

describe("renameIdentifiers", () => {
  it("renames user identifiers consistently to same-length, same-shape names and keeps protected ones", () => {
    const source = "class MyClass { foo(bar: string) { console.log(bar, parseInt(bar)); return this.foo; } }\nconst MAX_SIZE = 1;\n";
    const { source: renamed, renames } = renameIdentifiers(source, "a.ts", bare);
    expect(renamed).toHaveLength(source.length);
    expect([...renames.keys()].sort()).toEqual(["MAX_SIZE", "MyClass", "bar", "foo"]);
    for (const [from, to] of renames) {
      expect(to).toHaveLength(from.length);
      expect(to.replace(/[A-Z]/g, "A").replace(/[a-z]/g, "a")).toBe(from.replace(/[A-Z]/g, "A").replace(/[a-z]/g, "a"));
      expect(to).not.toBe(from);
    }
    expect(renamed).toContain("console.log(");
    expect(renamed).toContain("parseInt(");
    expect(renamed.match(new RegExp(`\\b${renames.get("foo")!}\\b`, "g"))).toHaveLength(2);
    expect(isMappedRename(source, renamed, renames)).toBe(true);
  });

  it("keeps a Hungarian I prefix, leaves string and comment contents alone, and renames name-position string literals", () => {
    const source = "interface IFoo { a: number; \"a\"(): void }\n// foo mentions a\nconst s = 'foo'; o[\"a\"];\n";
    const { source: renamed, renames } = renameIdentifiers(source, "a.ts", bare);
    expect(renames.get("IFoo")).toMatch(/^I[A-Z][a-z]{2}$/);
    const a = renames.get("a")!;
    expect(renamed).toContain(`"${a}"(): void`);
    expect(renamed).toContain(`${renames.get("o")!}["${a}"]`);
    expect(renamed).toContain("// foo mentions a");
    expect(renamed).toContain("'foo'");
  });

  it("protects intrinsic JSX tags and their attributes, renames user components and private names", () => {
    const source = "class C { #secret = 1; get() { return <div className=\"x\" onClick={this.#secret}><Widget flavor={1} /></div>; } }\n";
    const { source: renamed, renames } = renameIdentifiers(source, "a.tsx", bare);
    expect(renamed).toContain("<div className=\"x\" onClick=");
    expect(renames.get("#secret")).toMatch(/^#[a-z]{6}$/);
    expect(renames.has("Widget")).toBe(true);
    expect(renames.has("flavor")).toBe(true);
    expect(syntaxDiagnostics(renamed, "a.tsx")).toEqual(syntaxDiagnostics(source, "a.tsx"));
  });

  it("protects names that an option pattern selects and names spelled with unicode escapes", () => {
    const protection: Protection = { names: globalProtectedNames(), patterns: optionPatterns([{ "allowed-names": ["^start"] }]) };
    const source = "const startEnd = this; const endStart = this; let \\u0061bc = 1; abc++;\n";
    const { renames } = renameIdentifiers(source, "a.ts", protection);
    expect(renames.has("startEnd")).toBe(false);
    expect(renames.has("endStart")).toBe(true);
    expect(renames.has("abc")).toBe(false);
  });

  it("permutes one-letter names instead of running out", () => {
    const letters = "abcdefghijklmnopqrstuvwxyz".split("");
    const source = `${letters.map(letter => `let ${letter} = 1;`).join("\n")}\n`;
    const { renames, exhausted } = renameIdentifiers(source, "a.ts", bare);
    expect(exhausted).toEqual([]);
    expect(new Set(renames.values()).size).toBe(renames.size);
  });
});

describe("literalTokens", () => {
  it("drops metavariables, regex escapes, flags and character classes", () => {
    expect(literalTokens("$A.foo($$$B)", false)).toEqual(["foo"]);
    expect(literalTokens("(?i)^[A-Z]\\s*\\bbar\\\\n", true)).toEqual(["bar"]);
  });
});

describe.skipIf(!existsSync(FIXTURE_RULES))("renamed TSLint fixtures", () => {
  it("parse the same, keep every length and map back to the original text", () => {
    const problems: string[] = [];
    for (const rule of loadScoredRules()) {
      for (const variant of loadVariants(rule.name)) {
        for (const file of variant.files) {
          if (file.parsed.skipped) continue;
          const { source } = file.parsed;
          const renamed = renameIdentifiers(source, file.path, protectionFor(rule.name, file.config.options));
          const where = `${rule.name}/${variant.id}/${file.lintPath}`;
          if (renamed.source.length !== source.length) problems.push(`${where}: length changed`);
          else if (!isMappedRename(source, renamed.source, renamed.renames)) problems.push(`${where}: not a word-for-word rename`);
          if (syntaxDiagnostics(renamed.source, file.path).join() !== syntaxDiagnostics(source, file.path).join()) problems.push(`${where}: parse changed`);
          if (renamed.exhausted.length > 0) problems.push(`${where}: exhausted ${renamed.exhausted.join(",")}`);
        }
      }
    }
    expect(problems).toEqual([]);
  });

  const rules = loadScoredRules();
  it.each(goldenRules())("golden rule %s scores identically on renamed fixtures", async name => {
    const comparison = await renameCheck(rules.find(rule => rule.name === name)!);
    expect(comparison.renamerIssues).toEqual([]);
    expect({ lost: comparison.lostOnRename, gained: comparison.gainedOnRename, renamed: comparison.renamed }).toEqual({ lost: [], gained: [], renamed: comparison.normal });
  });
});
