# 0013: Findings are one stratified SQL layer over the match facts, from facts of the tape

Status: accepted
Date: 2026-10-30

## Question
06 S5: after the wave's fixpoint, how do match facts become finding keys? Which parts of W's
violation path (anchor, file gates, suppression comments) are SQL over facts, and which are
facts written before it runs?

## Decision
- **The pack's `violation` rule is RDL** (`waves::findings::FINDINGS`), emitted by the planner
  (`emit_batch`, as the wave's step is) and run once after the wave (`schedule::layer`: no
  documents' sources, no batch). It reads `match` and the wave's new `span(d, n, s, e)` (the
  terminal match's bytes) and feeds nothing back.
- **`finding(rule, code, d, line)` is one statement.** It reads `violation(r, c, d, n, b)`, a
  violation keyed by its anchored byte `b`, joined to the line holding `b`, the clause's code and
  the rule's report ceiling (`total`, a `count`). Below it are non-recursive strata: `anchored`
  (start, end, file start, or a regex group's binding), the syntax-error drop (`broken`) and
  suppression (`quiet`). This departs from "one statement" in 06's text: suppression is a negated
  relation, so it has its own stratum. The planner makes single-atom projections into views
  (`Emitted::setup`), which the layer creates before it runs.
- **What SQL cannot compute is written as facts first** (`Layer::write`), per clause and per
  document, as integer tables:
  - **The shape:** rule, code, anchor, `rerail-ignore` name, and whether `each` names a regex
    group.
  - **`group(c, d, n, s)`:** where the anchor's or `each`'s regex group binds in the node's text,
    as W's `node_groups` and `each_node_group_set` read it.
  - **The gate:** `applies(c, d, h)`, W's `run_clause` in its order: the scope, `grammar_matcher`
    (the `when` and `file` gates), the syntax gate, `FileGates`, and the text gate
    (`admits_source`); `h` comes from `honours_suppressions_at`.
  - **The error regions (`error`)** a clause reading around them (`around`) drops a witness at.
  - **Lines, and the statements that can enclose a `noqa` parent (`stmt`).** The innermost one is
    the one no deeper one holds, found through negation.
  - **The comment indexes, read off the tape:**
    - `noqa` (`exempt`, `mapped`, `directive`);
    - `nosec` (`bandit`, `after`: the first commented line from each line, so no aggregate);
    - `rerail-ignore` (`cover`).
  - **The report ceilings (`ceiling`).**
- **Families:** `finding` is partitioned by code rule. Each rule's set is a family
  (`{scope}#wave:findings:finding:<rule>`), read from the documents' cores and the shapes' digest,
  so an unchanged set cuts off. `Findings::publish` writes each (rule, document) a clause reads.
- **Coverage is Unknown only where W's clause fails:** on a file its parser did not read, or under
  syntax errors a clause is not sound around (`source-incomplete`, a limit of what was read). An
  Unknown domain publishes no keys.
- **A capture anchor must name a regex group.** A metavariable's binding is not in the match facts,
  so `Layer::new` refuses such a clause. None of the clauses the wave lowers has one; when one
  lowers, the match step records its binding from `rerail_capture`, as it records `span`.

## Prior work consulted
- **Design-index 11 (R L696-788):** count parity is a weak witness; parity is on positions. *Agrees:*
  the gate compares (rule, code, path, line) sets with W's violations, not counts.
- **Design-index 1138 (LA 6 F5, L306):** suppression comments are tape nodes. *Agrees:* the
  `noqa`, `nosec` and `rerail-ignore` indexes are read off the stored tape and joined as facts. W's
  per-witness checks become comparisons in SQL.
- **Design-index 1139 (LA 6 F6, L313):** path scoping is a caller join, not a query concern.
  *Agrees:* the scope and file gates are one `applies` table each clause's rows join; the match
  facts stay ungated.
- **Design-index 1118 (LA 1-2 L34-117):** that linter turned each spec into one
  `WITH RECURSIVE ... SELECT`, with its policy layer outside the query. *Partly differs:* the
  findings are one statement per relation in stratum order, because the suppression strata are
  negated. Like LA, the policy facts (scope, suppression settings) are written outside the
  statements.
- **Design-index 1130 (LCS 2):** that linter's report phase on 500 files went from 28.2 s to
  3.7 s through plan fixes. *Measured here:* the layer's SQL is 41 ms on 48 fixtures and 87
  clauses.

## Consequences
- `the_findings_are_ws_violations` (W evaluated one file at a time):
  - 8 fixtures: 28 keys, equal to W's, and 488 (rule, file)s Complete;
  - 48 fixtures: 121 keys, equal to W's, 2898 Complete and 30 Unknown. Each Unknown pair is one
    where W's clause fails (`source-incomplete`), and the gate checks that.
  - Running the layer again over the same matches changes no family.
- `the_findings_follow_ws_anchors_suppressions_and_ceilings`, over sources written for the cases the
  fixtures do not reach: 19 keys, equal to W's in one snapshot. The sources cover:
  - W605's regex group, one violation per binding;
  - `noqa` codes, blanket comments and file exemptions, and a parent-statement copy of E731;
  - `nosec`, `rerail-ignore`, and two E722 copies on either side of their report ceiling.
- W is the oracle one file at a time for the fixtures. A file W cannot read fails its whole
  evaluation, which then reports no other file's violations, whereas the layer decides each
  (rule, file) apart.
- A Python file's module comes from the package markers among the scope's documents, as W names
  a snapshot's (`parse_snapshot`).
