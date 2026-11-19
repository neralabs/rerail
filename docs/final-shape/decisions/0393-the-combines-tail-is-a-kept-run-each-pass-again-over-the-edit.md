# 0393: The combine's tail is a kept run, each pass run again over the link's edit

(was 0380: that number is the child loop's, on the hub.)

Status: built (2026-11-17), measured on ep2 below. 0370's tail (work queue Q18).

## Intro

After the combine links the programs, its tail runs a dozen passes over the linked program (about 9,300 rules on
ep2): the projected inputs, the helpers retyped, the sources on request (two rounds), the requests narrowed to a
node's document and the answers where the documents are, hiding, the fact spaces' guards, and the merges (repeated
rules dropped, identical relations merged). An edit of one rule's body changes one part of the link (0373 splices it
into the last link). The tail still ran every pass over the whole program, about 0.32 s per edit.

0370 proposed per-part summaries. This builds the tail on the pipeline manager's runs instead (0281 §3, 0333). The
tail is a pipeline run, kept in the store under the kept link's family. Each pass is run again over the edit of its
last input, onto its last output.

## Decision

1. **The tail is one kept run** (`corpus::tail_kept`). Its key is the link's family, the engine build and the
   tail's shape (`Tail::shape`). The last run is opened from the image store. The link is written as an edit of
   that run's input (`manager::rebase`), and the passes run again (`Pipeline::again`). The run is kept behind the run
   (`program_text::record_runs`). The narrowing and the answers where documents are now run as passes of the same
   pipeline. They used to finish it and start a new one that kept nothing, so every pass after them ran whole.
2. **A pass runs over the edit where its decisions stand** (`ProgramPass::again`). It writes the edit onto its last
   output from what it kept of its decisions (`ProgramPass::plan`, rows kept with its step) and checks those
   decisions against the edit. Where one does not stand, it runs whole:
   - **drop repeated rules**: each removed rule taken out, each added one copied in unless an earlier rule repeats it.
     A removed rule that repeated another runs it whole.
   - **sources on request**: the edit as it is, where no rule of it reads a round's attribute or a reader's head, and
     none defines what a reader, or a reader's reader, reads down through the rules.
   - **requests narrowed**: the edit as it is, where no rule of it is a request, none defines what a request unfolds,
     and the file-local split places the documents as it did.
   - **hide**: the added rules hidden alone, where the rounds' fixpoint stands. That means no rule of the edit is a
     view, a context or the guest demand, a relation copied for a grammar, or an aggregate over a base relation. None
     is recursive, reach and each changed head's carried facts are as they were, and no added rule's variable has a
     guest's grammar. Its plan keeps the relations reached, the copies, the demand's contexts and the carried facts.
   - **fact spaces**: the added rules' guards alone. The canonical relations are made again from the congruence
     classes and must be the last run's. No rule of the edit may have a shared head, and the split's locations must
     be as they were. Its plan keeps the canonical and shared relations and the report.
   - **merge identical relations**: the added rules renamed alone, where the classes come out named as before (the
     same renames and firsts).
3. **A pass that runs whole over an edit writes its output as an edit of its last output**
   (`manager::rebase_output`, by each rule's hash and source), so the passes after it still run over an edit.
4. **What a pass wrote records its source** (`RuleBuilder::source`: hide's rules now too), so an edit's removed rule
   finds what it made (`Version::derived`).
5. **Analyses are carried lazily** (`Store::lineage`). A version made as an edit of another maintains an analysis
   asked of it from the other's copy, which is held or kept as rows by a stored run. Before, only analyses already in
   memory were carried, so a reopened run's file-local split was computed whole three times per edit.
6. **A kept run names the images it was opened from** (`Store::kept_image`) instead of copying and hashing them
   again.
7. **Names in the tail do not depend on a rule's place.** An aggregate's helper is named by its text's rank among
   the helpers (`hide::helper_names`). A request's asked variables are named by the variable (`on_request`), as
   `rr::lower` does. An edit that moves a rule renames nothing else.
8. **The congruence classes are maintained over the edit** (`merge::Congruence`, an analysis): the relations reading
   a changed head, up through the rules in topological order, are classed again and joined to the kept classes by
   signature. Where a class splits or two join, it is computed whole. Fact spaces and merge read the one analysis
   (0.022 s twice → 0.008 s once).

9. **The request record is carried over the edit with each step** (`manager::AskedByRule::onto`, added at the hub's
   merge, 9e72e45ac). The hub carries 0331's record through a pipeline by rule id, from each step's input to its
   output, and states it at the end. A step written onto its last output reads the last run's ids, not the
   input's. So each kept step keeps the record as it was after it (`Kept::asked`, the stored run's
   `pipeline_run_step_req` and `pipeline_run_step_asked` rows). A step run over the edit carries that record on: a
   rule it kept of its last output keeps its readers, and a rule it added takes its source's. This holds where the
   edit adds and removes no request's rule (`Pipeline::spares_record`). Otherwise, onto a fold, or where the last
   run kept no record, the step runs whole.
   The merge's `again` also runs whole where the edit has a rule of a request that is merged or renamed, since the
   merge states that record itself. The congruence classes take the hub's rule that a request joins only its own
   widened copies (`merge::fixed`, `filter_classes`). An edit of a request's or a copy's rules computes them whole,
   because whether a request stands apart turns on its copy's rules.
10. **A request read by another part keeps its ask** (`prune::Prune`, at the hub's merge). The hub's prune keeps a
    request's rule only where one of the readers its record names is needed. A part prepared alone (0345) does not
    hold the other parts' readers, so under `RERAIL_PREPARE=parts` it dropped every ask another part reads. Phase 0
    then never listed the library's `python/` directory, and ep2 stopped at `#include "python/security.rr": no such
    file`. A reader the version neither declares nor makes now keeps its ask. In a whole program every reader is
    declared or has rules, so prune drops what it dropped before
    (`prune::tests::a_request_read_by_another_part_keeps_its_ask`).

`RERAIL_COMBINE_TAIL=whole` runs the tail whole each run. `check` runs it kept and whole side by side, and asserts
the two outputs are one program (size and digest).

## Measured

ep2 on the m3 (300 files, 317 rules, `RERAIL_PREPARE=parts`, an edit of B101's body, profiled). Per edit:

| pass | whole | over the edit |
|---|---|---|
| hide | 0.108 s | 0.003 s |
| fact spaces | 0.094 s | 0.041 s (congruence classes 0.008 s) |
| merge identical relations | 0.045 s | 0.013 s |
| sources on request (two rounds) | 0.052 s | Delta, too small for the profile to list |
| requests narrowed | 0.027 s | 0.013 s (the split's locations) |
| keep the tail run | 0.022 s | 0.016 s |

- `combine`: 0.52 s → 0.41 s (e76, hide and fact spaces) → 0.31–0.33 s (e79, e83, e86, every pass but the
  projected inputs, the retyped helpers and the answers where documents are run over the edit);
- rows: 1,334, none differing from h1; `RERAIL_COMBINE_TAIL=check` passes (e78, e81, e85).

## What it does not do

0385 takes up each of these, and finds the run was kept once per key (every edit ran from the first run) and a
`again` that kept the last run's facts over an edit that changed them.

- The last tail run is read from the store whole (0.04–0.05 s): every step's input and output, each down to its
  bases.
- The file-local split's locations (fact spaces, the narrowing, the answers where documents are) are split again for
  the changed relations, about 0.025 s in all.
- Every ninth edit in a chain runs whole (`REBASE_DEPTH`), to bound the overlays.
- A pass's report lines printed from inside the pass (on request, narrowed) are not printed on a Delta run.
