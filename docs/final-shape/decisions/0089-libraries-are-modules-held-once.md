# 0089: A library is a module, held once per run; its facts are one relation for every rule that reads it

Status: accepted (2026-11-08). It answers O135.

- **Stage 1 is built.** The full pack gives the same findings as with libraries pasted in, at a third to a half of
  the CPU. O135 has the measurements.
- **Stage 2 is built:**
  - Requests are keyed. 26 distinct requests for 29 askers.
  - A program that asks nothing is read with its libraries' declarations and kind sets only.
  - Hiding runs once over the combined program.
  - Holding a library once per bound (`.guests off`) is not built, because the pack has no such program. A run
    with mixed bounds takes the copied path.
- **Stage 3 is open.**

Stage 1 was measured over the Ruff fixtures, not `fixtures/python/calls/project`, because the pack finds nothing
in the calls project. The comparison is between shared libraries and pasted ones (`RERAIL_LIBRARIES=copied`), not
against the union of each rule run alone.

## Intro

A pack is many rule programs run together (decision 0051). Most of a program is not its own. It includes
libraries, and each library includes more:

```prolog
// packs/python/ruff-ruf034-useless-if-else.rr
#include "python/ruff.rr"        // 569 rules, with what it includes
#include "python/compare.rr"     // 485 rules
.decl cmp_same_ask(a: node, b: node)
cmp_same_ask(a, b) :- ...        // asks compare.rr a question
violation(...) :- ...            // the program's own rule
```

Today `tools::flatten` pastes each included file into the program's text. So the combined program holds one copy
of every library per program that includes it. `combine_texts` then renames each copy's relations to the
program's own (`p17_call_arg`). Much later, `simplify::merge_identical` finds the copies equal and merges them back.

The Python pack shows the cost (O135):

| | rules |
|---|---|
| programs in `packs/python` | 546 |
| rules, with libraries pasted in (the text's `:-`) | 296,298 |
| distinct library files those programs include | 60 |
| rules in those 60 files | 1,798 |
| the programs' own rules | 1,686 |

The combined program is 85 times larger than what it says. Each pass before the merge pays for the copies:

- parsing takes 9.4 s;
- `hiding::hide` takes 19.5 s;
- prune, simplify and demand take 20.6 s.

The run stops at the cap before it evaluates anything, at a 3.2 GB peak against the 1 GB budget.

The user set two requirements:

1. **Rules run once, so their facts are shared by all of the rules.** A pack is one evaluation, and each relation
   in it is derived once. It is not derived per rule and then merged.
2. **The common facts the libraries make are actually common.** A library relation is one relation in the run,
   whose rows every reader reads. It is not 538 relations that a later pass may prove equal. Its rows for a file
   also stay valid when the pack changes.

Merging after the fact meets neither requirement by construction. It works only when every copy turns out
textually equal after every earlier pass, and it pays for the copies first. This record makes sharing hold by
construction.

## What the libraries are

A survey of `docs/rule-syntax` (a dev script over the `.rr` texts, 2026-11-08) found:

- **Library names are unique.** No Python library relation (1,202 of them) is declared in two library files.
  Thirty names repeat across languages (`call_arg` in `python/calls.rr` and in `tsx/eslint-a-common.rr`). A run
  includes one language's libraries, so they never meet.
- **Libraries are closed.** A library rule reads its own file, what that file includes, `core.rr` and the engine's
  base relations. No program declares a relation of a library it includes.
- **Nine library relations are requests.** A program adds rules to them, and the library answers what was asked:

  | relation | distinct rule texts across the pack |
  |---|---|
  | `cmp_same_ask` | 7 |
  | `air_needs_guard` | 4 |
  | `seen_ask` | 4 |
  | `confusable_text`, `air_kind_ask`, `c4_builtin_ask`, `param_slot` | 3 each |
  | `sort_ask` | 2 |
  | `noqa_ask` | 1 |

  The library's answers then depend on which program asked. A program that asks nothing reads none of them.
- **Policies differ little in what they read.** No program says `.guests off`. Of the 546, 492 scope `**/*` and
  44 scope `**/*.py`, which select the same files in a Python run. Two programs exclude something.

## Decision

### 1. `#include` names a module; the run gets the graph, not pasted text

`tools::flatten` stops pasting includes. A program reaches the runner as its own text plus the modules it
includes. Each module is identified by its path and the digest of its text, and modules include modules.

`combine_texts` parses each module once per digest. The parse is kept for the process and in the store, as a
prepared program is (decisions 0073, 0074). A module included by 538 programs is parsed once, and a later run
with the same file parses it not at all.

### 2. A library's names are the run's, one per run

As `core.rr`'s names are the engine's (decision 0047), a library relation keeps its name, one per run. Only a
program's own relations are renamed (`p{i}_…`), as now.

The reader checks the two facts the survey found, so they cannot silently break:

- A relation is declared in one library module of the run. Otherwise the run is refused, naming both files.
- A program does not declare a relation of a library it includes. Otherwise the reader refuses it with the file,
  the line and the name, as 0047 refuses `core.rr`'s names.

### 3. A request makes an instance; equal requests share one

A library relation that a program adds rules to is a request. The library relations that depend on it, positively
or through negation, are its cone. Pooling every program's requests into one relation would give each reader
answers to other programs' questions. That is the union-scope sharing that the prior work measured as a loss
(ledger 9 and 15: share exact closures only).

So a cone is held once per distinct request, and no more:

- A program's requests are the canonical text of its rules for request relations, with variables renamed and
  rules sorted. Their digest is the request's key.
- Programs with the same key share one copy of the cone, named for the key (`q{k}_cmp_same`). The survey's
  largest case, `cmp_same_ask`, makes 7 copies for the pack, not one per asking program.
- A relation outside every cone is held once, whatever the programs ask.
- If no program asks, the cone holds no rows and pruning removes it, as it removes any relation that no output
  reads.

Requests are exact closures by construction. Two programs share a cone only if their questions are equal, so
neither reads the other's answers.

### 4. Hiding is computed per module, not per program

`hiding::hide` bounds a base atom to the documents a program shows (decision 0058). Its analysis carries what a
variable is from a relation's rules to its readers. A closed library's analysis depends only on the library and
on the program's bound, and the bound is one of two values (`.guests off` or not).

So a module is hidden once per bound it is read under. Its carried facts are then imported into the analysis of
each reader's own rules. For the Python pack that is one bound, so each module is hidden once. A `.guests off`
program would read the off instance of each library it includes. That instance is renamed like a request's cone.

### 5. One evaluation over the libraries once and every program's own rules

The combined program is:

- each library module once;
- each request's cone once per distinct request;
- each program's own rules.

For the Python pack that is about 3,500 rules, against 296,298. Prune, simplify, demand, lowering and the
file-local split run once over that program, as they do today over the large one. A relation is derived once in
the run, and its rows are what every reader reads. That is requirement 1.

`merge_identical` stays. It still merges relations that programs' own rules spell the same way (the converter's
copies, decision 0038), but it no longer gathers libraries back together.

Demand (decision 0048) and pruning work per relation, so a library relation's demand is every reader's. It is not
specialised per reader: two readers with the same binding pattern read one demanded relation.

A library relation is computed over every document the run reads. Each reader's own rules keep its scope, and
its findings are filtered by it (`rr_scope_drop`). The survey's scopes are all Python files, so nothing is
computed for a document that no reader wants. The profile reports, per module, the documents read against the
union of its readers' scopes, so a pack where these diverge shows it.

### 6. A file's library rows are kept under the library, not under the pack

The per-file memo (decision 0052) keys a file's rows by the digest of the whole file-local part
(`memo::program_digest(&split.local)`). Adding one rule to the pack changes that digest, so every file's library
rows are made again.

Instead, a file's entry holds its rows per unit. A unit is one library module's file-local part, or one request's
cone, or a program's own part, each under its own digest. A chunk binds the units it finds as inputs and
evaluates only the missing ones. A session does the same when it reruns what an edit changed (decisions 0063,
0077).

Then a library's facts for an unchanged file are made once across runs and packs, whichever rules read them. That
is requirement 2 across runs, not only within one.

## Stages

Each stage ends with a measurement on `fixtures/python/calls/project`, and the findings are compared with the
union of each rule's run alone. That is how serverplan's 66-rule pack was checked.

1. **Modules by reference; libraries once (§1, §2, §5).**
   - A program that extends a library gets its own copy of that library's request cone (`p{i}_`, as today). Exact
     and conservative.
   - The checks of §2 are in the reader.
   - Measured:
     - the combined program's rules;
     - CPU and allocation of each pass (decision 0084's spans);
     - the file-local split's cost against the rules, which O135 asks to be linear;
     - peak footprint;
     - findings.
   - Closes O135 if the whole pack runs within the cap and 1 GB.
2. **Requests shared by key; hiding per module (§3, §4).**
   - Measured: copies per request relation, the hiding span, findings.
3. **Per-unit memo (§6).**
   - Measured: a second run after adding one rule to the pack. Every unchanged file's library rows should be read,
     not made. Measure its CPU against a cold run's.

## Measured: the host's part of a cold run

Reading and combining the programs, the guests and preparing the combined program are the work of a cold run that
is not joins. On the Python pack (534 programs over ruff's 486 fixture files) they were 58 s of CPU. They are now
34 s, with the same findings:

- **Each program's text is read once** (`rr::read_text`). A run asked every text for its modules and then for its
  program, and each ask made the text's tape and resolved its includes again. The text read once answers both. The
  library files' statements are kept shared, and a program read with only its libraries' declarations copies just
  those, not every clause to drop it after.
- **Congruence signs a rule by a template made once** (`simplify::congruence`). Each round of refining the classes
  of identical relations printed every rule's canonical text under that round's labels. A rule's text is fixed but
  for the labels of the relations it reads. So the text is made once with each relation a placeholder, and a round
  compares the template with those labels. `congruence_by_templates_is_congruence_by_texts` checks the partition
  against the texts. This is used three times: the fact spaces, `simplify` and the merge after the joins are pushed.
- **No pass scans every rule once per relation.** That held in lowering's span patterns, in the climbs rewritten as
  `within`, in locating the chunks' rows (a target's readers are counted once and kept as rules move to `<rel>_at`)
  and in the scope guards (one pass, each rule's groups in order).
- **A cycle summary of a component that reads no member of itself settles in one round** (`front::cycles`). Its
  rules read only relations below it, so a second round only repeated the first.
- **A guest's tree is kept by grammar and text** (`guests::parse`). A pack's guests repeat their texts (one
  annotation in many files, the same guests in every chunk). Store lookups fell from 40088 to 1536, and each
  distinct text is restored and digested once. The facts are a function of the grammar and the text alone. The
  memo starts again past 8 MB of texts.
- **A dialect is made once per thread** for the hosts' numberings. Making one walks the grammar's kinds, and every
  host a guest is made in made its own.
- **Hiding's maps are hashed, and the facts encoding is one pass.** A string with nothing to escape is quoted whole.
  Storing the prepared program has a span of its own; it was unnamed CPU in `stats` before.

Paired cold runs, the base binary (`final-rerail-shape-matrix-offload` at 1e51c1f19) against this one (CPU is
user + system; load 60 to 120, so CPU differences under 15% are noise; the instruction counts are steadier):

```
                                                     base      now   (s CPU)
combine phase                                       27.77    15.87
stats phase                                         29.94    18.58
  parse each program (534)                          12.63     6.91
  the libraries, once                                1.65     1.09
  bound the combined program (hiding)                6.67     4.00
  fact spaces                                        3.06     1.33
  keep the combined program in the store             2.56     1.56
  prune, simplify, demand                            4.77     1.85
  push the file-local joins into the chunks          6.53     5.37
  locate the chunks' rows                            4.27     1.04
  merge identical relations again                    2.52     0.87
  keep the prepared program in the store         (unnamed)    4.84
  stats, own                                         6.98     0.23
guests, own                                          7.88     0.96
  tape cache lookups                         40088 calls   1536 calls
process instructions                             2680 G    2425 G   (both sampled; 2377 G unsampled)
```

A warm run is the run kept whole: 41.0 G instructions before, 40.3 G now, and none of the above is paid. A one-file
edit reads the combined and prepared programs from the store: 622 G instructions before, 611 G now (combine 2.2 to
2.0 s, stats 6.2 to 5.9 s, guests 2.5 to 2.1 s CPU). The edit's cost is elsewhere (cross, local), not in this list.

Left as they were: the file-local split (`typing::file_local::split_explained`, run by the pushed joins, `locate`,
`programs` and the guards; work another change is making), phase 0's run of `bootstrap.rr` (its worker parsing the
library's tapes and the statistics of its documents, about 2 to 4 s of a cold run; the graphs are kept and checked by
a `stat` later), the lowering of each program's own rules, and printing the prepared program's parts as documents.

## What this does not do

- **It does not change what a rule means.** A program sees the same relations with the same rows. Only the number
  of copies changes. The survey's checks (§2) become refusals, so a pack where they fail is refused, not
  silently merged.
- **It does not share a library's rows across languages.** A run has one language's libraries.
- **It does not address library documents** (typeshed, installed packages). Those are documents whose facts live in
  0083's phases. The libraries here are rule modules. The two meet only in that both are held once and memoised
  by content.
- **Per-program scope is not part of an instance's key.** If a pack's policies scope disjoint trees, a library is
  computed over their union. §5's profile line shows when this happens. Keying by scope is the response if it is
  measured to matter.

## Prior work consulted

- **Lessons ledger 9 and 15** (union sharing loses; share only exact closures with two or more consumers; partial
  sharing is slower than none). A library outside every request is an exact closure: all its consumers read the
  same relation over the same documents. A request's cone is not exact across programs. That is why §3 keys it by
  the request instead of pooling the requests.
- **Ledger 13** (extract demand-driven). Demand and pruning stay per relation, so a library relation that no
  reader needs is not made. Holding a library once never makes more of it than some reader asks for.
- **Ledger 16** (the cost is repeated scans of the same broad relations). The 538 copies of a library made the
  same relations 538 times before the merge. Holding them once removes that repetition at its source.
- **ast-index-spike design index §13 (sharing and dedup):**
  - S143: anchors share 4.0x across 1,970 queries;
  - S149 and S176: a union scoped to the whole corpus lost (0.86x, 7.27x scope inflation), while per-group sharing
    gained 1.45x.

  The modules here are the per-group case. Each group is the set of readers of one library, or of one request.
- **Design index §12, structural dedup** (D S068: 30% of the generated SQL was duplicated, and capture names never
  reached SQL, so equal statements could be found). Here the duplication is removed before any pass runs, instead
  of being found after.
- **Decisions:**
  - 0047: `core.rr` once per run, the precedent for §2;
  - 0038: `merge_identical`, which stays for programs' own copies;
  - 0051: one run for all rules;
  - 0052 and 0063: the per-file memo and session reruns, which §6 refines;
  - 0058: hiding, which §4 moves to per module;
  - 0073 and 0074: prepared programs in the store, the model for keeping a module's parse;
  - 0083: phases over fact spaces, which this does not change.
