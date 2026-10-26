/**
 * The identifier-poisoning gate: every ECMA pack whose recording lists protected identifiers (a pack opts in with
 * `protected-identifiers.json` beside its fixtures recording) is scored again on its fixtures with every user-chosen
 * identifier renamed (conformance/languages/identifier-rename.ts).
 * A shipped port that implements structure scores identically; one that matches names only the tool's tests use loses
 * TPs or gains FPs. Candidates are not gated, as in the conformance gate.
 */
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { globalProtectedNames, renamedPack, renamedTools, renameFixtures, renameIdentifiers, syntaxDiagnostics, type Protection } from "../../conformance/languages/identifier-rename.js";
import { listPortedPacks, loadPack, PACKS_ROOT } from "../../conformance/languages/pack.js";
import type { RuleRow } from "../../conformance/languages/report.js";
import { runPack } from "../../conformance/languages/runner.js";

const bare: Protection = { names: globalProtectedNames(), patterns: [] };

/**
 * The rename is a same-length substitution of whole words by the map: outside words nothing changed, and every changed
 * word is a renamed name (private names without their `#`). This is what keeps recorded findings on their tokens.
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
    const protection: Protection = { names: globalProtectedNames(), patterns: [/^start/u] };
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

/** What the gate holds a shipped port to: its whole score, findings included, which keep their positions on renaming. */
const shippedScores = (rows: readonly RuleRow[]): unknown[] =>
  rows
    .filter(row => !row.candidate)
    .map(row => ({
      rule: row.rule,
      tp: row.tp,
      fp: row.fp,
      fn: row.fn,
      engineFailures: row.engineFailures,
      partialFiles: row.partialFiles,
      syntaxErrorFiles: row.syntaxErrorFiles.count,
      divergences: row.divergences.length,
      falsePositives: row.falsePositives,
      falseNegatives: row.falseNegatives,
    }));

const gated = listPortedPacks()
  .map(name => loadPack(join(PACKS_ROOT, name)))
  .filter(loaded => loaded.pack.family === "ecma" && renamedTools(loaded).length > 0);

describe.each(gated.map(loaded => [loaded.pack.pack, loaded] as const))("pack %s on renamed fixtures", (_, loaded) => {
  const renamed = new Map(renamedTools(loaded).map(tool => [tool, renameFixtures(loaded, tool)]));

  it("renames names only: same length, word for word, same TypeScript parse, no name left over", () => {
    const problems = [...renamed].flatMap(([tool, fixtures]) =>
      fixtures.flatMap(fixture =>
        [...fixture.problems, ...(isMappedRename(fixture.source, fixture.renamed.source, fixture.renamed.renames) ? [] : ["not a word-for-word rename"])].map(
          problem => `${tool}/${fixture.path}: ${problem}`,
        ),
      ),
    );
    expect(problems).toEqual([]);
  });

  it("scores every shipped port identically", async () => {
    const directory = mkdtempSync(join(tmpdir(), "rerail-identifier-rename-"));
    try {
      const options = { fixtures: true, samples: false, date: "2026-01-01" };
      const normal = await runPack(loaded, options);
      const renamedReport = await runPack(renamedPack(loaded, renamed, directory), options);
      expect(renamedReport.errors).toEqual(normal.errors);
      expect(shippedScores(renamedReport.rules)).toEqual(shippedScores(normal.rules));
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  }, 120_000);
});
