# 0432: A phase's export is not placeless: a rule that joins it is cross-file

Status: decided (2026-11-18, while finding why the data-layer pack's edits were slow). Follows 0083 §2.3 (a
phase's exported delta is the next phase's input delta) and 0104 (phases on the run path).

## Intro

**The split.** A program runs in two parts. The file-local part runs once per chunk of files, and each file's rows
are kept in the per-file memo under the file's content: an edit of one file reads every other file's rows back from
the memo. The cross-file part runs once over all files and is maintained by its inputs' deltas (0364).

**Placeless inputs.** A file-local rule may join an input whose rows belong to no file, such as the run's options:
the same rows beside every chunk. The memo key of each file's entry then includes those rows, which is harmless when
they do not change between runs.

**The defect.** An earlier phase's exports (`struct__def`, `types__sig`, ...) were treated as placeless too. They
are every file's rows together, and an edit that adds, removes or renames a definition changes them. Each file's
memo entry, keyed by all their rows, then missed for every relation that read them, in every file:

```text
# a.py gains `def f(): ...`
struct__def("a.f", "function") is added              # one row of the export
every file's entry of every relation reading struct__def misses   # 50 files re-derived, not one
```

On the 50-file data-layer pack, an edit that added a function took 5.3 s and one that did not 1.6 s; the first
re-derived 55% of a cold run's joins.

## Decision

A phase's export is a run constant (one set of rows for the run), but not placeless. In the split
(`file_local::on_version`, `placeless_input`) only the run's options are placeless. A rule that reads a phase's
export reads an input with no location, so it is cross-file ("reads `types__sig` with no location"), and so is every
rule that reads what it derives. The cross-file part takes the export's delta as it takes any input's (0083 §2.3):
an edit costs what its delta reaches.

What stays as it was: `run_constant` still classes a phase's exports for a program that reads no document (the types
phase, P2, which runs over exports alone), for the cone of requests, and for the run's constants.

## Required behavior, asserted

- A rule that joins a phase's export is cross-file; one that joins an option stays file-local:
  `a_rule_joining_a_phase_export_is_cross_file_and_one_joining_an_option_is_not`.
- An edit's findings equal a from-scratch run's (the 50-file pack, checked by hand at 2026-11-18; the typing
  oracle tests, 30/30).

## Measured

The 57 data-layer rules over the first 50 files of corpus0, the profiling build, load 17–20 on the M5:

| | before | after |
|---|---|---|
| cold | 10.8 s | 9.2 s |
| an edit adding a function | 5.3 s | 1.7 s |
| an edit of a comment | 1.6 s | 1.4 s (a revert) |
| findings | 48 | 48 (the edit's equal a from-scratch run's) |
