# From rows to a verdict: the findings layer

A policy's rules derive rows of its `.violation` relations. Rows are not yet findings. The step after the query
turns them into finding keys, decides which files Rerail can vouch for, applies the gate and the budgets, and
gives the policy a verdict: pass, fail or unknown. That step is the findings layer of the target architecture: one
layer that runs after the Datalog and turns facts into findings.

Seven directives configure it. None of them is a rule; each says what to do with rows the rules have derived:

| Directive | What it decides |
|---|---|
| `code {c}`, `tag {t}` | a finding's key: its code from a column; an extra key field |
| `.on syntax_error`, `.on input_error` | whether a file the parser rejects, or an input that failed, is also a finding |
| `.gate V` | that V's findings, when there are any, replace the policy's report |
| `.budget V by … ` | how many findings may exist before they block |
| `.mode`, `.severity` | which findings block, and how they are shown |

This page gives their semantics and what the engine needs for each. The policies that use them are worked
examples: [file-length.rr](file-length.rr), [noqa-sec-ratchet.rr](noqa-sec-ratchet.rr),
[test-budget.rr](test-budget.rr). Facts that come from a CI run are in [RUN-FACTS.md](RUN-FACTS.md).

## 1. The stages

For one evaluation of a policy:

1. **Evaluate.** The rules run. Gate relations, and what they depend on, run first (section 4).
2. **Key.** Each row becomes a key `(policy, code, path, line)`, plus `tag` if the relation declares one
   (section 3). Its `at` column gives path and line: a node, a span, or a `@place`.
3. **Consolidate.** Equal keys are one finding. Keys outside `.scope report` are dropped. Suppression
   comments are applied (section 6 says what they do to budgets).
4. **Outcomes.** Each (policy, file) is `Complete` or `Unknown(reason)` (section 2).
5. **Gate.** If a gate relation has keys, they are the report, and stages 6–7 use only them (section 4).
6. **Budgets.** Findings a budget admits stop blocking; a budget that is exceeded adds its own finding
   (section 6).
7. **Verdict.** From the findings that still block and from the outcomes (section 7).

Stages 2–7 work on keys, not rows. That is the findings contract: order and multiplicity are free, so nothing
after stage 3 may depend on how many rows produced a key.

## 2. Incomplete inputs, and when a file is Unknown

The findings contract asks for two things: every key returned is a real finding, and every (policy, file)
reported `Complete` has all its keys. When an input is incomplete, the engine must give up one of them for the
places the input affects, never silently.

An input is incomplete when:

- **a document did not parse.** Its facts are withheld from the policy, as the scripts skip such a file
  (section 5);
- **a run artifact is missing, stale, failed or malformed** ([RUN-FACTS.md](RUN-FACTS.md));
- **a resource budget ran out** on a file. The engine has this today: `budget-exceeded`, `file-budget-exceeded`.

Files read with `.input … filename=` are not in this list. A missing or malformed repository file is a fact
about the repository: it gives an `input_error` row, and the policy decides what it means.

What happens to a rule that reads an incomplete input, directly or through derived relations:

1. **Positive dependence only.** Its rows are kept. With fewer facts a positive rule derives fewer rows, never a
   wrong one.
2. **Through `!` or an aggregate.** Its rows are withheld. With fewer facts, `!r(x)` can become true and a count
   can fall below a threshold, so a row could be wrong.
3. **In both cases**, the places the rule could have reported at are `Unknown(reason)`: rows may be missing there.

The places are found by evaluating the rule once more with the incomplete facts treated as unknown: an atom
over them matches anything. That over-approximates where the rule could land, which is the point:

- a rule whose `at` is `@place("scripts/probes.py", 1)` makes that one file Unknown;
- a withheld document keeps its `doc`, `path_part` and `file` facts; only its tree and text are withheld. A rule
  whose place is a node of that document can only land in it. A rule that reaches the document through a module
  name lands in the files that import that module;
- a place variable that only an unknown atom binds makes every file in the policy's report scope Unknown.

The extra evaluation runs only when something is incomplete, which on a clean CI run is never.

The granularity is the rule, not the relation. In [test-budget.rr](test-budget.rr), `budget_exceeded` has a rule
that reads the static count and two that read pytest's count. Without the artifact, the first still reports.

This follows what the engine already does for its budget clause: a count that is within its limit but only a
lower bound is a `source-incomplete` failure, never clean (`native/src/code_rule_dsl/eval/evidence.rs:240-301`).

## 3. `code {c}` and `tag {t}`: the key

A finding's code is normally the relation's name. `code {c}` takes it from column c instead:

```
.violation SEC(at: span, c: symbol) code {c} "{c} suppression annotation"
```

- The value must be a non-empty symbol of `[A-Za-z0-9_.:-]`, at most 64 characters. `code {c} in "SEC\d+"`
  declares the codes a relation can produce, for documentation and for the checker. A row whose code fails either
  test is not a finding; its place is `Unknown(invalid code)`.
- `.mode`, `.severity` and `.budget … by code` see each code: `.mode log SEC123` names one of them.
- The engine's finding id already hashes the code (`native/src/code_rule_dsl/eval/output.rs:80-90`). It
  changes from a fixed clause code to the column's value; nothing else changes.

`tag {t}` adds a field to the key, which the findings contract allows one rule to declare. Use it when several
findings share a place: each failing probe of serverplan_target_architecture is reported at line 1 of the probe
script, and without a tag they would be one key.

```
.violation SPTB(at: loc, c: symbol, probe: symbol) code {c} tag {probe} "{probe}: {c}"
```

## 4. `.gate V`: one failure that replaces the report

Several scripts stop before scanning when their allowlist or baseline is bad: they report one finding, and
nothing else, because nothing else can be trusted. `.gate V` says the same:

```
.gate invalid_baseline
.gate missing_baseline, unsigned_baseline, invalid_signature   // several: their keys together
```

When gate relations have keys after consolidation:

- the report is those keys alone;
- every other (policy, file) of the policy is `Unknown(gated)`. The other findings are not dropped silently: the
  files they would be in are reported as not vouched for, which keeps the contract;
- budgets are not evaluated, and metrics are not published;
- suppression comments do not apply to gate findings. They are usually at line 1 of a JSON file, and a gate that a
  comment can switch off is not a gate.

The checker requires that a gate relation not depend on any other violation relation of the policy. The engine
can then evaluate the gate's stratum first and skip the rest when it fires, as the scripts do. On the push path
that matters: a bad signature stops a whole-repository scan before it starts.

A gate finding blocks under the policy's mode like any other (section 7).

## 5. `.on syntax_error` and `.on input_error`

**Where `syntax_error` comes from.** `syntax_error(d, line, msg)` has one row per document that does not parse,
at the first error, as CPython reports one.

- **For Python** the verdict and line follow CPython's grammar, from a native parser (Ruff's). No CPython
  runs, in any form. Tree-sitter's tree cannot give the verdict, because it recovers from errors. If the parser
  is unavailable, the fact is an incomplete input (section 2): Unknown, never empty.
- **For other languages** the fact comes from the tree-sitter `ERROR` and `MISSING` nodes the engine already
  collects (`native/src/code_rule_dsl/soundness.rs`). The message is free text under the contract.

Exact messages matter for the scripts that report them, for example `SAP000: … SyntaxError: …`. The message is
not part of a finding's key, so tree-sitter's patched grammar (`native/grammars/tree-sitter-python/PATCHES.md`)
remains enough for the keys wherever it agrees with CPython on whether a file parses. On a repository whose CI
already rejects syntax errors this fact is empty; it matters on branches and fixtures.

**What happens by default.** A document with a `syntax_error` row is withheld from the policy: none of its facts
are visible, and it is incomplete in the sense of section 2. Most of the data-layer scripts do the same: a file
that raises `SyntaxError` contributes nothing.

This departs from the engine's handling of its own clauses, which run error-tolerant clauses around error regions
that cover less than half a file (`source-partial`). A Datalog policy's relations are not classified as tolerant
of error regions, and a cross-file fact from half a tree is hard to vouch for. Withholding the whole document is
the sound choice until relations are classified.

**`.on syntax_error report CODE`** also makes each parse error a finding with that code, at the error's line;
`at line 1` puts it at the top of the file, where several scripts report it (`SAP000`, `GPO000`, `LAO000`,
`RTA000`). The file's other findings stay withheld and its outcome stays `Unknown(parse)`. The finding says why.

```
.on syntax_error report SAP000 at line 1
```

**`.on input_error report CODE`** does the same for a repository file that is missing or malformed, at line 1
of that file, or with no path when the file is missing. A policy that needs more than one finding writes rules
over `input_error` instead.

## 6. `.budget`: findings that may exist

A ratchet is a ceiling on findings: up to the ceiling they are reported but do not block; past it the policy
fails. `.budget` puts the ceiling on a violation relation:

```
.budget V by code { "TP001": 12, "TP002": 0 }      // literal ceilings
.budget V by code from base                        // ceilings are the rows of base(code, n)
.budget V by code from base floor                  // and counts may not drop below them
.budget V total from base drop 10%                 // a drop of more than 10% fails too
```

`by` groups the findings: by `code`, by `severity`, by `group` (a `.group` of codes), by `path`, or `total`.

How a budget is decided:

1. **Counting.** For each group, count the distinct keys of V. Keys, not rows: two rows at one key are one
   finding, so the count does not depend on how the engine evaluated the rules. Suppressed keys still count, as
   in the engine's budget clause today (`ignore_marks`, `native/src/code_rule_dsl/model.rs:151-154`): a
   suppression comment does not lower a ceiling.
2. **Ceilings.** From the literal or from `base(group, n)`. A group with no ceiling has ceiling 0. `base` is an
   ordinary relation, usually a baseline file read with `.input` and verified with `@ed25519_ok`. A baseline that
   is missing or badly signed is not the budget's concern: it is a violation the policy writes, usually a gate.
3. **Comparison.** Over the ceiling fails. With `floor`, under it fails as well (the baseline must be lowered,
   and re-signed, when findings go away). With `drop N% [from obs]`, a fall of more than N% below `obs(group, n)`
   fails (the ceiling by default).
4. **Incomplete counts.** If any file in the budget's scope is `Unknown`, the count is a lower bound L. L over the
   ceiling still fails: the true count is at least L. Otherwise the budget is `Unknown(budget incomplete)`, never a
   pass. A drop or floor test can only fail when the count is exact.
5. **Report.** A failing group is one finding, with code `V.budget` and no path unless the budget says
   `report CODE at "path"`. Findings within budget do not block, whatever their mode; findings of a group over
   budget block under the policy's mode.

**Metric ratchets are rules, not budgets.** Some ratchets cap a measured number, not findings: the number of
suppression comments, of tests, of tracked documents. A metric is an aggregate the rules compute exactly. The
policy compares it with its ceiling in an ordinary rule:

```
over_budget(@place(BASE, 1), k, n, m) :- annotations(k, n), base(k, m), n > m.
```

A budget over findings cannot do this, because a finding count is a count of keys. In
[noqa-sec-ratchet.rr](noqa-sec-ratchet.rr) two SEC codes on one line are two annotations, but one key per code.
The earlier `.budget M by k from base` form over a metric relation is therefore dropped. Section 2 covers the
incomplete case: a metric is an aggregate, so a rule over it is withheld when its inputs are incomplete.

**Updating a baseline.** Rerail does not sign. The repository's signing script holds the human's key, and Rerail
gives it the numbers to sign: `rerail scan --json` lists each budget's counts and each `.output` metric. The
signing script then writes the baseline in the layout its policy verifies.

## 7. `.mode`, `.severity` and the verdict

- `.mode block | steer | log` is the policy's default; `.mode log CODE, …` overrides it for some codes. These are
  the engine's existing modes (`RuleMode`, `native/src/code_rule_dsl/model.rs:620-643`).
- `.severity V warning | error` is presentation only: the level in the report and in SARIF, and a grouping for
  `.budget … by severity`. It never decides blocking. In noqa-sec-ratchet every annotation is a warning and
  `.mode log SEC`; only the ratchet blocks.

A finding **blocks** when its mode is `block`, no budget admits it, and it is in the report (a gate has not
replaced it). The verdict of a policy is:

| Verdict | When |
|---|---|
| fail | some finding blocks |
| unknown | nothing blocks, and some (policy, file) or budget is `Unknown` |
| pass | otherwise |

How a run treats `unknown` depends on the reason, as today (`src/engine/audit/audit.ts:93-169`). Resource budgets
are coverage gaps and do not fail; parse failures fail, except in a repository audit. One reason is new:
incomplete run artifacts. `rerail scan --require-facts` makes them fail, which is what CI wants; without the flag
they are gaps, which is what a laptop without the artifacts wants.

## 8. What the engine needs

| Piece | Where it goes | Built on |
|---|---|---|
| Outcome per (policy, file): `Complete` or `Unknown(reason)` | the evaluation result, beside violations | blind files (`workspace/results.rs:321-371`), clause coverage (`eval/output.rs:164-227`) |
| Finding key and id from the key, not from byte spans | the consolidation step | contract §4 and §6; `eval/output.rs:80-90` today |
| `code {c}`, `tag {t}` | key extraction | the code field already in the id |
| Gate stratum, evaluated first | the Datalog planner | stratification, which the checker already computes |
| Incomplete inputs: withheld rules, Unknown places | the planner marks rules that cross `!` or an aggregate; one extra evaluation when needed | — |
| `syntax_error` and withheld documents | a native fact: a parser following CPython's grammar for Python, the error ranges elsewhere | Ruff's parser (not the Data-layer branch's CPython worker); `soundness.rs` `SyntaxErrors` |
| `.budget … from base`, `floor`, `drop` | the findings layer, over consolidated keys | `ClauseKind::Budget` (`model.rs:598-604`), with ceilings per group from a relation instead of a literal |
| Verdict and exit status | the scan CLI | `--fail-on-violations` counts every violation today (`src/cli/audit-cli.ts:256`); it should count blocking findings |
| `--require-facts`, `--facts DIR` | the scan CLI | [RUN-FACTS.md](RUN-FACTS.md) |

## 9. Prior work

- **Findings contract.** The keys, multiplicity and Unknown rules here are its sections 1–3. Section 2 of this
  page is its completeness guarantee applied to inputs, and section 6 counts keys because the contract makes
  multiplicity free. It departs in one place: the contract says a budget never changes which keys are findings.
  That sentence is about resource budgets and still holds. A `.budget` ratchet changes which findings block,
  not which exist.
- **Spike design §18 (linter architecture): the policy layer (allowlist, ratchet, signatures) sits outside the
  query.** Agrees. Gates and budgets run after the query; signatures are checked by rules, because a signature is
  a fact about a file, but what a bad one means is a directive.
- **Target architecture, point 9: one findings layer after the Datalog.** This page is that layer. Point 5,
  "stale facts must be detectable", is section 2's incomplete inputs, and RUN-FACTS.md's stale artifacts.
- **The engine's budget clause.** Reused: its lower-bound rule (`evidence.rs:240-301`) is section 6 step 4, and
  suppressed rows still count, as there. Its ceiling becomes a relation.
