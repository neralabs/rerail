# 0720. A pack's `.rerail` rules take options, offer fixes and name their port's suppression comments; a pack states the core its rules need

Status: built. Builds on 0130 (`.test` blocks), 0168 (a batch's policies as one pack), 0249 (findings as typed
relations) and the pack format's "Rules written in .rerail".

## Intro

Each pack's JSON rules are being replaced by hand translations to `.rerail`, checked to report the same findings. A
pack could ship `.rerail` rules, but the product refused three things the translations use, so converting a pack would
take away what its users have:

- **Options.** A translation reads `option("name", v)` and `option_item("name", i, v)` (core.rerail). The product
  refused any `config` for a `.rerail` rule, and the engine took one set of option values a run, not one per rule.
- **Fixes.** A translation carries `fix [safe] F "title"` lines and rules ending in `=> edits`. The engine lowered
  them to nothing, and the product refused a rule with a `fix` line.
- **Suppression comments.** A translation of a port carries `suppress TOOL "rule"` lines. Check 2 refused them on
  every `.rerail` rule, and set-up's reading of a repository's lint config saw no `.rerail` rule as a port. (The
  engine applying the lines is separate work.)

And a pack whose `.rerail` rules need engine work newer than 0.2.1 had no way to say so: `RERAIL_DOCUMENTS_SINCE` was
the one floor the release scripts knew for them.

## Decision

1. **A rule's options are the names it reads; nothing declares them.** `describeRr` lists the names the rule's
   `option("name", …)` and `option_item("name", …)` atoms read, its includes' among them. The 123 hand translations
   that read options are written so: none declares one, and each says what an option left unset means
   (`!option("option", _)`). A value comes from where a JSON rule's does: a rule setting's `config`, a pack's `config`
   in `.rerail/packs.json`, a port's options mapped from the repository's linter config. `configureBundledCodeRule`
   refuses a name the rule does not read, as it refuses an option a JSON rule does not declare, and a value that is no
   fact (text, a number, true, false, null, or a list of them). The rule states no type and no allowed values, so
   none is checked: a value the rule does not expect is the rule's to give a meaning. That is less than a JSON rule's
   declaration checks, on purpose: the alternative was a JSON declaration in every rule file, which the first draft of
   this decision had (the converter's `.options {…}` line) and no hand-written rule carries. The rule's file is not
   rewritten: the checkout catalog keeps the values with the rule (`RerailRuleEntry.config`, and in its line of the
   catalog's version, so a changed value is a new catalog), and every run hands them over.
2. **The values reach the engine per policy, and become `option` facts.** `runRr` takes `options` by policy path.
   Policies run together read one set of `option` facts (a pack's programs share the relation, 0168), so the host
   (`rr_host::run`) runs together only the policies given the same values, each such group into its own directory
   with its values beside the run's own settings, in the one options object the run reads; `facts::repository` makes
   the `option` and `option_item` rows from it as before. Policies given no value keep running as one pack; each other
   set of values adds a run. No option may take the name of a run setting (`RUN_SETTINGS`: `inputFiles`, `dialect`,
   `toolCalls`, …): `describeRr` refuses the file, since the two share the object. The converter's generated files
   still carry `.options {…}` defaults, which the engine reads as before; no rule a pack ships has one, and the
   product reads none.
3. **A fix is lowered to rows of its edits, and composed into the JSON rule's fix.** For each `fix` line of a
   `.violation` V and each rule of its relation F in the same file, each ``replace(target, `text`)`` edit becomes an
   output relation (`lower::fixes`, `rr_fixedit_<digest>`), joined with V's rows through F's head: per finding, the
   finding's columns, the target, and the value of each `$x` placeholder of the text, after constant columns naming
   the fix, rule and edit. Column types are those of the atoms that bind each variable; an edit whose target or
   placeholder no positive atom types, or whose rule does not lower, gives no rows, and the policy's own rules are
   lowered as before. The host reads the rows back with the findings (`rr_host::fixes`): a finding takes its first
   fix, by `fix` line then by rule, whose every edit has one row; each edit's text is filled from the file (a node's
   source, a symbol, a number) and padded where it would join an identifier, and the edits, all in the finding's
   file and none overlapping, are composed into one replacement `{span, text}`. That is the shape a JSON rule's
   violation carries (`CodeRuleViolationFix`): the audit records it and the steer shows it as a suggested fix, which
   Rerail never applies. `delete`, `insert_before`, `insert_after` and a library's `.edit` relation are not run
   (their separators, lines and expansion are the applier's, not built): such a fix is not offered, not refused.
4. **A pack's rule names only the comments of the tool it ports.** A rule is a port when the pack's
   `conformance.json` has a port entry whose `policy` is its file. Check 2 lets a port's violations carry `suppress`
   lines for its tool's comments alone (ESLint's family: `eslint`, `oxlint`, `biome`; Ruff: `ruff`, `noqa`, `flake8`;
   Bandit: `nosec`; Clippy, PHPStan, Psalm, PHP_CodeSniffer and SQLFluff: their own), and refuses them on a rule with
   no port entry. Check 4 holds a `.rerail` port to the JSON port's suppression conditions (the upstream rule named on
   every violation, Oxlint's beside ESLint's, Biome's same rules), read from `describeRr`'s `violations`. Set-up's
   lint config reads a `.rerail` rule's `suppress` lines as a JSON port's `violation.suppress`
   (`RERAIL_SUPPRESS_KEYS`), so the repository's ESLint config turns it on or off and gives it options.
5. **A pack states the core its `.rerail` rules need in `package.json`, `rerail.core`.** It is a floor beside the
   ones `scripts/release/core.mjs` maps (`coreFloor`'s `stated`): once core is at that release, check 5 fails a range
   that admits an older core and the Release pull request raises it; before, the range stays, so a checkout's own
   core still loads the pack. Core never reads the field. Fixes and `suppress` lines first run in 0.2.3
   (`RERAIL_RULE_FEATURES_SINCE`), so check 5 fails a pack whose rules use either and whose `rerail.core` is
   missing or older. A rule that reads options needs no newer core: an older one runs it with none set.

`fixtures/packs/rr-curly` ships the translation of ESLint's curly as it is written and proves the path end
to end (`test/engine/catalog/rerail-rules/rerail-rule-options-fixes.test.ts`); `rr_host::tests` proves option values
per policy and a two-edit fix in the engine.
