/**
 * The vocabulary of the pack validation contract (docs/pack-format.md, "The validation contract"): the nine checks,
 * the sub-checks every failure is filed under, and what a rule and a sub-check can come out as. A leaf module: the
 * scrub list (scrub.ts) and the report schema (validation-report.ts) need the ids when they load, and the validator
 * (pack-validation.ts) imports both.
 */

/**
 * Checks 1 to 6 are the language packs' own bar; 7 to 9 judge a port against what its upstream tool's source says
 * (provenance, generality, size). Check 4 also carries the upstream-test sub-checks.
 */
export const PACK_CHECKS = [
  { number: 1, name: "rules" },
  { number: 2, name: "proof" },
  { number: 3, name: "examples" },
  { number: 4, name: "ports" },
  { number: 5, name: "package" },
  { number: 6, name: "licences" },
  { number: 7, name: "provenance" },
  { number: 8, name: "generality" },
  { number: 9, name: "size" },
] as const;
export type PackCheckName = (typeof PACK_CHECKS)[number]["name"];
export type PackCheckNumber = (typeof PACK_CHECKS)[number]["number"];

/**
 * Where a sub-check's result is recorded. `rule`: on each rule of pack.json. `tool`: once per upstream tool of
 * conformance.json. `pack`: once for the pack. A tool-level or pack-level failure fails its check and never sets a
 * rule's status. A `rules` failure that names no rule (pack.json does not parse, a file is not listed) is pack level.
 */
export type SubCheckLevel = "rule" | "tool" | "pack";

/**
 * The rules a sub-check judges, or for a tool-level or pack-level one, the rules it stands behind. `every`: every
 * rule. `examples`: rules with `examples`. `ports`: rules with a port entry in conformance.json. A rule outside them
 * gets `n/a`.
 */
export type SubCheckApplies = "every" | "examples" | "ports";

/**
 * Whether scrub.json may cover a sub-check's failure, and the group the report puts it in. `fidelity`: a defect of
 * the port. `upstream-policy`: upstream's own position on the rule, such as deprecating it or not turning it on in
 * any approved configuration. `null`: never scrubbed.
 */
export type ScrubGroup = "fidelity" | "upstream-policy";

export interface SubCheck {
  /** The check's name for the check's own bar, else `<check>:<name>`. check-scrub.mjs reads these quoted literals. */
  id: string;
  check: PackCheckName;
  level: SubCheckLevel;
  applies: SubCheckApplies;
  /** Judged only for a port whose tool has recorded source facts (conformance.json `tools[].recommended`). */
  needsSourceFacts: boolean;
  scrub: ScrubGroup | null;
  /** When it fails. */
  fails: string;
}

/** Every sub-check, in check order. A failure always names one. */
export const SUB_CHECKS = [
  {
    id: "rules",
    check: "rules",
    level: "rule",
    applies: "every",
    needsSourceFacts: false,
    scrub: null,
    fails:
      "pack.json does not parse or names another pack; a rules/ file is not listed; a listed document is missing, repeats a JSON key, is not a valid rerail.policy/v1 code rule, does not compile as the catalog compiles it, or has a document id or compiled id other than its listed id",
  },
  {
    id: "rules:deterministic",
    check: "rules",
    level: "rule",
    applies: "every",
    needsSourceFacts: false,
    scrub: null,
    fails: "the document has an llm clause, which needs a model at evaluation time",
  },
  {
    id: "rules:in-scope",
    check: "rules",
    level: "rule",
    applies: "ports",
    needsSourceFacts: false,
    scrub: null,
    fails:
      "the compiled rule is out of scope for src/example<extension>, for some extension its tool's approved configurations lint (source.json `extensions`), or, for a tool without source facts, for some suffix of a dialect of a language its clauses name",
  },
  {
    id: "proof",
    check: "proof",
    level: "rule",
    applies: "every",
    needsSourceFacts: false,
    scrub: null,
    fails: "the rule has neither examples nor a port entry",
  },
  {
    id: "examples",
    check: "examples",
    level: "rule",
    applies: "examples",
    needsSourceFacts: false,
    scrub: null,
    fails: "the caught example produces no violation of the rule, or the passes example produces one",
  },
  {
    id: "ports:points",
    check: "ports",
    level: "rule",
    applies: "ports",
    needsSourceFacts: false,
    scrub: null,
    fails:
      "the conformance runner's bar at the port's declared match: an unexplained FP on any scored source, an unexplained FN on the fixtures, an engine failure, a recording error the port owns, or a stale divergence",
  },
  {
    id: "ports:span",
    check: "ports",
    level: "rule",
    applies: "ports",
    needsSourceFacts: false,
    scrub: "fidelity",
    fails: "on the fixtures, a finding that ports:points matches differs in a finer part the recording carries (column, endLine, endColumn)",
  },
  {
    id: "ports:unknown-misses",
    check: "ports",
    level: "rule",
    applies: "ports",
    needsSourceFacts: false,
    scrub: "fidelity",
    fails: "on the fixtures, an upstream finding Rerail misses is excused only by a rerail-unknown divergence",
  },
  {
    id: "ports:evidence",
    check: "ports",
    level: "rule",
    applies: "ports",
    needsSourceFacts: false,
    scrub: "fidelity",
    fails: "no upstream finding of the rule is reproduced, at every part the recording carries, in a fixture unit where the port agrees completely",
  },
  {
    id: "ports:coverage",
    check: "ports",
    level: "rule",
    applies: "ports",
    needsSourceFacts: true,
    scrub: "fidelity",
    fails:
      "an upstream test of the rule (a test file, or a RuleTester case in one) is not among the fixtures check 4 scores: source.json's `unscored` lists it, with why",
  },
  {
    id: "ports",
    check: "ports",
    level: "pack",
    applies: "every",
    needsSourceFacts: false,
    scrub: null,
    fails: "a declared capability has no ported rule and no exemption, an exemption is stale, a recording or corpus error belongs to no port, or a golden manifest does not list exactly the pack's rules",
  },
  {
    id: "package",
    check: "package",
    level: "pack",
    applies: "every",
    needsSourceFacts: false,
    scrub: null,
    fails: "package.json is not the pack's publishable package",
  },
  {
    id: "licences",
    check: "licences",
    level: "pack",
    applies: "every",
    needsSourceFacts: false,
    scrub: null,
    fails: "a copied file is not listed in an UPSTREAM.md, a listed file is missing, a recorded licence is not on the allowlist, or LICENSE is missing",
  },
  {
    id: "provenance:pins",
    check: "provenance",
    level: "tool",
    applies: "ports",
    needsSourceFacts: true,
    scrub: null,
    fails:
      "source.json is missing or invalid; or its tool, version, configurations or commit differ from the tool's directory, conformance.json's tools[] entry, or the tool's recordings",
  },
  {
    id: "provenance:upstream-rule",
    check: "provenance",
    level: "rule",
    applies: "ports",
    needsSourceFacts: true,
    scrub: "fidelity",
    fails: "source.json has no entry for the port's upstream rule, or the entry lists no implementation file or no test file",
  },
  {
    id: "provenance:stable",
    check: "provenance",
    level: "rule",
    applies: "ports",
    needsSourceFacts: true,
    scrub: "upstream-policy",
    fails: "upstream deprecates the rule",
  },
  {
    id: "provenance:recommended",
    check: "provenance",
    level: "rule",
    applies: "ports",
    needsSourceFacts: true,
    scrub: "upstream-policy",
    fails: "no approved configuration turns the rule on",
  },
  {
    id: "generality:names",
    check: "generality",
    level: "rule",
    applies: "ports",
    needsSourceFacts: true,
    scrub: "fidelity",
    fails: "the document hardcodes a name that occurs in upstream's tests but not in upstream's code",
  },
  {
    id: "generality:regexes",
    check: "generality",
    level: "rule",
    applies: "ports",
    needsSourceFacts: true,
    scrub: "fidelity",
    fails: "a regex of the document cannot be run as a JavaScript regex, so it cannot be judged",
  },
  {
    id: "size:combinatorial-expansion",
    check: "size",
    level: "rule",
    applies: "ports",
    needsSourceFacts: true,
    scrub: "fidelity",
    fails: "the document is over 15,360 bytes and a third or more of its clauses is structure written out over and over",
  },
  {
    id: "size:written-out-repetition",
    check: "size",
    level: "rule",
    applies: "ports",
    needsSourceFacts: true,
    scrub: "fidelity",
    fails: "the document is over 15,360 bytes and one node kind occurs three or more times along a single nesting path",
  },
] as const satisfies readonly SubCheck[];

export type SubCheckId = (typeof SUB_CHECKS)[number]["id"];
export type RuleSubCheckId = Extract<(typeof SUB_CHECKS)[number], { level: "rule" }>["id"];
export type ScrubbableSubCheckId = Exclude<(typeof SUB_CHECKS)[number], { scrub: null }>["id"];

const ids = <T extends SubCheckId>(keep: (check: (typeof SUB_CHECKS)[number]) => boolean): [T, ...T[]] => {
  const kept = SUB_CHECKS.filter(keep).map(check => check.id as T);
  if (kept.length === 0) throw new Error("no sub-check matches");
  return kept as [T, ...T[]];
};

export const SUB_CHECK_IDS = ids<SubCheckId>(() => true);
/** The sub-checks every rule has a result for, in report order. */
export const RULE_SUB_CHECK_IDS = ids<RuleSubCheckId>(check => check.level === "rule");
export const SCRUBBABLE_SUB_CHECK_IDS = ids<ScrubbableSubCheckId>(check => check.scrub !== null);

/**
 * `ok`: no failure. `failed`: a failure scrub.json does not cover, or a stale scrub.json entry. `scrubbed`: every
 * failure is covered by scrub.json. `not-supported-yet`: the rule compiles to `language-not-supported`. Checks 1 and
 * 2 judge it, and every later rule-level sub-check reports `not-supported-yet`.
 */
export const PACK_RULE_STATUSES = ["ok", "failed", "scrubbed", "not-supported-yet"] as const;
export type PackRuleStatus = (typeof PACK_RULE_STATUSES)[number];

/**
 * One rule's outcome on one sub-check. Besides the statuses: `n/a`, the sub-check does not apply to the rule (a check
 * 4 or 7-9 sub-check on a Rerail rule proven by examples, check 3 on a rule without examples). `not-judged`, it
 * applies but this layer records no evidence for it: checks 7-9 for a tool without source facts, or check 4 for a
 * port another harness scores (TSLint's golden manifest). Neither counts as a pass.
 */
export const SUB_CHECK_OUTCOMES = ["ok", "failed", "scrubbed", "n/a", "not-judged", "not-supported-yet"] as const;
export type SubCheckOutcome = (typeof SUB_CHECK_OUTCOMES)[number];
