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
- **The layer is one statement** (`emit::single`): `INSERT OR IGNORE INTO new_finding WITH ...
  SELECT`. The planner emits the rule's relations as non-recursive strata. `single` folds every
  relation below `finding` into a materialized common table expression, the union of its steps
  in stratum order, with the planner's views folded in too. It refuses a program with recursion or
  lattices, which need the fixpoint. The relations are:
  - `anchored`: start, end, file start, or a capture's binding;
  - `broken`: the syntax-error drop;
  - `quiet`: suppression;
  - `violation`: one per anchored byte;
  - `total`: a `count` for the report ceiling.

  The run is one wave of the scheduler's loop, which records the families.
- **What SQL cannot compute is written as facts first** (`Layer::write`), per clause and per
  document, as integer tables:
  - **The shape:** rule, code, anchor, `rerail-ignore` name, and whether `each` names a regex
    group.
  - **`group(c, d, n, s)`:** where the anchor's capture binds at a matched root. A same-node regex
    group is read in the node's text, as W's `node_groups` and `each_node_group_set` read it. A
    metavariable is read from W's matcher run at the root the wave matched (`match_root`, within
    the rule's visit budget): its single capture's start, or a multi-capture's first node's. The
    lowered root programs test labels only, so their batch's captures hold the root alone
    (`match`); a budget W's matcher exhausts leaves the domain `Unknown`.
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
- **Coverage is Unknown only on a limit:**
  - W's reading limit, where W's clause fails too (`source-incomplete`): a file whose error regions
    cover at least `UNREAD_PERCENT` of it, or syntax errors under a clause not sound around them;
  - the visit budget, where a capture's binding needs W's matcher.

  An Unknown domain publishes no keys. The gate checks each `source-incomplete` domain against a
  W failure.
  Confirmed by the user (2026-10-30): W's read cutoff is the limit S5's "Unknown only on a
  budget or limit" allows. The layer is wired in at S6 (`slice::audit`, `slice::apply`), not in
  S5.
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
  - 48 fixtures: 121 keys, equal to W's over every (rule, file), the Unknown ones included; 2898
    Complete and 30 Unknown. Each Unknown pair is one where W's clause fails (`source-incomplete`)
    and reports no violation, and the gate checks both.
  - Running the layer again over the same matches changes no family.
- `the_findings_follow_ws_anchors_suppressions_and_ceilings`, over sources written for the cases the
  fixtures do not reach, with W reading all the files in one snapshot. The sources cover:
  - W605's regex group, one violation per binding;
  - `noqa` codes, blanket comments and file exemptions, and a parent-statement copy of E731;
  - `nosec`, `rerail-ignore`, and two E722 copies on either side of their report ceiling;
  - metavariable anchors: a single capture, a multi-line multi-capture, and one an `any` may
    leave unbound (the witness span).

  With these the gate gives 35 keys, equal to W's.
- W is the oracle one file at a time for the fixtures. A file W cannot read fails its whole
  evaluation, which then reports no other file's violations, whereas the layer decides each
  (rule, file) apart.
- A Python file's module comes from the package markers among the scope's documents, as W names
  a snapshot's (`parse_snapshot`).
