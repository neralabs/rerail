# 0290: a relation whose rows name no document is cross-file

Status: built (2026-11-15). Amends the per-file memo's location columns (decision 0052) and the split (decision
0094 §6).

## Intro

The per-file memo keeps each file's file-local rows under that file's content, so an edit evaluates only the edited
file's rows again. For that, every row the chunks hand on (an export, or a file-local output) must name its document.
`file_local::locate` gives such a relation a document column (`r_at(d, …)`). Where it can't, it used to return the
relation's name, and then the memo was off for the whole run, for every policy in the batch.

On the data-layer pack over dl10 (39 policies, 8 files), four relations blocked the memo this way:

| Relation | Why it names no document |
|---|---|
| `p22_ma_branch_aware()` | nullary: `ma_branch_aware() :- ma_owner(d), ma_text(d, t), contains("BranchAwareIndexPolicy", t)`, read by `MA033(@place(…)) :- ma_owner(_), !ma_branch_aware()` |
| `p22_rr_place_MA001`, `p22_rr_place_MA026` | a `@place(p, l)` finding: a path and a line, no node |
| `p7_rr_place_CONN000` | the same |

So a one-file edit evaluated all 8 files again: 2.4 s of the edit's 7.25 s CPU were the chunks' joins.

The user chose between two ways (2026-11-15): move such relations to the cross-file part, or keep the memo for the
program but derive those relations again each run. They chose the first. It keeps one rule: a relation is either
per-file or cross-file, and there is no partial-memo state.

## Decision

`locate` moves each relation it can't locate to the cross-file part, and then locates again:

- The relation gets the qualifier `cross_file` (`file_local::CROSS_FILE`). The split holds a relation with it
  cross-file, with the cause "its rows name no document (moved by `locate`)".
- Its file-local readers follow it through the split's ordinary fixpoint, because a rule that reads a cross-file
  relation is cross-file.
- What it read is handed on in its place. Those relations are located in the next round, or moved too.
- Each round moves at least one relation that was file-local, and a moved one is never a target again, so the
  rounds end.

The qualifier is a fact of the program like `levels` (`program_facts`' `qualifier` rows), so it survives the
prepared program's image. The split memo's key includes the moved relations (`split_memo::keys`).

The `per-file memo` decision in the profile names the moved relations.

## Results (dl10, the 39 policies, debug engine, CPU binding)

The same five steps as before the change. In each step the per-policy outputs are the same files, and the pack's
421 output relations hold the same rows; only their order in `outputs.rro` differs.

| Step | Before (G instructions) | After |
|---|---|---|
| cold | 159.5 | 165.4 |
| warm | 2.80 | 2.81 |
| one file edited | 86.6 | 83.0 |
| warm again | 2.24 | 2.24 |
| the edit reverted | 84.7 | 50.3 |

The revert now reads every file from the memo, and an edit whose new content was seen before reads every file too
(58.5 G, profiled).

A new edit saves little, because the cross-file part's kept inputs (decision 0117) now take the chunks' saving. In
the profiled edit, the chunks' joins fell from 2.4 s to 1.3 s, and `stats`' `the kept inputs` rose from 0.03 s to
0.76 s (0.69 s of its own, outside reading the frames). That is final-shape-rr-align's Q20 ("taking them costs about
40 times the fallback"), not built here.

The cold run does 6 G more. It now writes every file's memo entry; that share was not measured apart.

## Tests

`file_local::tests::a_relation_naming_no_document_moves_to_the_cross_file_part` (in `rerail-device-metal`'s test
binary, which mounts `file_local`'s tests): a nullary relation read by a cross-file rule moves, with the cause, and
a second `locate` moves nothing more. The engine's memo, partial, warm and
`the_run_after_a_cold_one_writes_nothing` tests pass.
