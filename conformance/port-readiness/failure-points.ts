/**
 * Each check a rule can fail, in words a contributor new to the gates follows, and what to do about it: the comment
 * and the job summary name a failure by these (comment.ts), not by the gate or check id. A check with no entry falls
 * back to its step's title and its id.
 */
import { STAGE_TITLES, STEPS, type StepId } from "./summary.js";

export interface FailurePoint {
  /** What is wrong, as a sentence fragment: "Flags code its test fixtures say is fine". */
  what: string;
  /** What to do about it. */
  fix: string;
}

const RECORDING_STALE: FailurePoint = { what: "Upstream's recorded answers are out of date", fix: "re-record the linter (`npm run port-readiness:record`)" };

/** By check id (types.ts Check.id), across every gate and step. */
const POINTS: Readonly<Record<string, FailurePoint>> = {
  // Conformance: compiles and loads.
  json: { what: "Is not valid JSON, or repeats a key", fix: "fix the document's JSON" },
  schema: { what: "Doesn't match the rule schema", fix: "fix the fields the detail names" },
  "current-contract": { what: "Is written for an older rule format", fix: "update it to `rerail.policy/v1`: one `mode`, no severity" },
  "engine-compile": { what: "Doesn't compile on the engine", fix: "fix what the compiler names" },
  identity: { what: "Its id doesn't match its document", fix: "make the document's `id` the id it compiles to" },
  deterministic: { what: "Needs an AI model to evaluate (an `llm` clause)", fix: "replace the `llm` clause with code clauses" },
  "bundled-load": { what: "Doesn't load alongside the pack's other rules", fix: "fix the conflict the detail names, such as two rules with one id" },
  // Conformance: compact.
  "no-combinatorial-expansion": { what: "Lists cases one by one instead of expressing the logic", fix: "rewrite the repeated clauses as one general clause" },
  "no-written-out-repetition": { what: "Nests the same pattern three or more levels deep", fix: "express the nesting as a rule, not one level at a time" },
  // Conformance: the pack's conformance check on its test fixtures.
  "no-unexplained-false-positives": { what: "Flags code its test fixtures say is fine", fix: "fix the rule, or declare the difference in the pack's `divergences.json`" },
  "no-unexplained-misses": { what: "Misses problems its test fixtures expect it to flag", fix: "fix the rule, or declare the difference in the pack's `divergences.json`" },
  "catches-something": { what: "Flags nothing in its test fixtures", fix: "add a fixture it should flag, or `examples` in `pack.json`" },
  "fixtures-row": { what: "Has no test fixtures to be scored on", fix: "add its `conformance.json` entry and record its fixtures" },
  scored: { what: "The pack's conformance check can't score it", fix: "fix what the detail names" },
  "engine-reads-fixtures": { what: "The engine fails on some of its test fixtures", fix: "fix the engine failure the detail names" },
  "divergences-current": { what: "Declares a difference from upstream that no longer happens", fix: "remove the stale entry from `divergences.json`" },
  "corpus-recorded": { what: "The pack's real-code recordings don't cover it", fix: "record the pack's corpora for its upstream rule" },
  "conformance-job": { what: "The conformance job didn't report it", fix: "re-run CI; if it persists, the conformance job is missing the rule" },
  // Port readiness: what it needs before any port readiness gate can run.
  origin: { what: "Doesn't declare where it comes from (a port of an upstream rule, or an original)", fix: "declare its origin" },
  "conformance-entry": { what: "Not declared: no `conformance.json` entry says which upstream rule it ports", fix: "add its `conformance.json` entry" },
  "gates-yaml": { what: "Its upstream linter isn't set up for port readiness yet", fix: "add the linter's `gates.yaml`, or keep the rule a candidate" },
  // Port readiness: the upstream licence.
  "license-sha256": { what: "The kept copy of upstream's licence isn't upstream's", fix: "re-record the linter, or restore `licenses/<project>/`" },
  "declared-license": { what: "Upstream's package declares another licence than `gates.yaml` records", fix: "correct `upstream.license` in `gates.yaml`" },
  "license-allowed": { what: "Upstream's licence isn't on Rerail's licence allowlist", fix: "don't port from it, or have the licence approved" },
  // Port readiness: the linter's other checks.
  "corpus-pins": RECORDING_STALE,
  "recording-covers-rules": RECORDING_STALE,
  "corpus-covers-rules": RECORDING_STALE,
  "pins-agree": RECORDING_STALE,
  "runtime-pins": RECORDING_STALE,
  "case-sources": { what: "A test case changed since upstream's answers were recorded", fix: "re-record the linter, or restore the case" },
  "corpus-complete": { what: "A real-code file it is checked on couldn't be downloaded", fix: "re-run CI" },
  "conformance-runs": { what: "The pack's conformance check didn't run", fix: "fix the error the detail names" },
  "in-scope-narrowing": { what: "`gates.yaml` limits it to a file type upstream doesn't check", fix: "correct `rerail.in_scope` in `gates.yaml`" },
  // Port readiness: the port against upstream.
  "in-scope": { what: "Doesn't check every file type upstream checks", fix: "widen its scope, or narrow `rerail.in_scope` in `gates.yaml`" },
  "no-fixture-only-names": { what: "Hardcodes names that only appear in upstream's tests", fix: "match the logic, not the test cases' names" },
  "regexes-judged": { what: "Has regexes the overfit check can't run", fix: "rewrite them in syntax the check runs" },
  "identical-results": { what: "Reports different findings than upstream on upstream's own tests", fix: "fix the rule where the first difference shows" },
  fires: { what: "Never reproduces one of upstream's findings", fix: "fix the rule so it flags what upstream flags" },
  "stays-silent": { what: "Never stays silent where upstream does", fix: "fix the rule so it passes what upstream passes" },
  "identical-on-goldens": { what: "Disagrees with upstream on real-code files", fix: "fix the rule where the first difference shows" },
  "engine-reads-goldens": { what: "The engine can't fully read some real-code files it is checked on", fix: "the fix is the engine's; the rule may be right" },
  "no-real-code-gap": { what: "Listed in `gates.yaml` as differing from upstream on real code", fix: "fix the rule, then remove its `real_code_gaps` entry" },
};

/** What the check `check` failing at `step` means, and its fix. */
export function failurePoint(step: StepId, check: string): FailurePoint {
  const known = POINTS[check];
  if (known !== undefined) return known;
  const title = STEPS.find((entry) => entry.id === step)!.title;
  return { what: `${title} (${check})`, fix: "see the detail" };
}

/** The failure with its stage: `Conformance: Flags code its test fixtures say is fine`. */
export function failureLabel(step: StepId, check: string): string {
  const stage = STEPS.find((entry) => entry.id === step)!.stage;
  return `${STAGE_TITLES[stage]}: ${failurePoint(step, check).what}`;
}
