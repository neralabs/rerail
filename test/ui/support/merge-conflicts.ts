import type { Conflict, ConflictSide } from "../../../src/engine/index.js";

/** One side of a conflict: this branch's by default. */
export const side = (over: Partial<ConflictSide> = {}): ConflictSide => ({
  label: "This branch", ref: "tbelanger/export-api", sha: "aaa111", author: "tbelanger", tip: null, url: null,
  summary: "Clones each matcher per rule copy.", location: "native/src/model.rs:469",
  hunk: "@@ -469 +469 @@\n- matchers: Vec<GrammarMatcher>,\n+ matchers: Vec<GrammarMatcher>, // cloned\n",
  ...over,
});

/** An open, ambient overlap with pull request #398, detected `detectedAt`. */
export const conflict = (id: string, over: Partial<Conflict> = {}): Conflict => ({
  id, shortId: "a1b2c3", kind: "overlap", status: "open", level: "ambient", signals: null, score: 120,
  reasons: ["local code: ×1", "+40 lines on theirs in 2 h", "3 callers", "even odds either side loses"], queued: false, queuedFor: null,
  question: `question:${id}`, title: `Shared matchers ${id}`,
  mine: side(),
  theirs: side({
    label: "PR #398", ref: "refs/pull/398/head", sha: "bbb222", author: "Tim", url: "https://github.com/acme/storefront/pull/398",
    summary: "Shares compiled matchers; no rule copy deep-copies them.", hunk: "+ matchers: Arc<[GrammarMatcher]>,",
  }),
  evidence: ["Both change native/src/model.rs lines 465–472."],
  detectedAt: "2026-11-02T11:00:00Z",
  dismissal: null,
  summary: "PR #398 (Tim) is changing matchers",
  hook: null,
  ...over,
});

/** Conflict `id`, dismissed as not a conflict by tbelanger at `at`. */
export const dismissed = (id: string, at: string, over: Partial<Conflict> = {}): Conflict =>
  conflict(id, { status: "dismissed", dismissal: { name: "tbelanger", at }, ...over });
