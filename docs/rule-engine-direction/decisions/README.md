# Engine decision records

One file per decision: `NNNN-short-name.md`, numbered in order. A record is never edited to reverse it. A new record supersedes it, and the old one gets a `Superseded by` line.

The protocol in `../prior-work/README.md` applies: no record without prior-work citations.

## Template

```markdown
# NNNN: <decision>

Status: proposed | accepted | superseded by NNNN
Date: YYYY-MM-DD

## Question
What is being decided, and which targets it affects (per-edit, full repo in 1–2 s, outer SQL query).

## Prior work consulted
- ast-index-spike design index, topic <n>: <conclusion> (source: <file> L<a>-<b>)
- ast-index-spike history index, topic <n> / trap T<n> / dead end D<n>: ...
- lessons-ledger.md L<n>: ...
Agreement: agrees | departs, because ...

## Decision
...

## Evidence
Measurements on the gate workloads (README.md, "Measurement gate"): CPU, wall time, RSS, findings parity under findings-contract.md, with compile time reported separately. Proxy metrics only as diagnostics.

## Consequences
What becomes easier or harder; what is now forbidden; what would make us revisit it.
```

## Index

| # | Decision | Status |
|---|---|---|
| 0001 | Findings are a set under the loose contract (`../findings-contract.md`) | proposed |
| 0002 | Rule documents carry no SQL or code; `relational` is frozen, then removed (`../authored-sql.md`) | proposed |
| 0003 | Tape v9np: a structure-only, regular, column-stored tape with folded kind codes, block-based starts and tail-relative ends, no stored parent, 0.93–1.01× v7 in size; Blade equals tree-sitter-php (`../research/tape-regularity.md`, `../research/tape-v8-measurements.md`) | proposed, pending 4 pre-lock measurements |
| 0004 | The transducer IR is the concrete execution IR; everything decidable per document lowers to it, and SQL composes above it (amended by 0005: local structure only) (`../research/transducer-ir.md`, `../research/sql-vs-transducer.md`) | proposed |
| 0005 | Named derived facts: shared and relational intermediates are named facts defined separately from rules (`rerail.facts/v1`), compiled to SQL, cached by digest and invalidated by dependency cone; keyed sets and bottom-up folds withdrawn from the IR; amends 0004 (`0005-named-derived-facts.md`, `../research/derived-facts.md`) | proposed |
| 0006 | The anchor index is always built, never optional; performance comparisons report execution, with construction separate (`0006-anchor-index-always-built.md`) | accepted |
