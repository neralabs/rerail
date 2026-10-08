# TSLint conformance harness

Scores `rerail.policy/v1` ports of the TSLint core rules against TSLint's own rule test fixtures
(`test/rules/**/*.lint` at a pinned palantir/tslint SHA). Each rule gets an objective verdict, so the
answer to "can the code-rule DSL express this rule?" rests on fixture evidence.

**Equivalence set: 163 core rules, 14 excluded, 149 scored.** The 14 pure formatter rules (align, eofline, indent,
jsdoc-format, max-line-length, no-consecutive-blank-lines, no-trailing-whitespace, one-line, semicolon,
space-before-function-paren, space-within-parens, trailing-comma, typedef-whitespace, whitespace) were removed from
the equivalence set by decision on 2026-09-23: whitespace and layout belong to a formatter, not the code-rule DSL. They
are listed in `excluded-rules.json` with that reason; the harness never evaluates them, and reports and summaries list
them separately instead of counting them as `NO_PORT`. Other TSLint "Format"-category rules (quotemark, new-parens,
arrow-parens, import-spacing, linebreak-style, no-irregular-whitespace, number-literal-format, newline-before-return)
stay in.

## Inner loop

Run the harness from the repository root. `conformance` below stands for `node --import tsx conformance/tslint/cli.ts`:

```sh
conformance fetch                  # once; sparse, shallow checkout into tmp/tslint-fixtures (idempotent)
conformance --rule no-var-keyword  # evaluate one port, print every miss/extra with line text and caret
conformance --rule quotemark --ends   # also list violations whose start matches but end differs
RERAIL_TSLINT_PORTS=<dir> conformance --rule curly   # score scratch copies of the ports instead of rules/
conformance golden                 # golden gate only: every golden rule meets its golden.json entry
conformance --label all --no-corpus # all rules; writes the consolidated report + summary (see below)
conformance --label m3 --no-corpus --compare conformance/tslint/history/summary-<date>-m2.json --delta-only
                                   # milestone: summary + delta against an earlier summary
conformance --corpus src           # every port in one workspace over src/: cold load + 1-file edits
conformance --rename-identifiers   # identifier-poisoning gate: every rule on renamed fixtures (see below)
conformance --rename-identifiers --rule class-name   # one rule's detail on renamed fixtures
conformance literals [--all]       # static scan: identifier literals a port hardcodes that only fixtures use
node --import tsx conformance/tslint/promote.ts <rule>   # promote a proven port into the golden set (see below)
node --import tsx conformance/tslint/promote.ts --check  # pack <-> golden.json <-> audit consistency
```

While you port a rule, read the fixtures in `tmp/tslint-fixtures/test/rules/<rule>/` and the implementation in
`tmp/tslint-fixtures/src/rules/<camelName>Rule.ts`. `--rule` output shows 1-based `line:col`, the TSLint options
of each variant, and the document the variant ran with.

## File layout

```
conformance/tslint/
  rules/<rule>.json                 a non-golden port: ONE rerail.policy/v1 doc (id "tslint:<rule>", scope include ["**/*"])
  rules/<rule>/<variant>.json       optional per-variant override, golden or not (see below)
  rules/<rule>.verdict.json         optional manual verdict sidecar (non-golden rules only)
  golden.json                       golden gate: expected verdict, TP and out-of-scope files per golden rule
  golden.ts golden-gate.ts          golden membership (the pack folder), golden.json, and the gate
  promote.ts                        promotion into the pack and its --check
  tslint-rules.tsv                  rule list (all 163 core rules): name, T = needs type info, F = has fixer
  excluded-rules.json               rules removed from the equivalence set: rule -> reason
  repairs.ts                        minimal edits that make invalid fixtures parse (effective verdicts)
  audit.ts                          PASS honesty classes (genuine / bounded / fixture-fit / option-baked / requires-config / poisoned)
  rename.ts poison.ts               identifier rename of fixtures and the normal-vs-renamed score comparison
  literal-scan.ts                   identifier literals in ports that only the fixtures use
  grammar-gaps.ts                   constructs checked against tsc and the engine (valid TS the grammar rejects)
  engine-probes.ts                  minimal repros of engine relation/pattern behaviour vs upstream ast-grep
  consolidated.ts                   effective verdicts, primitive histogram, type-layer needs
  port-size.ts                      port size metrics: bytes, docs, checks, relation/not/any/all nodes, rule depth
  summary.ts                        one-row-per-rule summary JSON and the `--compare` delta
  fix-check.ts                      applies a port's emitted `violation.fix` edits and compares with TSLint's `.fix` files
  fixtures.ts lint-format.ts runner.ts corpus.ts report.ts cli.ts   the harness
```

### Run output

A full run writes, relative to the repository root:

| file | where | commit? |
|---|---|---|
| `report-<date>-<label>.md` | `conformance/tslint/history/` (`tmp/rerail-conformance/` with `--delta-only`) | yes, unless `--delta-only` |
| `summary-<date>-<label>.json` | `conformance/tslint/history/` | yes |
| `delta-<date>-<label>.md` | `conformance/tslint/history/` (only with `--compare`) | yes |
| `report-<date>-<label>.json` | `tmp/rerail-conformance/` (ignored) | no: ~3.5 MB of per-fixture detail, regenerable |

Every full run also re-runs the grammar-gap and engine probes against the current engine and puts them in the report.

The **summary** has one scalar row per rule: raw and effective verdict, TP/FP/FN, engine failures, partial files,
primary primitive, eval ms, and the port's size (`port-size.ts`, summed over the default doc and every override):
`defaultBytes`, `totalBytes`, `docs`, `checks`, `relations` (= `inside` + `has` + `precedes` + `follows`), `not`,
`any`, `all` (nodes, i.e. keys, not branches) and `maxDepth` (deepest rule-object nesting; a leaf rule is 1).

**`--compare <summary.json>`** (path from the repo root or cwd) diffs the run against an earlier summary and writes
`delta-<date>-<label>.md`: totals (verdict counts, TP/FP/FN, size sums, summed eval ms), verdict changes, and every
rule whose verdict, score or shape changed, with before → after values. Eval ms alone never qualifies a rule; it is
noise. **Milestones** commit the new summary and the delta (`--compare … --delta-only`) and `git rm` the previous
summary in the same commit: only the latest summary stays in the tree (it is the base for the next compare), and
the deltas carry the history. The full `.md` goes to `tmp/`, since it nearly duplicates the previous one; no full
report is committed. `--no-write` prints the delta (or the report) instead of writing.

A **variant** is a fixture directory that contains a `tslint.json`, with its path relative to the rule directory:
`.` is the rule directory, and `single` or `tags/content` name nested directories. As in TSLint's runner, a
variant lints every `**/*.lint` beneath it. Its `tslint.json` sets the rule options.

**Overrides.** The DSL has no rule options, so write `rules/<rule>.json` for TSLint's default options and add
`rules/<rule>/<variant>.json` for each variant whose options change what is flagged. For example,
`rules/quotemark/single.json` covers `test/rules/quotemark/single/`. A variant without an override runs the default
doc. The `--corpus` run uses only the default doc. The harness flags any override that matches no variant. A golden
rule's default doc is its pack doc (see Golden set).

## Verdicts

Automatic, per rule:

| verdict | meaning |
|---|---|
| `PASS` | every evaluated fixture matches exactly: no misses, no extras, no engine failures |
| `PARTIAL` | anything else; report shows TP/FP/FN, precision/recall, end-position agreement |

`source-partial` is not an engine failure. The engine reports it for a file with syntax errors when it still ran
the AST clauses outside the error regions. Violations lost inside a region already count as FNs, so TP/FP/FN decide
the verdict. The report lists these files per rule: `Np` in the rules table, and `partial files` plus `PARTIAL`
lines in `--rule`. `source-incomplete` means a clause that is unsound under errors skipped the file, and it still
counts as an engine failure (`+Ne`).
| `COMPILE_ERROR` | a port document fails `CodeRuleDslRuntime.compile` (message shown) |
| `NO_PORT` | no default doc (pack doc or `rules/<rule>.json`) and no overrides |

Manual, from `rules/<rule>.verdict.json` (never read for a golden rule; `golden.json` carries its notes). Its verdict replaces the automatic one in the report; the automatic
metrics stay visible.

```json
{
  "verdict": "BLOCKED" | "INFEASIBLE" | "AUTO",
  "primitive": "the primary missing capability (vocabulary below)",
  "secondary": ["other capabilities the port also needs, most decisive first"],
  "needs": ["type-layer questions, for rules that need the checker"],
  "reason": "one or two sentences",
  "evidence": [{ "variant": "single", "file": "test.ts.lint", "line": 12, "why": "what this line proves" }]
}
```

- `BLOCKED`: a specific engine primitive is missing. Name it in `primitive`; the report groups rules by primitive.
- `INFEASIBLE`: out of reach for a syntactic single-file engine, e.g. the rule needs the type checker (`T` rules),
  cross-file resolution, or file bytes and encoding.
- `AUTO`: use the automatic verdict. Add the sidecar anyway when the reason is worth recording (for a PASS that is a
  stand-in, `primitive` names what a genuine port would need).
- `primitive` is the ONE capability whose absence accounts for the most unmatched violations on valid code. Invalid
  fixture syntax (`fixture-artifact`) is primary only when it is the sole blocker; `grammar-gap` counts as a real
  blocker, because real code hits it too. Everything else the reason mentions goes in `secondary`.
- Vocabulary: `constraints` (single-metavar predicate) · `metavar-compare` (ordering/equality between two captures) ·
  `utils` · `stopBy-rule` · `shared-metavar-env` · `field` · `nthChild/position` · `per-subrule-dialect` ·
  `finding-anchor:capture` (violation starts at a capture group or sub-span, i.e. what lookbehind would give) ·
  `finding-anchor:node-end` (violation starts at a node's end) · `regex-ast-context` (keep or drop regex matches by the
  enclosing AST node) · `count-aggregate` · `file-name` · `file-bytes` · `cross-file` · `scope-graph` · `control-flow` ·
  `constant-eval` · `type-checker` · `trivia/whitespace` · `jsdoc-parse` · `tslint-disable` · `fixture-artifact` ·
  `grammar-gap` · `harness-bug`.
- `needs` (type-checker rules): `symbol-resolution` (name to declaration, aliases), `overload-resolution`
  (getResolvedSignature), `expression-type` (inferred/contextual types, flags, assignability), `lib-types`
  (declarations from lib.d.ts), `compiler-options` (strictNullChecks and friends), `cross-file`. Primary is
  `type-checker` only when the cited fixture lines need `expression-type` or `overload-resolution`.

### Effective verdicts and PASS classes

`repairs.ts` holds minimal per-line edits for fixtures that tree-sitter cannot parse. Each is tagged
`fixture-artifact` (tsc rejects the line too) or `grammar-gap` (valid TypeScript the grammar rejects or misparses).
Repairs keep every expected failure's start column; the harness checks this. A rule's repaired score is reported
next to the raw one, and its **effective** verdict is `PASS (repaired fixture)` when it passes every fixture after
`fixture-artifact` repairs alone. A rule that needs a `grammar-gap` repair keeps its raw verdict.

Every effective PASS carries a class from `audit.ts`: `genuine`, `bounded` (unrolled to a named depth),
`fixture-fit` (a stand-in the fixtures happen not to break), `option-baked` (the default doc hardcodes an option list)
or `requires-config` (TSLint has no meaningful option-less behaviour: a mandatory option, or bare `true` is inert or a
blanket ban, so the default doc bakes a fixture's options or steers nothing) or `poisoned` (passes because it matches
identifiers only the fixtures use; see below). Only `genuine` ports outside `promote.ts`'s `REQUIRES_CONFIGURATION`
list (rule and TSLint evidence) can be promoted into the golden set.
- `line` is the 1-based line in the fixture source with the markup removed, as `--rule` prints it.

### Identifier poisoning

A port can pass by matching names the fixtures happen to use (`foo`, `MyClass`, `Good header`) instead of the
structure the rule is about. Two checks catch it:

- `--rename-identifiers` scores every rule on the normal and on renamed fixtures (`rename.ts`). Each user-chosen
  identifier is renamed, consistently per file, to an opaque name of the same length and character shape, so TSLint's
  expected spans stay on the same tokens; string-literal member names (`"a"()`, `o["a"]`) follow their identifier;
  string contents and comments are untouched. Protected: keywords, names declared in TypeScript's `lib.es*.d.ts`, a
  host/React list, identifier literals in the rule's TSLint source, and words and regex patterns from the variant's
  options. A rule whose TP/FP/FN moves is a poisoning candidate; rules that compare names lexically
  (`NAME_ORDER_RULES` in `poison.ts`) move by construction. Renamer problems (changed TypeScript diagnostics or engine
  error regions, names without a free replacement) are listed per file. The command exits 1 when a golden rule
  moves, and `rename.test.ts` runs the same gate for every golden rule.
- `literals` lists identifier-like tokens in each port's patterns, contexts, selectors, regexes and templates that
  also occur in the rule's fixtures, marked `suspicious` (a fixture identifier), `fixture-text` (only in fixture
  strings/comments) or `option-default` (the default doc bakes a variant's option), unless a keyword, standard-library
  name, rule-source literal, option or source-syntax word justifies them. `--all` shows the justified ones.

## Golden set

The golden set is the ports Rerail ships, disabled, in the opt-in pack
(`code-rule-dsl/rules/opt-in/tslint/<rule>.json`, rule id `tslint:<rule>`; see its README). The pack
folder decides membership and holds each golden rule's default doc, so there is no `rules/<rule>.json` or
`rules/<rule>.verdict.json` for it; its option-variant overrides stay in `rules/<rule>/`. `RERAIL_TSLINT_PORTS`
replaces the non-golden defaults, the overrides and the sidecars, never a golden default: to change one, edit the
pack doc and let the gate judge it.

The harness scores a golden default doc with its shipped scope (TypeScript sources outside build output). A fixture
file outside that scope is not evaluated and is reported as out of scope (`No` in the rules table, `outOfScope` in
the summary), never as a miss. None of the 75 golden rules loses a fixture file this way. Non-golden ports keep their
`**/*` scope.

`golden.json` has one entry per golden rule: `verdict` (`PASS`, or `PASS (repaired fixture)` when invalid fixture
syntax is the only blocker), `truePositives` (on the repaired fixtures for a repaired-fixture PASS; FP, FN and
engine failures must be 0), `outOfScope` (fixture files the shipped scope leaves out), and the notes a sidecar gave
the report: `reason`, `primitive`, `secondary`, and `evidence`, which for a repaired-fixture PASS cites the fixture
lines that are not valid TypeScript. `conformance golden` and every full run check each golden rule
against its entry and exit 1 with one line per regression; the report's Golden set section lists the result.

`node --import tsx conformance/tslint/promote.ts <rule>` promotes a port: its audit class must be `genuine` and it must not be in
`REQUIRES_CONFIGURATION`; scored with the doc as it would ship, it must reach an effective PASS and score identically
on renamed fixtures. It writes the pack doc (the port's default doc with a `Port of TSLint's ...` description prefix
and the shipped scope) and the `golden.json` entry, then prints the `git rm` for the port default and sidecar.
`--check` needs no fixtures and runs in the unit tests: the manifest lists exactly the pack's rules, every one is
audited `genuine` and needs no configuration, every pack doc is well-formed with the shipped scope, and no golden
rule keeps a port default or sidecar in `rules/`.

## Porting rules

1. Write one document per rule, plus per-variant overrides only where options differ.
2. Don't invent engine features. If the DSL can't express part of a rule, write the best partial port anyway. Then
   add a sidecar that names the missing primitive and cites the fixture lines that prove it.
3. Violations are matched on **start position** (line and UTF-16 column, exactly as TSLint reports). End positions
   are scored separately (the `end` column). The engine reports the whole matched node, so match the node that
   starts where TSLint's failure starts. For example, `pattern: "var"` matches the bare `var` keyword token.
4. Don't tune a port to one fixture's quirks. The same doc must stay sensible on real code: `--corpus <dir>`
   runs it over a real TypeScript tree (the full run defaults to Rerail's own `src/`).

## How evaluation works

- `.lint` parsing mirrors TSLint's `src/verify/{lines,parse}.ts` and `src/test.ts`. That covers `~~~ [msg]`,
  multi-line `~~~` continuations, `~nil` zero-width marks, `[0]`-style ids, `name % ('arg')` message templates,
  the first-line `[typescript]: <range>` gate, and `#if typescript` blocks. Version checks treat TypeScript as
  4.0.2, TSLint's last pinned version, so fixtures gated to older TypeScript are skipped.
- Each fixture file is evaluated in its own `CodeRuleDslWorkspace`, the same API and `tryApply`/`applyAsync` path
  the enforcer uses. Snapshot path: `<rule>/<variant>/<file>` without `.lint`, so scope globs see the real
  extension (`.ts`, `.tsx`, `.d.ts`).
- Language labels: `.tsx` → `tsx`; `.js`, `.jsx`, `.mjs` and `.cjs` → `javascript`; everything else → `typescript`.
  Code rules remain `typescript`, so they compile across TypeScript, TSX, and JavaScript dialects; JavaScript fixtures
  use the JavaScript grammar and TSLint's `jsRules` options.
- **Fixes are not scored.** When a port document declares `violation.fix` and a fixture has a TSLint `<file>.fix`,
  `--rule` applies every emitted fix the way TSLint's `Replacement.applyAll` does (by end, then start, descending)
  and prints `FIX matches` or the first differing line; the rule header shows `fixes: M/N .fix files reproduced`.
- Eval time per rule is the median over 5 runs of the summed apply time across its fixtures.
- The corpus run adds every default doc to ONE workspace, as `#prepareWorkspace` does with the catalog. It reports
  the cold full load, then 1-file edits on the median-size and the largest file. The edits are the production hot
  path, which has a <50 ms budget.

### Fixture quirks to know

- **Syntax errors.** If a fixture has a syntax error (some are deliberate, e.g. `quotemark/invalid-*`, and some
  are accidental, e.g. `quotemark/jsx-*` never closes a paren), the engine reports only the AST witnesses outside
  the error regions and marks the file `source-partial` (`PARTIAL` lines). A clause whose result an error could flip
  (`inside`/`precedes`/`follows` under a `not`, or a `not` inside one of them) skips the file with
  `source-incomplete`
  instead (`ENGINE` lines). TSLint's parser recovers and still reports, so failures inside an error region show up
  as FNs; `--rule` then also prints the score on the repaired fixture (`repairs.ts`). Before adding a repair, check
  the line with tsc: if tsc accepts it, it is a `grammar-gap`, not a `fixture-artifact`. The grammar-gap probes
  count either code as a parse error.
- **Other rules.** A variant whose `tslint.json` also enables other rules gets a note in the report.
- **`tslint:disable` comments.** A few fixtures use them (`comment-format/upper`, `max-file-line-count`). TSLint
  honors them; Rerail does not.
- **`encoding`.** Its fixtures are decoded the way TSLint does (UTF-16 BOMs). The rule is about file bytes.
- **Nesting.** `completed-docs/defaults` contains the nested variant `edge-case`, whose files are also linted by
  the parent, just as in TSLint.
- **Relations follow ast-grep 0.45.3.** `precedes`/`follows` look at the node's own siblings only (`neighbor`: the
  adjacent sibling; `end`: every sibling in that direction), never at ancestors' siblings or inside a sibling's
  subtree. Anonymous tokens (`,`, `(`, `;`, `async`, ...) are children and siblings too; `kind` cannot name them,
  `regex` can (`follows: { regex: "^,$" }`). Metavariables bind once per match: `all`, relations and `not` all see
  the bindings made so far, and a relation binds from the first related node that matches (no backtracking).
  Composite terms (`all`/`any`/`not`) run before relational ones in the same rule object. A contextual pattern
  matches its `selector` node alone; the `context` only guides parsing.
- **Engine behavior.** A sub-rule that uses a tsx-only kind such as `jsx_attribute` never matches on `.ts` files:
  it drops out of an `any` and satisfies a `not`. The rest of the rule still fires on `.ts`.
