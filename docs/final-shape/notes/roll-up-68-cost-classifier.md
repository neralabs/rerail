# Roll-up 68: cost-classifier-metal-rf (480a02cc10) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at e98319cea4. Theirs: cost-classifier-metal-rf at 480a02cc10. The one merge base is
ca7e1c18a6, theirs' pin in roll-up 51. Theirs brings thirteen commits of its own and no merges (decision 0597,
contexts: code inside code analysed as a pass of its own rules):
- step 2, the ladder's rungs 2.1 to 2.12:
  - the passes are made once, and a made pass is a `.pass` with its exports;
  - a context's views are its roots' nodes, and what it holds is never copied, widened or narrowed;
  - a context reads its host's finished rows, and host, context and host go in one plan;
  - a nested annotation is a root, and a context's finding stands at its string's line;
  - a run with no root runs no rule of the context, and a context's strata run only on changes to its roots and
    reads;
  - a context's rows on Metal are the CPU's;
- step 3: one context declared in two files, and a guest's view of its host. A host's rule reading a context's
  export is no rule of the context. Python's annotation context is `python/annotation.rerail`;
- step 4, part 1: a context's guests are read by its rules alone;
- 0561 amended: a climb's reader joins the climb only when it binds nothing past the walk's row.

## Items

1. **The passes made (`passes_made`) are rerail-plan-base's.** Theirs mounts the module in rerail-planner, beside
   `answers`, for its readers there. In ours (crate split 5h, 5j) `answers` is rerail-plan-base's. The readers are
   the contexts and the corpus's kept link (rerail-rr-root's), and the combine's hiding (rerail-corpus-prep's). The
   module reads only the program image, so it sits beside `answers`. The planner's glob re-exports carry it, and the
   engine's `logic.rs` re-exports it by the planner's path, as theirs does.
2. **The combine's context passes are in ours' `corpus/base.rs`** (rerail-rr-root), where ours keeps the combine's
   tail. Theirs' one hunk to `corpus.rs` (`contexts::Passes`, `CrossingCycles` over the combined program) is in
   `tail_passes`. The other three conflicted files are ours with theirs' changes since the base: `logic.rs`'s
   re-export, and `corpus/tests.rs`' two hunks.

## Audit

`scripts/check/merge_audit.py` reports 1 finding, on theirs' side: rerail-planner's `passes_made` mount, which item 1
places in rerail-plan-base.

## Checks

On spark2, CPU, one platform:
- `cargo check --workspace --tests` is clean. The crate DAG, global-state and thread-state checks pass.
- Theirs' new tests in rerail-rr-root, rerail-planner, rerail-corpus-prep and rerail-logic pass (20). They cover the
  contexts, the climb's end, the passes made and read again, the annotations, and a program of no language.
