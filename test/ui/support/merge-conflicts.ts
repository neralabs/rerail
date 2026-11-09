import type { Conflict, ConflictSide, DecisionLogItem } from "../../../src/engine/index.js";

/** One side of a conflict: this branch's by default. */
export const side = (over: Partial<ConflictSide> = {}): ConflictSide => ({
  label: "This branch", ref: "tbelanger/export-api", sha: "aaa111", author: "tbelanger", tip: null, url: null,
  summary: "Clones each matcher per rule copy.", location: "native/src/model.rs:469",
  hunk: "@@ -469 +469 @@\n- matchers: Vec<GrammarMatcher>,\n+ matchers: Vec<GrammarMatcher>, // cloned\n", decisionId: null,
  ...over,
});

/** An open, ambient overlap with pull request #398, detected `detectedAt`. */
export const conflict = (id: string, over: Partial<Conflict> = {}): Conflict => ({
  id, shortId: "a1b2c3", kind: "overlap", status: "open", level: "ambient", signals: null, score: 120,
  reasons: ["local code: ×1", "+40 lines on theirs in 2 h", "3 callers"], queued: false, queuedFor: null, snooze: null,
  question: `question:${id}`, title: `Shared matchers ${id}`,
  mine: side(),
  theirs: side({
    label: "PR #398", ref: "refs/pull/398/head", sha: "bbb222", author: "Tim", url: "https://github.com/acme/storefront/pull/398",
    summary: "Shares compiled matchers; no rule copy deep-copies them.", hunk: "+ matchers: Arc<[GrammarMatcher]>,",
  }),
  evidence: ["Both change native/src/model.rs lines 465–472.", "No decision in force answers it."],
  detectedAt: "2026-11-02T11:00:00Z",
  resolution: null,
  ...over,
});

/** A conflict resolved by `by` with `action`, saved as `rule`. */
export const resolved = (id: string, action: "mine" | "theirs" | "new", by: "person" | "precedent", over: Partial<Conflict> = {}): Conflict =>
  conflict(id, {
    status: "resolved",
    resolution: {
      action, by, name: by === "person" ? "tbelanger" : "precedent", logId: `log-${id}`,
      rule: `.rerail/candidates/${id}.json`, text: `Decision for ${id}.`, at: "2026-11-02T11:30:00Z",
    },
    ...over,
  });

export const logItem = (id: string, recordedAt: string, over: Partial<DecisionLogItem> = {}): DecisionLogItem => ({
  id, question: `overlap:${id}`, title: `Question ${id}`, status: "provisional", verdict: "b", outcome: "Theirs (PR #398)",
  by: { kind: "person", name: "tbelanger" }, source: { ref: "mine", label: "This branch" }, recordedAt,
  rule: `.rerail/candidates/${id}.json`, text: `Text ${id}.`, follows: null,
  ...over,
});

/** The decision log as the status carries it: one question per item, in the order given. */
export const logOf = (items: readonly DecisionLogItem[], problems: string[] = []) => ({
  questions: items.map(item => ({ question: item.question, title: item.title, items: [item] })),
  problems,
});
