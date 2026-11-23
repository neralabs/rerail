import { describe, expect, it } from "vitest";
import type { RerailRuleEntry } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { takes } from "../../../src/engine/rr/rr-rule-scope.js";

/** A Python rule on everywhere, with the scope the engine's describe reports for it. */
function rule(scope: Partial<RerailRuleEntry["scope"]>): RerailRuleEntry {
  return {
    id: "rr-scoped",
    path: "/packs/rr/rules/rr-scoped.rerail",
    placement: [{ folder: "", mode: "steer" }],
    scope: { include: [], exclude: [], text: [], ...scope },
    language: "python",
    files: [],
    provenance: {
      ruleId: "rr-scoped", revision: 1, payloadHash: "hash", source: { kind: "pack", id: "rr", packId: "rr" },
      repositoryKey: "repository", checkoutPath: "/repo", placement: [],
    },
  };
}

describe("which writes a .rerail rule takes, by its .scope", () => {
  it("takes every file of its language with no .scope", () => {
    expect(takes(rule({}), "docs/conf.py")).toBe(true);
    expect(takes(rule({}), "shop/views.html")).toBe(false);
  });

  it("takes only the files under its folders with .scope dirs, which the engine reports as include globs", () => {
    const shop = rule({ include: ["shop/**"] });
    expect(takes(shop, "shop/views.py")).toBe(true);
    expect(takes(shop, "docs/conf.py")).toBe(false);
  });

  it("takes no test file with .scope tests false, which the engine reports as exclude globs", () => {
    const noTests = rule({ exclude: ["**/test/**", "**/tests/**", "test/**", "tests/**"] });
    expect(takes(noTests, "shop/tests/test_views.py")).toBe(false);
    expect(takes(noTests, "tests/test_views.py")).toBe(false);
    expect(takes(noTests, "shop/views.py")).toBe(true);
  });

  it("takes the files its .scope include globs match, and none an exclude glob matches", () => {
    const scoped = rule({ include: ["**/*.py"], exclude: ["migrations/**"] });
    expect(takes(scoped, "shop/views.py")).toBe(true);
    expect(takes(scoped, "migrations/0001_initial.py")).toBe(false);
  });

  it("takes files of another kind its .scope text globs read as text, never a hidden one, and only in scope", () => {
    const templates = rule({ include: ["shop/**"], text: ["**/*.html"] });
    expect(takes(templates, "shop/templates/home.html")).toBe(true);
    expect(takes(templates, "shop/templates/.draft.html")).toBe(false);
    expect(takes(templates, "docs/index.html")).toBe(false);
    expect(takes(templates, "shop/notes.md")).toBe(false);
  });

  it("takes no file where its placement turns it off", () => {
    expect(takes({ ...rule({}), placement: [{ folder: "", mode: "steer" }, { folder: "legacy", mode: null }] }, "legacy/old.py")).toBe(false);
  });
});
