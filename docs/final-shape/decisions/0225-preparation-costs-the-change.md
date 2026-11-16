# 0225: An edit costs what it changes: rules keyed by module, and preparation's analyses on the image

Status: draft (2026-11-14), for review. Revised the same day with the edit profile measured (below). Builds on
decision 0173 (a program is a flat image) and 0089 (libraries held once). Stops decision 0210's line of work at its
§12.

## Intro

Before a batch of policies runs, the engine *prepares* its program: it prunes, simplifies, lowers, pushes joins
into the file-local part, factorizes, locates and splits it. Each of those passes rewrites the program, and most of
them first ask questions of the whole program:
- which relations are constants;
- which columns hold locations;
- which relations are string filters, and which are file-local;
- the strata;
- which relations are equal (`congruence`);
- the domains of columns;
- the spans a pattern reads.

Decision 0210 made these questions cheaper. dl1 (44 data-layer policies over 2 files, cold) went from 23.64 to 20.80
G instructions in twelve steps. Each step found the same shape:
- a pass that rebuilds its answer over the whole program, every time it is asked;
- relations keyed by their names;
- an answer found by a hand-written fixpoint with its own numbering.

The first draft of this record said preparation should cost what changed. Measuring what a user waits on (an edit,
then a run again) showed something larger. Preparation is a small part of an edit. What an edit of one rule pays for
is every file's file-local part evaluated again, for every policy. So this record now asks first that **an edit cost
what it changes**: a rule's edit re-evaluates what reads that rule, and a file's edit what reads that file. The
preparation questions become rules over the image after that.

## What an edit costs (measured)

The edit profile of t34 over corpus0: 300 Python files and the 317 rules of the main pack, one store, a profiling
build, 2026-11-14. Load was 55–100 throughout, so wall times are not comparable; instructions and the shares are.
`prepare` is the main thread's preparation section.

| Step | Instructions | CPU | `prepare` | Violations |
|---|---|---|---|---|
| cold | 856 G | 88.1 s | 1.33 s | 462 |
| warm (nothing changed) | 9.9 G | 1.1 s | — | 462 |
| one file: a function appended | 89.4 G | 9.1 s | 0.18 s | 463 |
| one rule: its message changed | 96.4 G | 10.8 s | 0.75 s | 463 |
| one rule: a literal added to its body | **765 G** | **75.3 s** | 0.70 s | 463 |

The profile's spans (with the profiler on, so shares only) of the two cheap edits:

- **The file edit:** preparation 1.8% (the prepared program read back from the store in 0.05 s, no analysis run).
  The rest: opening inputs and building indexes 20%, the store writer's frames 16%, the cross-file part 28%,
  measuring sources 10%.
- **The message edit:** preparation 6.9%, and about 16% with lowering (0.48 s) and the program's other analyses.
  The rest: the store writer's frames 29% (2.7 s), parsing 14% (1.1 s of it the 317 rule texts read again), the
  per-file memo 16%, retain 10%.

So:

1. **A rule-body edit costs 89% of a cold run.** Its `local` section is 23 s, as cold's is 26 s: every file's
   file-local part is evaluated again, for all 317 policies. The per-file memo's key is a digest of the whole
   file-local program (`corpus::memo::program_digest`), so a change to one policy's rules is a miss for every file
   and every policy.
2. **A message edit prepares and parses the batch again.** A finding's message is a constant in the policy's
   `rr_diagnostic` rule, so the program's text changed and its digest with it. Nothing it evaluates changed.
3. **Preparation is 1–16% of an edit**, and its whole-program analyses run only when a policy's text changed.

The analyses themselves (`RERAIL_MEASURE`, a scratch hook: each call's wall time, nested, summed over the pool's
threads at load 55–100, so an upper bound):

| Analysis | dl1 cold: calls, ms | 50 files cold: calls, ms | message edit: calls, ms |
|---|---|---|---|
| the split (`file_local::split_in`) | 44, 1,175 | 32, 1,119 | 15, 1,735 |
| `congruence` | 24, 672 | 17, 746 | 9, 785 |
| `merge_identical` | 16, 580 | 11, 411 | 7, 491 |
| the strata | 242, 184 | 684, 418 | 666, 398 |
| `constant_relations` | 46, 151 | 30, 249 | 22, 559 |
| span patterns | 99, 33 | 324, 137 | 324, 360 |

The 50-file cold run is 28.5 s of CPU; these sum to about 3.9 s of wall on the pool.

On the message edit, the split ran whole: its memo is the run's own (0197), so a new process finds nothing in it.
The two largest splits were `whole-new` over 5,030 and 1,897 derived relations, and two more `whole-big-cone`.

What `congruence` still merges:

| | Calls | Merged nothing | Groups | Rules in them | Across policies | Within one | Plain (unprefixed) |
|---|---|---|---|---|---|---|---|
| dl1 | 24 | 16 | 384 | 1,051 | 69 | 82 | 233 |
| 50 files, main pack | 17 | 11 | 538 | 5,338 | 37 | 61 | 440 |

Most calls merge nothing. Where it does merge, the groups are mostly not library copies (5–9 groups are) but
helpers the engine made once per occurrence, over dl1:
- `rr_ctx<k>` contexts: 127 groups, the two of a rule's two aggregates in most policies (`p10_rr_ctx7`, `p10_rr_ctx8`);
- `exists_<k>`, simplify's document parts: 43;
- `rr_agg<k>` and `rr_cover<k>`: 34;
- each policy's scope relations (`p<k>_rr_scope_in`, `_drop`, `_not`), equal across policies of one scope: 34;
- pushed joins (`fl<k>_`): 20.

## Decision (proposed)

### 1. An edit is steered by the edit profile

The measure is the table above, instructions and CPU, one step at a time. A cold dl1 run stays a regression check,
not a target. No more constant-factor work on the tree's passes: 0210 ends at §12.

### 2. A rule's edit re-evaluates what reads it

The per-file memo is keyed per module (0089's modules: a library once, each policy's own part on its own), not by
the whole file-local program. A file's entry holds each module's rows under that module's own digest. An edit of one
policy's rules misses only that policy's part, and what reads it, in every file. The other parts are read from the
memo as a warm run reads them. The rule-body edit above should then cost a warm run plus one policy's local part
over 300 files.

The shape (2026-11-15):
- **A row set's key is its relation's identity.** `plan::identities` (0103, used by the cross part's kept strata,
  0077 §4) digests a relation's stratum's rules, declaration and facts with the identities of what it reads, down
  to the inputs. Over the file-local part, an export's or an output's identity changes exactly when a rule it is
  derived from does. The run's options and the earlier phases' rows are leaves of it, digested with their values.
  A base relation's rows are keyed by the relation and the columns kept.
- **One frame per file still** (0146 §1): the entry's sections are named by identity, not by relation name, and
  the memo's space is the plan without the program (the build, the grammar, the node ids, the phase, the tree
  demand, the modules). A section whose relation the run does not have is kept when the entry is written again,
  so two packs over one tree do not undo each other; one this run replaces is dropped.
- **A partial hit evaluates its missing cone.** A file whose entry lacks some identities is built, and its chunk
  runs only the strata that derive the missing relations (`plan::Hooks::only`). The relations outside the cone are
  the entry's. A semi-join's base rows (`file_local::semijoin_bases`) are taken again for the nodes all its
  exports hold, the entry's and the run's.
- **No new store, no new cache**: the entry is the frame 0146 made, with one more section.

Built (2026-11-15) as above, with one change: the sections keep their relations' names, and the entry holds the
identities it was made under in one more section (`memo::COVERED`, two words each). A relation with no rows for
a file has no section, so whether an entry holds it is read there, not from its rows. The space names the phase
and the file-local part's relations as well: nested runs with no memoised relations otherwise shared one space
and overwrote each other's frames. The kept statistics and the tree counts stay keyed by the whole program: they
are of every file together. The cross-file part is not (2026-11-15): its inputs' frames and a session's kept part
are named by the cross program alone (`retain::keys`), since an edit of a file-local rule leaves that program as it
was. Under the whole program's key every edit gave each input a frame of a new name, none found again, and an input
no memo entry holds read as changed (ep2: 574 of 2,375 cross strata ran again; a session ran all 2,715).

Measured on the edit profile (ep2, load 30–45), findings identical to the warm run's at each step:

| Step | Instructions | CPU | `local` |
|---|---|---|---|
| warm | 8.1 G | 0.7 s | — |
| one rule: a literal added to its body, before | 765 G | 75.3 s | 23 s |
| the same, now (3 of 1,266 relations missed, in every file) | 431–460 G | 45–52 s | 12–13 s |

All 300 files are partial hits, and their chunks run 15 of 4,238 rules. What is left is not the cone's rules but
the files around them (profile shares of the edit):
- **every entry written again whole** (15%): each now holds new identities, so 447 MB of rows are encoded again for
  three relations' sake;
- **guests** (24%) and parsing and each file's facts: a partial hit is built as a missed file is, though its cone
  reads a few base relations;
- **the entries' rows handed on again** (7%): through the export's set, not appended as a full hit's are.

Each is the shape this section asks for, not yet met: an entry's sections written when their rows change, and a
partial hit built only for what its cone reads. Next.

Measured again (2026-11-15, after the cross part's keying above and the hub's roll-up; ep2, B101's body edited,
finding sets identical to the unchanged run's):

| Step | Instructions | CPU | Wall | Cross strata run |
|---|---|---|---|---|
| unchanged (the whole-run cutoff) | 6.9 G | 0.8 s | 1.9 s | — |
| one rule's body, a new process | 80.2 G | 10.8 s | 13.6 s | 1 of 2,715, no input changed |
| the same in a session (dl30, 30 files) | 53.6 G | 8.1 s | — | 1 of 2,715 |

Combining the 317 programs (34 G) and preparing (12 G) are now most of an edit, in a session as in a new process:
the next lever (work queue).

### 3. A message is not the program

A finding's message, and the other texts only the output reads, are kept beside the program, not in the digest that
keys preparation and the memo. Changing one re-evaluates nothing: the findings are read again with the new text.

Done (2026-11-15) for messages:
- The lowered `rr_diagnostic` rule carries the message's values only (`"message":[v1,v2,…]`); its template is a
  `.message` setting of the clause, which the choices' key leaves out.
- The digests that key preparation, the memo and the prepared program's stored text read the rule texts with each
  message's literal replaced by its skeleton (the template's holes, no text), so an edit of the words is not a change.
- `rr_diagnostic` is never reused from the session's unchanged outputs; the writer composes each row's message from
  the template and the row's values.
- Since decision 0283 (the phased-facts merge), a finding is 0249 §6's typed relation, not a JSON text: the holes'
  values are its last columns (`findings::HOLE`), and the writer drops them once it has made the message.

Over dl1 the findings are byte-identical, and a cold run costs about 1% more instructions (the templates read
once per run, ~12 ms). The edit profile (ep2: corpus0, the main pack, one store):

| Step | Instructions | CPU |
|---|---|---|
| warm | 7.3 G | — |
| one rule's message changed, before | 96.4 G | 10.8 s |
| one rule's message changed, now | 25.5–29.5 G | 2.1–2.6 s |

The prepared program is read back from the store, and the new message is in all 195 of that rule's rows. What
remains over warm is the session's measure of the rows it holds and the run's own setup, not the edit.

### 4. The interim numberings are stand-ins

0210's numberings (`shape::Rels` and `Form`, `ColMap`, `front::Graph`, the span-pattern ids, `lower::domains`' and
`text_kind_demand`'s numberings, the keeps factors' bits) are kept only until their pass reads the image. Each goes
when its pass moves, and no new one is added. A pass that needs ids before then waits for 0173 stage 2.

### 5. The whole-program questions are rules over the image

0173 §9 and §10 name prune, magic sets, the strata and merging repeats as rules over the image's relations, kept by
the engine's maintenance. This record adds:

- **The split.** Constants, locations, filters and file-local are greatest fixpoints: a relation stays until some
  rule of it fails. Each is the complement of a least fixpoint ("leaves"), which is plain recursive Datalog with
  stratified negation. The split goes first: it is asked most, costs most, runs whole in every new process, and its
  form is the clearest. A cross-file relation's cause (`causes`) stays a Rust explanation, made for a rule the rules
  say fails.
- **`congruence`.** Partition refinement, as 0173 §10's group-by over rules' byte hashes, labels by class.
- **Span patterns, column domains, text-kind demand, the keeps factors.** Each is a fixpoint over reads.

Each is checked against the tree pass it replaces on every pack program, as 0173 stage 3 says. A pass's edit (a few
rules appended or dropped, 0173 §9's `from`) is then a delta of the analysis, and a round costs its rules. This comes
after §2 and §3: preparation is 1–16% of an edit, and §2 and §3 remove most of the edits that prepare at all.

### 6. What `congruence` merges

A helper is made once by its generator, not made per occurrence and merged back: the lowerer keys each `rr_ctx`,
`rr_cover` and `rr_agg` helper by its stem, column types and rule templates (`front::template`, variables numbered
in order) and reads an equal one again; `project_existentials` does the same for `exists_<k>`. An aggregate's helper
is shared only by clauses of the same heads: aggregate demand narrows it by its readers' contexts, so one shared
with a reader the program does not reach was computed for that reader's rows too (b110's count beside `body_len`'s,
which the pruning test caught when it was shared by all). A relation's clauses are reached, and kept, together.

Done (2026-11-15). Over dl1, findings unchanged:

| | Groups of the four generators | Groups in all | Instructions |
|---|---|---|---|
| before | 204 | 384 | 22.96 G |
| contexts, covers, `exists_` | — | — | 23.1–23.4 G |
| and aggregates, same heads | 106 (all shared) | 286 (all shared) | 21.5–21.9 G |

Load was 36–111 across these runs; instructions vary about 1.5% run to run. The saving is the aggregates': a shared
helper is evaluated once.

What remains merges across programs: a policy's helper and a library's, and policies of one scope. That is
`merge_identical`'s work after combining, not a copy to remove.

The calls that merge nothing are not dropped. On dl1 they are 17 of 24 calls but about 15 ms of `congruence`'s 350:
13 are over programs of 24–93 rules. Telling them apart beforehand would cost a mechanism for nothing.

### 7. A prepared program is kept as its image and analyses, per module

As §2 keys the memo per module, the prepared program is kept per module too: its image and its analyses'
relations. An edit of one policy prepares that policy's part and maintains the rest by delta. A cold run is an append
of everything.

### 8. Who does what

0173 stage 2 (the planner on the image: t36 item 7) belongs to the Typing-support-final-shape work, and is under way.
§2 and §3 do not wait for it: they change the memo's and the prepared program's keys, not the passes. §5–§7 begin
when stage 2 is merged, so the passes and the planner share one image and one set of ids. The order: §2 (§3 for messages and §6 done),
then the split (§5), then the other questions, then §7.

## What this is not

- Not a new evaluator: the analyses run in the engine, as rules.
- Not a cache: what is kept is the program, its modules' rows and relations derived from it, maintained as any
  relation is.
- Not a change of findings: each step is checked against what it replaces.

## Found while measuring

- **A cold run of the main pack over 50 files did not finish** (300 files ran past 7 minutes), since the hub merge
  a1f6d2f41. Format strings are guest documents now (0176), so `node`'s document column holds about 6,771 values
  for 50 files. A factorized export's part key (`fzk(d) :- node(d, v, …), text(v, _), line(v, _)`) was bounded past
  keeps' small-relation cutoff and inlined into its cross-file readers. Each reader row then enumerated its
  document's nodes again: one stratum went from 1,422 to 7.9 M rows. Fixed in 2ff9ffe8e: the factorized parts' keys
  and export demand's keys are never inlined. Now 300 files cold: 75 s wall, 847 G, and 50 files' violations
  identical to 5de418d7a's.
- **Peak memory on corpus0 is 2.5 GB**, over decision 0126's 1 GB overall budget (cold, and the edits 1.6–2.9 GB). 50
  files cold: 1.0 GB, as 5de418d7a's 991 MB. Open (O-corpus0-peak-memory).

## Open questions

- Whether the analyses run in phase 0's bootstrap run, or in a phase of their own over the image's relations.
- Whether the explanation of a cross-file relation (`causes`) can be read off the rules' failing reads, so no Rust
  explanation is left.
- What §2's per-module key is for a batch of policies that share a request cone (0089 stage 2), and for a policy
  whose rules a library's rules read.
- Which texts besides messages §3 moves out of the digest (a finding's tags, a policy's description).
