// scripts/ship/translations-model.ts: a rule's findings through its original and its translation compared as sets, a
// rule's outcome, the gate of a pack's equivalence.json against a run, the file rewritten from a run, and a pack's
// examples laid out as corpora. No engine runs here.

import { describe, expect, it } from "vitest";
import {
  compareFindings,
  type Declaration,
  DeclarationError,
  DECLARATION_FORMAT,
  emptyDeclaration,
  exampleFiles,
  type Finding,
  gateProblems,
  layOutExamples,
  NOT_EXPLAINED,
  parseDeclaration,
  renderDeclaration,
  type RuleResult,
  ruleResult,
  summarize,
  writeDifferences,
} from "../../../scripts/ship/translations-model.js";

const f = (path: string, line: number, column?: number, code = "C", rule = "r"): Finding => ({ rule, path, line, code, ...(column === undefined ? {} : { column }) });

/** A rule's result on the named corpora: each corpus's original-only and translation-only counts. */
function result(rule: string, counts: Record<string, [number, number]>): RuleResult {
  const corpora: RuleResult["corpora"] = {};
  for (const [corpus, [o, t]] of Object.entries(counts)) corpora[corpus] = { both: 1, originalOnly: o, translationOnly: t, samples: { originalOnly: [], translationOnly: [] } };
  const differs = Object.values(counts).some(([o, t]) => o + t > 0);
  return { rule, status: differs ? "different" : "same", corpora };
}

const declaration = (d: Partial<Declaration>): Declaration => ({ ...emptyDeclaration("p"), equivalent: true, ...d });

describe("comparing a rule's findings", () => {
  it("compares (rule, path, line, code) as sets, duplicates and messages aside", () => {
    const c = compareFindings([f("a.ts", 1), f("a.ts", 1), f("a.ts", 2), f("b.ts", 3, undefined, "X")], [f("a.ts", 1), f("a.ts", 4), f("b.ts", 3, undefined, "Y")]);
    expect(c).toEqual({ both: 1, originalOnly: 2, translationOnly: 2, samples: { originalOnly: ["a.ts:2 C", "b.ts:3 X"], translationOnly: ["a.ts:4 C", "b.ts:3 Y"] } });
  });

  it("compares columns only where both sides give one", () => {
    expect(compareFindings([f("a.ts", 1, 5)], [f("a.ts", 1)])).toMatchObject({ both: 1, originalOnly: 0, translationOnly: 0 });
    expect(compareFindings([f("a.ts", 1, 5)], [f("a.ts", 1, 7)])).toMatchObject({ both: 0, originalOnly: 1, translationOnly: 1, samples: { originalOnly: ["a.ts:1:5 C"], translationOnly: ["a.ts:1:7 C"] } });
    expect(compareFindings([f("a.ts", 1, 5), f("a.ts", 1, 9)], [f("a.ts", 1, 9)])).toMatchObject({ both: 1, originalOnly: 1, translationOnly: 0 });
  });

  it("keeps a few samples and every count", () => {
    const many = Array.from({ length: 12 }, (_, i) => f("a.ts", i + 1));
    const c = compareFindings(many, []);
    expect(c.originalOnly).toBe(12);
    expect(c.samples.originalOnly).toHaveLength(5);
  });
});

describe("a rule's outcome", () => {
  it("is untranslated without a .rerail, failed when refused or a side failed, else same or different", () => {
    expect(ruleResult({ rule: "r", translation: undefined, corpora: {} }).status).toBe("untranslated");
    expect(ruleResult({ rule: "r", translation: "r.rerail", refused: "check-rr reader-failed r.rerail: x", corpora: {} })).toMatchObject({ status: "failed", message: "check-rr reader-failed r.rerail: x" });
    expect(ruleResult({ rule: "r", translation: "r.rerail", corpora: { a: { original: [], translation: [] }, b: { failure: "the translation failed: boom" } } })).toMatchObject({ status: "failed", message: "b: the translation failed: boom" });
    expect(ruleResult({ rule: "r", translation: "r.rerail", corpora: { a: { original: [f("x.ts", 1)], translation: [f("x.ts", 1)] } } }).status).toBe("same");
    expect(ruleResult({ rule: "r", translation: "r.rerail", corpora: { a: { original: [f("x.ts", 1)], translation: [] } } }).status).toBe("different");
    expect(summarize([result("a", { e: [0, 0] }), result("b", { e: [1, 0] }), { rule: "c", status: "failed", corpora: {} }])).toEqual({ same: 1, different: 1, untranslated: 0, failed: 1 });
  });

  it("compares a rule its original failed on some files of, and keeps those failures beside it", () => {
    const r = ruleResult({ rule: "r", translation: "r.rerail", corpora: { a: { original: [f("x.ts", 1)], translation: [f("x.ts", 1), f("y.ts", 2)], originalFailures: ["source-incomplete: file has syntax errors", "budget-exceeded: spent"] } } });
    expect(r).toMatchObject({ status: "different", corpora: { a: { both: 1, translationOnly: 1, originalFailures: { count: 2, first: "source-incomplete: file has syntax errors" } } } });
  });
});

describe("the gate", () => {
  const pack = ["same", "diff", "none", "gone"];

  it("passes a run that matches its declaration", () => {
    const d = declaration({ untranslated: { none: "the engine has no comments" }, different: { diff: { reason: "the original misses a re-export", right: "translation", counts: { examples: { originalOnly: 0, translationOnly: 2 } } } } });
    const results = [result("same", { examples: [0, 0] }), result("diff", { examples: [0, 2] }), { rule: "none", status: "untranslated" as const, corpora: {} }];
    expect(gateProblems(d, results, ["examples"], pack)).toEqual([]);
  });

  it("fails a different rule not listed", () => {
    expect(gateProblems(declaration({}), [result("diff", { examples: [1, 0] })], ["examples"], pack)).toEqual(["diff: reports different findings and is not listed in different (examples: 1 original-only, 0 translation-only)"]);
  });

  it("fails a listed rule whose counts changed, on a corpus the run compared", () => {
    const d = declaration({ different: { diff: { reason: "why", right: "original", counts: { examples: { originalOnly: 1, translationOnly: 0 } } } } });
    expect(gateProblems(d, [result("diff", { examples: [2, 0], real: [0, 1] })], ["examples", "real"], pack)).toEqual([
      "diff: on examples, declared 1 original-only, 0 translation-only, now 2 original-only, 0 translation-only",
      "diff: on real, declared 0 original-only, 0 translation-only, now 0 original-only, 1 translation-only",
    ]);
  });

  it("fails a listed rule that is the same now", () => {
    const d = declaration({ different: { same: { reason: "why", right: "original", counts: { examples: { originalOnly: 1, translationOnly: 0 } } } } });
    expect(gateProblems(d, [result("same", { examples: [0, 0] })], ["examples"], pack)).toEqual(["same: reports the same findings as its original now; remove it from different"]);
  });

  it("fails a rule without a translation that is not listed, and a listed one that is translated now", () => {
    expect(gateProblems(declaration({}), [{ rule: "none", status: "untranslated", corpora: {} }], ["examples"], pack)).toEqual(["none: has no translation and is not listed in untranslated"]);
    expect(gateProblems(declaration({ untranslated: { same: "no comments" } }), [result("same", { examples: [0, 0] })], ["examples"], pack)).toEqual(["same: is translated now; remove it from untranslated"]);
  });

  it("fails any failed rule, listed or not", () => {
    const failed: RuleResult = { rule: "none", status: "failed", message: "examples: the translation failed: boom", corpora: {} };
    expect(gateProblems(declaration({}), [failed], ["examples"], pack)).toEqual(["none: failed: examples: the translation failed: boom"]);
    expect(gateProblems(declaration({ untranslated: { none: "boom" } }), [failed], ["examples"], pack)).toEqual(["none: failed: examples: the translation failed: boom"]);
  });

  it("fails entries without their explanation, and entries naming no rule of the pack", () => {
    const d = declaration({ untranslated: { none: " " }, different: { diff: { reason: "", right: "unknown", counts: { examples: { originalOnly: 1, translationOnly: 0 } } }, ghost: { reason: "x", right: "neither", counts: {} } } });
    expect(gateProblems(d, [{ rule: "none", status: "untranslated", corpora: {} }, result("diff", { examples: [1, 0] })], ["examples"], pack)).toEqual([
      "none: listed in untranslated without the engine's refusal or the feature it lacks",
      "diff: listed in different without a reason",
      "diff: listed in different without saying which side is right (original, translation or neither)",
      "ghost: listed in different, but the pack has no such rule",
    ]);
  });

  it("lists, but does not ask for explanations of, a pack that does not gate", () => {
    const d = declaration({ equivalent: false, different: { diff: { reason: NOT_EXPLAINED, right: "unknown", counts: { examples: { originalOnly: 1, translationOnly: 0 }, elsewhere: { originalOnly: 9, translationOnly: 9 } } } } });
    expect(gateProblems(d, [result("diff", { examples: [1, 0] })], ["examples"], pack)).toEqual([]);
  });

  it("fails a gating run that compared no corpus, and counts listed on a corpus it did not compare", () => {
    expect(gateProblems(declaration({}), [], [], pack)).toEqual(["the run compared no corpus"]);
    expect(gateProblems(declaration({ equivalent: false }), [], [], pack)).toEqual([]);
    const d = declaration({ different: { diff: { reason: "why", right: "original", counts: { examples: { originalOnly: 1, translationOnly: 0 }, gone: { originalOnly: 2, translationOnly: 0 } } } } });
    expect(gateProblems(d, [result("diff", { examples: [1, 0] })], ["examples"], pack)).toEqual(["diff: listed in different with counts on gone, which the run did not compare"]);
  });

  it("fails an untranslated rule still listed in different", () => {
    const d = declaration({ untranslated: { none: "no comments" }, different: { none: { reason: "why", right: "original", counts: { examples: { originalOnly: 1, translationOnly: 0 } } } } });
    expect(gateProblems(d, [{ rule: "none", status: "untranslated", corpora: {} }], ["examples"], pack)).toEqual(["none: has no translation; remove it from different"]);
  });

  it("fails a same rule its original failed on files of for another reason than syntax errors, and the original's failures of no rule", () => {
    const same = (originalFailures: string[]): RuleResult => ruleResult({ rule: "same", translation: "same.rerail", corpora: { examples: { original: [f("x.ts", 1)], translation: [f("x.ts", 1)], originalFailures } } });
    const parse = ["source-incomplete: file did not parse: error regions cover 90% of it, first at line 1, column 3", "source-incomplete: file has syntax errors", "source-partial: read around 2 error regions"];
    expect(same(parse).status).toBe("same");
    expect(gateProblems(declaration({}), [same(parse)], ["examples"], pack)).toEqual([]);
    expect(gateProblems(declaration({}), [same([...parse, "budget-exceeded: spent"])], ["examples"], pack)).toEqual(["same: on examples, the original failed on 1 file(s) for another reason than syntax errors; the first: budget-exceeded: spent"]);
    expect(gateProblems(declaration({ equivalent: false }), [same(["budget-exceeded: spent"])], ["examples"], pack)).toEqual([]);
    expect(gateProblems(declaration({}), [same(parse)], ["examples"], pack, { examples: ["adapter-error: boom", "adapter-error: again"] })).toEqual(["on examples, the original's run had 2 failure(s) of no rule; the first: adapter-error: boom"]);
    expect(gateProblems(declaration({ equivalent: false }), [same(parse)], ["examples"], pack, { examples: ["adapter-error: boom"] })).toEqual([]);
  });

  it("fails a listed different rule its original failed on files of for another reason than syntax errors", () => {
    const diff = ruleResult({ rule: "diff", translation: "diff.rerail", corpora: { examples: { original: [f("x.ts", 1)], translation: [], originalFailures: ["budget-exceeded: spent"] } } });
    expect(diff.status).toBe("different");
    const d = declaration({ different: { diff: { reason: "why", right: "translation", counts: { examples: { originalOnly: 1, translationOnly: 0 } } } } });
    expect(gateProblems(d, [diff], ["examples"], pack)).toEqual(["diff: on examples, the original failed on 1 file(s) for another reason than syntax errors; the first: budget-exceeded: spent"]);
    expect(gateProblems({ ...d, equivalent: false }, [diff], ["examples"], pack)).toEqual([]);
  });
});

describe("the declaration", () => {
  it("round-trips, its lists sorted", () => {
    const d = declaration({ untranslated: { z: "a", a: "b" }, different: { m: { reason: "r", right: "neither", counts: { examples: { originalOnly: 1, translationOnly: 2 } } } } });
    const text = renderDeclaration(d);
    expect(Object.keys(JSON.parse(text).untranslated)).toEqual(["a", "z"]);
    expect(parseDeclaration(text, "p")).toEqual({ ...d, untranslated: { a: "b", z: "a" } });
  });

  it("refuses a file that is not one", () => {
    const base = { format: DECLARATION_FORMAT, pack: "p", equivalent: true, untranslated: {}, different: {} };
    expect(() => parseDeclaration(JSON.stringify({ ...base, format: "v0" }), "p")).toThrow(/format/);
    expect(() => parseDeclaration(JSON.stringify({ ...base, pack: "q" }), "p")).toThrow(/names pack/);
    expect(() => parseDeclaration(JSON.stringify({ ...base, equivalent: "yes" }), "p")).toThrow(/equivalent/);
    expect(() => parseDeclaration(JSON.stringify({ ...base, different: { r: { reason: "x", right: "both", counts: {} } } }), "p")).toThrow(/right/);
    expect(() => parseDeclaration(JSON.stringify({ ...base, different: { r: { reason: "x", right: "original", counts: { e: { originalOnly: 1 } } } } }), "p")).toThrow(/counts/);
  });

  it("says why a file is not one with a DeclarationError, for text that is not JSON too", () => {
    expect(() => parseDeclaration("{ not json", "p")).toThrow(DeclarationError);
    expect(() => parseDeclaration("{ not json", "p")).toThrow(/^equivalence\.json of p: not JSON: /);
    expect(() => parseDeclaration("[]", "p")).toThrow(new DeclarationError("equivalence.json of p: not a JSON object"));
  });

  it("is rewritten from a run, keeping reasons and the counts of corpora not run", () => {
    const before = declaration({
      equivalent: false,
      untranslated: { none: "no comments", now: "was untranslated" },
      different: { diff: { reason: "known", right: "original", counts: { fixtures: { originalOnly: 4, translationOnly: 0 }, examples: { originalOnly: 9, translationOnly: 9 } } }, healed: { reason: "old", right: "original", counts: {} } },
    });
    const results: RuleResult[] = [
      { rule: "none", status: "untranslated", message: "no .rerail translation", corpora: {} },
      { rule: "fresh", status: "untranslated", message: "no .rerail translation", corpora: {} },
      result("now", { examples: [0, 0] }),
      result("diff", { examples: [1, 0] }),
      result("new", { examples: [0, 3] }),
      result("healed", { examples: [0, 0] }),
      { rule: "broken", status: "failed", message: "examples: the translation failed: boom", corpora: {} },
    ];
    const after = writeDifferences(before, results, ["examples"], { real: "not prepared" });
    expect(after.untranslated).toEqual({ none: "no comments", fresh: `no .rerail translation; ${NOT_EXPLAINED}`, broken: "failed: examples: the translation failed: boom" });
    expect(after.different).toEqual({
      diff: { reason: "known", right: "original", counts: { examples: { originalOnly: 1, translationOnly: 0 }, fixtures: { originalOnly: 4, translationOnly: 0 } } },
      new: { reason: NOT_EXPLAINED, right: "unknown", counts: { examples: { originalOnly: 0, translationOnly: 3 } } },
    });
    expect(after.incomplete).toEqual({ real: "not prepared" });
    expect(gateProblems(after, results.filter((r) => r.status !== "failed"), ["examples"], ["none", "fresh", "now", "diff", "new", "healed", "broken"])).toEqual([]);
  });

  it("leaves a gating pack's new entries unexplained, so its gate fails until someone explains them", () => {
    const results: RuleResult[] = [{ rule: "fresh", status: "untranslated", corpora: {} }, result("new", { examples: [0, 3] })];
    const after = writeDifferences(declaration({}), results, ["examples"]);
    expect(after.untranslated).toEqual({ fresh: "" });
    expect(after.different.new).toMatchObject({ reason: "", right: "unknown" });
    expect(gateProblems(after, results, ["examples"], ["fresh", "new"])).toHaveLength(3);
  });
});

describe("a pack's examples", () => {
  it("are the examples that are files, with why the others are not", () => {
    const pack = {
      rules: [
        { id: "a", examples: { caught: { path: "src/x.ts", text: "x" }, passes: { path: "src/y.ts", text: "y" } } },
        { id: "b", examples: { caught: { text: "npm run fixtures:push" } } },
        { id: "c", examples: { caught: { path: "../out.ts", text: "z" } } },
        { id: "d" },
      ],
    };
    const { files, skipped } = exampleFiles(pack);
    expect(files).toEqual([
      { rule: "a", kind: "caught", path: "src/x.ts", text: "x" },
      { rule: "a", kind: "passes", path: "src/y.ts", text: "y" },
    ]);
    expect(skipped).toEqual(["b caught: no path (not a file's example)", "c caught: path ../out.ts is not a plain relative path"]);
  });

  it("are laid out in as few corpora as hold them without two texts at one path", () => {
    const e = (path: string, text: string): { rule: string; kind: string; path: string; text: string } => ({ rule: "r", kind: "caught", path, text });
    const bins = layOutExamples([e("a.ts", "1"), e("a.ts", "1"), e("a.ts", "2"), e("b.ts", "3"), e("a.ts/c.ts", "4"), e("a.ts", "5")]);
    expect(bins.map((bin) => Object.fromEntries(bin))).toEqual([{ "a.ts": "1", "b.ts": "3" }, { "a.ts": "2" }, { "a.ts/c.ts": "4" }, { "a.ts": "5" }]);
  });
});
