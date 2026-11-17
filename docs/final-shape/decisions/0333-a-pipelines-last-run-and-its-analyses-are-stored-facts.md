# 0333: A pipeline's last run and its analyses are stored facts, so a new process reruns on an edit

Status: accepted (2026-11-17, the user's choice: store the analyses as facts). Step P of 0281's path.
Builds on 0281 §2 (analyses cached per version, carried and maintained across a `Delta`), §3 (`Pipeline::again`
reruns on an edit of the last input), §6 (steps stored as facts in 0103's two-rule form), 0103 (caches are two
rules), 0225 §5 (analyses as rules over the image).

## Intro

An edit of one policy should cost that policy. 0281 §3 gives that within one process: `Pipeline::again(run, v)`
reruns each rule pass on the edit's rules and carries each analysis across the edit. But the last run (`Run`) and
its analyses are values in memory, and ep2, the edit profile, runs its edit in a new process. There, every pass runs
whole again, over about 10,000 rules (`combine` and `prepare`, 1.33 s on ep2).

This decision keeps both in the store, as facts:

- **The run.** Each step's pass, its input and output versions, and for a rule pass what a rerun compares: its
  selection, the names of the analyses it asked for, and `own_end`.
- **The analyses.** Each analysis a step's input held, as rows.

A new process opens the last run from the store and runs `again` over an edit of its input.

## Decision

1. **The input is an edit of the last input (rebase).** `Delta::of` reads an edit only when the new input is an
   overlay of the last one. `manager::rebase(last, new)` makes it one: `new`'s rules and header written onto
   `last`, the rules matched by hash. Rules in both are kept as `last`'s; the others are removed or added; the
   header is set where it differs. A rebase that would add or remove more than half the rules is not made, and the
   pipeline runs from scratch.
2. **A run is stored by its pipeline's name and the run that made it.** The store's aliases cannot be pointed
   elsewhere, so there is no "last run" entry to move. The corpus run's manifest names one digest for the runs of
   its pipelines, and each run is kept under `run_key(name, digest)`. The next run reads the root's newest manifest
   and opens its pipelines' runs by those keys. `pipeline_run_step(run, k, …)` holds the steps (0103's two rules:
   the key, and the run it names). A step's versions are kept as they were made:
   - a base, as its image (`store_image`);
   - an overlay, as its delta over the step's input, reopened with `Version::over`;
   - unchanged, as nothing.

   So rule ids, and the `from` links a rerun follows, are the same in the next process. A step whose versions the
   store lost ends the run there: the steps after it run whole.
3. **A rerun compares analyses by name.** A stored rule pass names the analyses it asked for. On the rerun, the
   pass's `begin` asks for them again on the new input; each stored name is matched to what it asked by name, and
   compared on both inputs. A name it did not ask for again means the pass runs whole.
4. **An analysis is stored as rows.** `Analysis` gains `VERSION`, `rows(&self, v)` and `of_rows(v, rows)` (by
   default none: it is not stored, and is computed again). A step input's analyses are kept as
   `analysis_key(run, k, name, version)` and `analysis_row(…)`. A version reopened from a stored run looks an
   analysis up there before computing it. Its ids are the image's, which reopening keeps.
5. **Where a run is kept.** The combine's tail and the preparation pipeline are kept (`keeping`). When the run
   ends, each is stored under its name, and the next run opens it and rebases its input onto it. Every other step
   on the path is an edit of its input already (0281 7a-1).
6. **Measured once.** ep2, at the end, after these are built. A pass that reads the whole program in its own body
   (`Hide`'s fixpoint, the fact spaces, `Prune`) still runs whole. Each such pass is made to rerun on the delta
   only where that measurement shows it is worth it.

## What it costs

- **Storage.** A run's images are mostly shared: an overlay's delta is the size of its edit. The analyses are rows
  over relation ids: `About` per relation, `Strata`'s components and edges, `Needed` and `Components` sets, the
  split, the fields and `FileLocal`.
- **A rebase** hashes the rules of both versions once, O(rules).
- **No new cache form.** A run is stored as facts in 0103's two-rule form, next to the steps of 0281 §6.

## As built (2026-11-17)

- **Rebase.** `manager::rebase(last, new)`: rules matched by `RuleRef::hash` (by text, so two processes' images
  agree), the header by `Builder::set_header`, written as an overlay of `last` (`finish_overlay`). `None` past half
  the rules, or when `last` is `REBASE_DEPTH` (8) overlays deep.
- **The run's parts.** `Run::parts` lists every version once, from its bases up (`Version::layer`: a base's image, an
  overlay's delta, a view's left-out rules, reopened by `Version::over` and `Version::view_without`), with the steps
  (`StoredStep`, `StoredRule`) and each step input's analyses as rows. `Run::assemble` rebuilds the run;
  `RuleKept::named` holds the stored analyses' names, which `same_named` matches to what the pass asks again.
- **The store.** `program_text::StoredRuns` keeps a run as one source of facts under its key (`pipeline_run`,
  `pipeline_run_version`, `pipeline_run_step`, `pipeline_run_decision`, `pipeline_run_analysis`), the versions'
  images and deltas beside it. Opened, each version is handed its analyses' rows (`Store::read_stored`); `Cx::get`
  reads one there before computing it (a debug build checks it against a computation).
- **Which analyses are stored.** `Strata`, `Needed`, `Components`, `FileLocal`. `About` and `FieldsData` are read off
  the version's own rows, and are not. The split (`answers::Split`) is not yet.
- **Finding the last run.** An alias cannot be pointed elsewhere, and the memo plan is the same across an edit, so a
  run is not keyed by its plan. The run's manifest names one digest for its pipelines' runs (of the first pipeline's
  input, `runs_digest`) in its `also` list. The next run reads the root's newest manifest's list
  (`ContentStore::corpus_run_also`) and opens each pipeline's run under `run_key(name, digest)`.
- **Where.** `preparation::front` and `preparation::run` are kept (`kept`): run `again` over the input rebased onto
  the last run's, else from scratch. Both runs are stored behind the run, when its store is durable. The combine's
  tail is not kept yet: its passes read no analysis, so a stored run gives it nothing until they rerun on the delta.
- **What a new process gains now.** The analyses carried across the edit (`FileLocal`'s `maintain`, the file-local
  passes' split), and the last input's analyses read rather than computed. No rule pass on these pipelines is local
  (`WHOLE`), so each runs whole. Making the large passes rerun on the delta is the next part, chosen by the ep2
  measurement.
- **Test.** `a_stored_run_reruns_on_an_edit_in_a_new_process`: a run kept, the store reopened, the edit rebased onto
  the stored input, the rule pass visiting the edit's added rules alone, the input's `Needed` read back, the output a
  run from scratch's.
