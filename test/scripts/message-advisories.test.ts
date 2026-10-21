import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { COMMENT_MARKER, documentAdvisories, renderAdvisories, ruleDocuments } from "../../scripts/review/message-advisories.js";
import { fixturesPath } from "../support/oracles.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

/** A repository root holding a copy of the `valid` fixture pack as packs/valid, its messages set as given. */
function repository(messages: Record<string, string>): string {
  const root = mkdtempSync(join(tmpdir(), "rerail-message-advisories-"));
  roots.push(root);
  cpSync(fixturesPath("packs", "valid"), join(root, "packs", "valid"), { recursive: true });
  for (const [name, message] of Object.entries(messages)) {
    const path = join(root, "packs", "valid", "rules", `${name}.json`);
    const document = JSON.parse(readFileSync(path, "utf8")) as { clauses: { violation: object }[] };
    document.clauses = document.clauses.map(clause => ({ ...clause, violation: { ...clause.violation, message } }));
    writeFileSync(path, JSON.stringify(document));
  }
  return root;
}

describe("message advisories", () => {
  it("takes shipped and candidate rule documents, not probes or other pack files", () => {
    expect(ruleDocuments([
      "packs/python/rules/ruff-b905.json",
      "packs/python/candidates/rules/bandit-b404.json",
      "packs/python/candidates/probes/rules/probe-assert-used.json",
      "packs/python/upstream/ruff/fixtures.expected.json",
      "packs/python/pack.json",
      "docs/pack-format.md",
    ])).toEqual(["packs/python/rules/ruff-b905.json", "packs/python/candidates/rules/bandit-b404.json"]);
  });

  it("lists each document's advisories, with the upstream rule code its conformance.json records", () => {
    const root = repository({ "assert-used": "Replace this EX101 assert with an explicit check.", "pass-statement": "Consider removing this pass statement." });
    const documents = documentAdvisories(root, ["packs/valid/rules/assert-used.json", "packs/valid/rules/pass-statement.json"]);
    expect(documents).toEqual([
      { file: "packs/valid/rules/assert-used.json", advisories: ["message names its upstream rule code EX101: Replace this EX101 assert with an explicit check."] },
      { file: "packs/valid/rules/pass-statement.json", advisories: ["message hedges instead of advising: Consider removing this pass statement."] },
    ]);
  });

  it("renders one comment that starts with its marker, and nothing when no document has an advisory", () => {
    const root = repository({ "pass-statement": "Consider removing this pass statement; keep it as a <placeholder>." });
    const markdown = renderAdvisories(documentAdvisories(root, ["packs/valid/rules/assert-used.json", "packs/valid/rules/pass-statement.json"]));
    expect(markdown.startsWith(`${COMMENT_MARKER}\n## Message advisories\n\n1 advisory on the rule messages this pull request adds or changes`)).toBe(true);
    expect(markdown).toContain("**`packs/valid/rules/pass-statement.json`**\n- message hedges instead of advising: Consider removing this pass statement; keep it as a &lt;placeholder&gt;.\n");
    expect(markdown).not.toContain("assert-used");
    expect(renderAdvisories(documentAdvisories(repository({}), ["packs/valid/rules/assert-used.json"]))).toBe("");
  });
});
