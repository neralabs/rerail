# 0117: A relation carries its delta; maintenance is the core's, driven by the deltas that are not empty

Status: accepted (2026-11-11). Steps 1–4 are built (below).

## Intro

**The question (the user).** Should maintenance move into the core, and be managed with subsidiary, optimized,
less generic data structures?

**The answer in short.**
- **Yes. Maintenance becomes the core's driver for maintained strata, not a second evaluator.** It uses the same join
  specs, planner, plans (now kept across processes, 0116 step 2) and native strata as every run.
- **The subsidiary structure is a relation's delta, held beside the relation.** It has three parts:
  - the rows dead in place;
  - the rows appended;
  - each document's digest of its rows.

  "Is this delta empty?" is then a field read, not a query, and so is "which documents changed?".
- **What it removes is work on what did not change.** An edit's run today:
  - rebuilds every input from every file's memo entry (19 unchanged entries, 135 MB);
  - hashes every input by document;
  - rewrites the whole program for maintenance (203,141 rules, 46,494 strata, 39,640 of them over empty deltas);
  - reads back the old rows of every relation of a stratum that runs.

  With the delta on the relation, each of these touches only what changed.

**How to read this record.**
1. The structure.
2. The driver.
3. Why each is exact.
4. Prior art and the spike's lessons.
5. The order of work, with the measured targets from 0116.

## 1. The structure: a relation with its delta beside it

A maintained relation is its **kept rows**, read in place from the frames (0099, 0104), plus side structures:

| Side structure | What it holds | What it answers |
|---|---|---|
| Dead rows | Bit per kept row, set by the run (0090 step 4's survivors) | The relation's rows now, without a copy |
| Appended rows | Columns of the rows the run added | The insert delta, `r__plus` |
| Removed rows | The dead rows, as a view over the kept columns | The delete delta, `r__minus` |
| Document digests | Per document: the sum of its rows' hashes, and their count (`retain::DocDigests`) | Which documents' rows changed; the relation's digest (`digest_of_docs`) |

These are the structures a run already has, but in separate places:
- the session's counted exports (0096 step 3);
- the kept part's input documents (`Retained::input_docs`);
- the survivors' dead rows (`Start::Base`);
- the `__plus` and `__minus` relations, materialized as relations of their own.

Here they are one value with one owner, so every step reads the delta where it is and never rebuilds it.

**A delta is O(changed rows) to make and to test.** Its size, and whether it is empty, are counts on the
structure. The relation's digest after an edit is the kept digest, less the changed documents' digests, plus their
new ones. The digest is a sum over rows, so this is exact.

## 2. The driver: strata in order, with the set of non-empty deltas

0101 §1 already skips a maintained stratum whose every delta is empty, as the run reaches it. 0116 measured what the
skip leaves behind: the whole maintenance program is still built, keyed, split into strata and passed over, about
3 s of a 20 s edit. The driver moves that work behind the skip:

1. **Walk the base program's strata in order**, with the set of relations whose delta is not empty. The inputs'
   deltas seed the set (step 1, below).
2. **A stratum that reads nothing in the set is not touched.** It is not rewritten, planned, opened or read back.
   Its relations are their kept rows, with empty deltas.
3. **A stratum that reads a relation in the set is rewritten there and then**, for the positions whose delta is not
   empty (`maintain::rewrite` of that stratum alone, under counting by default, delete and rederive for recursion).
   It is planned with the kept plans' delta-first orders (`first`, 0116 step 2), and run. Its relations' deltas are
   then the side structures above, and are added to the set when they are not empty.
4. **The whole-program passes are the base program's**, made once and kept with the prepared program (0101 §2's
   "better" form). Strata, first and last reads, loads and rule numbers are computed once, not each run.

**Why it is exact.** Every rule the rewrite writes for a stratum has a delta at the position that changed (0077
§3). Over empty deltas it derives nothing, and its survivors are the kept rows. A stratum's rewrite depends only on
which of its reads changed, so rewriting it when it is reached gives the same rules as rewriting the whole program
first, and then keeping that stratum's.

## 3. Step 1: an input is its kept rows plus the changed documents' rows

An input of the cross part is an export or a base relation: rows the file-local part makes for each file, kept in
that file's memo entry. Today a command-line run rebuilds each input from every file's entry. The session's shortcut
(the counted exports, 0096 step 3) lives only in the process's memory, so a new process never has it.

The kept part's frames already hold each input as the last run had it, with its document digests. So:

- **An unchanged file's memo hit is not replayed.** Its export and base rows are already in the kept input. Its
  lines (the file-local outputs) are still taken from its entry, decoded alone.
- **Each input is its kept rows with the changed documents' rows dead, plus the rows of the files this run
  evaluated.** The changed documents are those the statistics keeper counted in or out.
- **The input delta is those rows: gone and new.** It is handed to the maintenance rewrite as given (0090 §2), with
  no hashing of unchanged documents.

**When it is exact.** A row belongs to the document its first column names (`memo::document_of`), the rule that
`retain::document_delta` and `input_delta::delta` already use. It is exact for an input none of whose changed
files' rows name another document. If any row does (a value document's row, which several files hold), or if the
kept part is missing or was not written by the run the keeper counted, the run takes today's route for that run:
every hit is held from its entry, as the session's "hold after all" does. So it never answers wrong, and falls back
to the measured cost.

**Checked by `RERAIL_CROSS_VERIFY`.** It builds both forms and compares them row by row.

## 4. Prior art, and the spike's lessons

- **DRed and counting** (Gupta, Mumick and Subrahmanian, *Maintaining Views Incrementally*, SIGMOD 1993): the
  rewrite rules this engine already uses (0077, 0090). Unchanged.
- **Differential dataflow's arrangements** (McSherry, Murray, Isaacs and Isard, CIDR 2013): an indexed collection
  shared by its readers, plus batches of changes, each batch merged in when it is small. The relation here is the
  same idea, at the run's grain: kept, indexed rows plus one batch per run. Unlike differential dataflow, there is
  no timestamp lattice. A run is one version.
- **DBSP's Z-sets** (Budiu, Chajed, McSherry, Ryzhyk and Tannen, VLDB 2023): a delta is a weighted set, and the
  incremental form of a query applies the query to deltas. The document digests are a Z-set's sum, by document. The
  driver's skip is DBSP's "a zero delta propagates zero".
- **Soufflé's incremental evaluation** (Zhao, Subotić, Raghothaman and Scholz, *Towards Elastic Incrementalization
  for Datalog*, PPDP 2021): counting per tuple, with the incremental program generated beside the base one. The
  driver here generates it per stratum, when reached, which is the departure: their whole-program generation is the
  cost 0116 measured.
- **The spike's ledger** (`docs/rule-engine-direction/prior-work/ast-index-spike-history-index.md`):
  - *Process-held state:* "`Store.attach` rebuilds; fingerprint check" (DH L3319-3343). And "An in-memory vocabulary
    silently fell back after reopen: 'Storing data in memory... is dishonest'" (DH L2185-2189). The session-only
    counted exports are this trap: a new process silently took the slow route. Step 1 keeps the state in the store
    (the frames) and checks that it is the run's (the keeper's files against the kept part).
  - *§2.20 (d):* "Write paths must be O(inserted), not O(index)", and "Prove one-shot equals incremental". The read
    path here follows the first. The cross-verify switch is the second.
  - *"Persist running totals"* (DH L3345-3357): the document digests are running totals, kept with the frames.

## 5. Order of work

Each step is measured unprofiled on the 20-file corpus. Findings must equal the cold run's, after edits and
unchanged. Then the step is measured on the data layer's 9,010 files.

| Step | What | Measured cost it targets (0116) |
|---|---|---|
| 1 | Inputs from the changed documents; no replay of unchanged hits | Memo replay and lookup 1–2 s; 135 MB decoded; inputs hashed |
| 2 | Deltas as side structures of the relation (dead rows, appended rows); `__plus`/`__minus` views, not relations | Strata opened and indexed for deltas (part of 1.0–2.0 s) |
| 3 | The driver: rewrite per stratum when reached; base-program passes kept | Rewrite 0.5–1.7 s; whole-program passes about 1.8 s; strata split 0.4–2.0 s |
| 4 | Old rows read back only for strata reached | Kept part read 0.6–1.0 s; part of the 5.5 GB peak |

**The end target** stays 0090's: an edit of one function costs under 5% of a cold cross part. On the 20-file corpus,
the joins are about 0.7 s of an edit's 26 s of CPU today. The target is a run whose cost is mostly its joins.

## Built: step 1 (2026-11-11)

**What it does.**
- **Each memo entry has a second frame**, under the same part, holding its outputs: its nodes, tree counts and lines
  (`memo::outputs_rel`).
- **An unchanged file reads only that frame.** A file is unchanged when it is as the statistics keeper counted it:
  same content, same key. It is looked up before the first wave (`Memo::found_outputs_all`), and its rows are not
  decoded.
- **At the waves' end, each input of the cross part is rebuilt from the kept frames**
  (`retain::kept_inputs`, `kept_with_changed`):
  - an unchanged input is its kept rows;
  - a changed base relation is its kept rows, less the changed files' old entry rows by count, plus this run's
    rows;
  - a changed export is its kept rows, less the rows its location column gives to a changed document, plus this
    run's rows.
- **The checks.** Each removal is checked against the old entries, and every row must be attributable. A failure
  of either sends the run down the old route for that run: every hit is decoded whole from the entry already found.
  The inputs are made first and installed only if all of them succeed.
- **The cross part takes the kept inputs as its old sources** (`adopt_inputs`), named as the run binds them, so it
  does not admit them again.
- **Off switch:** `RERAIL_KEPT_INPUTS=off`.

**Measured** (20 files, unprofiled, findings equal to the cold run's each time):

| | Before (instructions, CPU) | Now (instructions, CPU) |
|---|---|---|
| First edit after cold | 289.6 G, 31.9 s | 282.0 G, 25.0 s |
| Second edit | 235.3 G, 27.5 s | 212.0 G, 21.5 s |
| Unchanged | 9.7 G, 1.2 s | 9.5 G, 0.8 s |
| Deleting a function with 4 findings | | 58 findings, equal to a cold run of the edited corpus (fell back, below) |

The profile of an edit says "19 memo hits read for their outputs alone; 148 of 5171 inputs changed, from 1 changed
documents". The memo replay is 0.3 MB of values, where it was 135.3 MB.

**What it still costs.** The install is 0.62 s. Of that, 0.20 s reads the kept inputs, and the rest copies every
input's rows into the run's export and base structures. That copy is step 2's to remove: an input becomes its kept
rows with its delta beside it, not a copy.

**Open: a value document's rows.** A value document's row belongs to each file that asked for it, and the kept base
relation can hold it once. Subtracting a changed file's old rows by count cannot then tell whether another file
still holds the row. So the run falls back, as it did on the deletion above. The exact form is step 2's count side
structure: how many files hold each row of a base relation, kept with its frame.

## Step 2, designed (2026-11-11): an input is the sum of its files' entries, kept by owner

**What step 1 got wrong.** Step 1 takes a changed file's old rows out of the kept input. That is exact only if the
kept input holds each row once for each file whose memo entry holds it. Two kinds of row are held by several files:
- a value document's rows (each file that asked for the value holds them);
- a string's rows, such as `value_type(v, t)` (each file whose text holds the string).

A cold run adds such a row once per chunk, not once per file, so the kept input can hold fewer copies than the
entries do. Taking one file's copy out can then remove a row other files still hold. The check catches this only
when the kept input holds fewer copies than one entry; a run with a shared row in a chunk of two files would pass
the check and drop the row.

**The rule.** An input of the cross part is the sum of the files' memo entries' rows, whichever route built it:
- **a base relation is that sum as a bag**: a row once for each file whose entry holds it. This is what the
  statistics keeper already counts (0096 §3), so its statistics stay exact;
- **an export is that sum as a set, with each row's count**: the number of files whose entries hold it. The
  session's counted exports (0096 step 3) are this structure, kept only in memory; step 2 keeps it in the store.

The chunk route builds its base rows from the entries it makes (not from the chunk's rows), and counts each export
row's files as it attributes them.

**Kept by owner.** A row's owner is its first node column's document (the location column the memo attributes rows
by; a string's own value for `value_type` and the text tests). Every row of one owner has the same holders: a
value document's rows go to every file that asked for it, a string's to every file that holds it. So:
- the frames hold each input's rows grouped by owner, each owner's rows contiguous within its granule;
- each owner's entry in the input's document table carries its holders, where it is not 1.

An edit then touches only the owners its changed files' rows name, read from their ranges. No other row is read,
hashed or copied.

**The edit.** For each owner the changed files' rows name:
- a base relation's rows are the kept rows, less the changed files' old entries' rows, plus this run's;
- an export's holders are the kept holders, less the changed files that held the owner, plus those that hold it
  now; its rows are this run's when a changed file holds it now, else the kept ones;
- a row removed that the kept rows do not hold, or an owner whose rows differ between two holders, is an error,
  and the run takes the old route.

**The relation with its delta.** The bound input is the kept rows with the changed owners' ranges dead and their new
rows appended (`retain::Layered`): a view, not a copy. A lookup goes through the kept rows' own index and skips the
dead rows. `r__plus` and `r__minus` are the rows an owner gained and lost, as sets, handed to the rewrite as given.
The digest is the kept one, less the changed owners' sums, plus their new sums. Written back, only the changed
owners' granules are rewritten, from the ranges.

## Built: step 2 (2026-11-11)

**What it does.**
- **The chunk route's base rows are its entries'** (`raw_base`, `canonical` in the chunk): a row once per file whose
  entry holds it. The batch's own rows are used only for a chunk the memo cannot attribute.
- **The frames group each input by owner** (`retain::group_col`; input identity v3), so each owner's rows are one
  range (`KeptInput::ranges`). Reading the kept inputs no longer hashes every row; each range is checked at its ends,
  and a changed owner's rows against its digest.
- **The keeper counts each export's value documents' holders**, kept in its blob (version 4).
- **`patch_owners` changes only the owners the changed files' rows name**: base relations by multiset arithmetic,
  exports by holders, with the errors above sending the run down the old route.
- **The bound input is `retain::Layered`**: the kept rows with the changed owners' ranges dead and their new rows
  after them, looked up through the kept rows' index.
- **`r__plus` and `r__minus` are handed to the rewrite as given.** The profile says "129 inputs' deltas kept beside
  them; 0 found by hashing their documents".
- **Written back, a changed granule is made from the ranges**, with no pass over the input.

**Measured** (20 files, unprofiled; findings equal to the cold run's each time):

| | Step 1 | Step 2 |
|---|---|---|
| Cold | 927.4 G | 934.3 G |
| First edit after cold | 282.0 G, 25.0 s CPU | 277.1 G, 28.7 s CPU |
| Later edits | 212.0 G, 21.5 s CPU | 216.3 G, 23.7 s; 214.0 G, 25.3 s |
| Unchanged | 9.5 G | 9.7 G |
| Deleting a function (58 findings) | fell back; 1079.4 G | on the new route, no fallback; 1089.5 G |

The machine's load was 40–100 during these runs, so CPU seconds vary by several seconds; the instruction counts
are the comparison. **Step 2 did not make an edit cheaper.** It made the route exact (the deletion is right on it,
and shared rows can no longer be dropped), and it removed the hashing of the changed inputs. The install the
profile showed at 0.6–0.8 s is about 0.2 s unprofiled (sampled). Almost all of that is `kept_inputs` copying every
kept input's rows out of the frames: a read of unchanged rows, which is step 4's to remove (old rows read only for
the strata reached).

**What an edit costs now.** The main cross part runs 8,313 of 10,355 strata (5,578 maintained from deltas, 2,720
derived again), with 148 inputs changed. That is steps 3's and 4's work, not this step's.

**The deletion costs more than a cold run** (1089.5 G against 922.2 G). Deleting 100 lines changes nearly every row of
that file (its nodes move), so the deltas are most of the file's rows, and maintaining from deltas that large costs
more than deriving again. The driver of step 3 should choose between the two by the size of the delta.

## Built: step 3 (2026-11-12): the run walks the base program; a reached stratum's form is made there

**What it does.**
- **The run is given the base program** (the cut's: clean strata left out, as before), with a driver
  (`plan::Maintain`, `Hooks::maintain`). The changed inputs' deltas are bound first (step 2), and they seed the set
  of relations that moved.
- **Each stratum is a slot of the walk** (`plan::run`, decision 0117 step 3 in its loop):
  - *No read moved:* each relation is its kept rows, held in place (`hold_as`); nothing is rewritten, planned,
    opened or read.
  - *A read moved:* the driver makes the stratum's form (`corpus::Driver::form`): `maintain::rewrite_with` over a
    program of that stratum alone, its reads declared as inputs and the moved ones as changed
    (`maintain::One`). The old rows it reads are bound there, with their statistics (`bind_kept`, the binding the
    whole rewrite did for every relation before the run). Its pieces (`__del`, survivors, `__redo`, `__ins`, its
    deltas) are the form's own strata, each run by the same native engine (`native_stratum_now`), with the form as
    the program.
  - *After it:* a relation whose delta (`r__minus`, `r__plus`) holds rows moves its readers. One derived again
    with no old rows to diff against moves them with no delta, and they derive again. The form's pieces are
    released; its relations, deltas and old rows stay.
- **A form always makes its relations' deltas**, since the walk asks whether they are empty. A delta or old source
  an earlier form made or bound is read, not made again.
- **The device waves' prefetch and the strata scoreboard (0113) were off in a driven run** at first: both work over
  the base program's strata ahead of the walk. The walk now places a level at a time, and does both for the strata it
  reaches (below, "Built: the walk a level at a time").
- `RERAIL_MAINTAIN_WHOLE` rewrites the whole program first, as before.

**Measured** (20 files, unprofiled, findings equal to the cold run's each time; load 27–45):

| | Whole rewrite (hub) | Driven |
|---|---|---|
| First edit after cold | 283.1 G | 250.7 G |
| Later edits | 261.3, 257.2 G | 219.0, 208.7, 210.4 G |
| Strata | 8,438 rewritten (189,301 rules), 45,321 maintained strata walked | 3,719 of 8,438 reached and maintained, 4,719 held as kept |
| Main thread (sampled) | 12.24 s | 10.06 s |

**What an edit costs now** (sampled, 16.1 s CPU in all, 10.1 s on the main thread):
- statistics 1.6 s: the reached strata's lazy statistics (0.6 s), patching kept statistics by delta (0.35 s), the
  given deltas measured (0.17 s);
- opening strata 1.2 s;
- the prepared program from the store 1.2 s (0116 step 2's; not this step's);
- the kept part 0.7 s;
- the memo 0.5 s, joins 0.5 s, planning 0.4 s;
- and on the store's writer, 2.4 s, with the main thread waiting 1.7 s for it.

**What step 4 has to remove, measured.** Each edit reads 12,960,053 rows of 9,671 relations from the frames, and
writes back 12,227,715 rows:
- 2,752 relations whose rows changed: 9,323,541 rows, each relation written whole, though a delta changed it;
- 2,674 relations whose rows did not change, rewritten only because their reads' digests changed: 2,904,174 rows.

## Built: step 4, first part (2026-11-12): a relation whose rows did not change is not written again

**What it does.**
- **The run names the relations whose rows are as they were** (`Outcome::unchanged`): those of a stratum held as
  kept, and those a form maintained to an empty delta.
- **Their frames are not written again** (`Retained::write_frames`). Each one's digest is its stored one, so its rows
  are neither copied out nor hashed. Where its reads' digests changed, only the reads are written:
  `ContentStore::queue_frames_retagged` updates the frame's `reads`, and its rows, digest and the rest stay as stored.
  The frames' snapshot shares the stored rows (`Data::Encoded` holds them in an `Arc`).

**Measured** (20 files, unprofiled, `RERAIL_STATS_KEEP_TRACE`; findings equal to the cold run's each time):

| | Before | After |
|---|---|---|
| Frames written whole although their rows had not changed | 2,674 (2,904,174 rows) | 0; 3,620 frames' reads updated, no rows |
| Frames of relations that changed | 2,752 (9,323,541 rows) | 214–216 (8,989,543–9,012,206 rows) |
| Later edits | 208.7–219.0 G | 214.1–218.6 G (load 49–63) |

The second edit after a rebuild still writes 2,752 relations whole. That edit's first run kept every relation as
changed, since none had a frame of the run's yet.

**Then open in step 4** (both built below):
- **A relation that changed is still written whole**: 214 relations, 9.0 M rows an edit, where a delta of a few
  rows changed each. Its frame is one part. The fix is the inputs' layout (step 2): a derived relation's frame in
  parts by its owner's granule, with only the changed owners' granules written. The digest is a sum of row hashes
  (`rows_digest`), so it can be patched by the delta, without a pass over the rows.
- **The old rows are still read for every stratum the cut keeps dirty** (`Retained::load`: 9,671 relations,
  12,960,053 rows), though the walk reaches 3,719 strata. A relation should be admitted from its frames when a
  reached stratum reads it, or when the run's end hands it on.

## Built: the walk a level at a time (2026-11-12): the choice of 0090 §4, the device waves and the strata issued ahead

A stratum's producers are all on lower levels, so at a level's first stratum whether a changed read reaches each
stratum of the level is known. The walk places the whole level there (`plan::run`, `Slot`):

- **Quiet or reached.** A stratum no changed read reaches holds its kept rows, or leaves them stored (below). A
  reached one gets its form.
- **Maintained or derived again (decision 0090 §4).** A form keeps its form derived again (`front::Again`: its rules
  over the current reads, and its deltas against its old rows). The run chooses as the whole rewrite chose: its input
  deltas past θ of its old rows, and the form derived again priced below the maintained one (`again_wanted`). Derived
  again, its pieces are its rules and its deltas. On the 20 files no stratum's input deltas pass θ, so none is.
- **The device waves (decisions 0095 §5, 0100 §5).** The count tables and first joins of every stratum the level
  runs (the forms' pieces, and the base program's strata that run) go out as one batch, as a base level's do.
- **The strata issued ahead (decision 0113).** Each form whose pieces a strand can run is issued as a job on the host
  stream, its pieces in order on one strand, and retired at its stratum's position (`stream.wait`, `Run::absorb`). The
  pieces after one that declined, read SQL or made an atom run on the loop's thread. `RERAIL_STRATA_PARALLEL=off`
  runs them all there.
- **A strand's statistics** were copied by a pass over every relation's column statistics (`Model::fork`): 5.4 s of
  an edit with 3,700 forks. They are kept by relation now (`ColStats`), and a strand copies those of what it reads.

**Measured** (20 files, a fresh store, unprofiled; findings equal to the cold run's each time; load 17–30):

| An edit | Instructions | CPU | Wall |
|---|---|---|---|
| Forms run on the loop's thread (`RERAIL_STRATA_PARALLEL=off`) | 210.0, 210.3 G | 13.1, 13.2 s | 15.0, 13.4 s |
| Forms issued as jobs (the default) | 226.2–230.7 G | 18.0–18.2 s | 11.6–12.1 s |

3,701 of 3,701 reached strata's forms ran as jobs. Issuing them costs 16 G instructions and 5 s of CPU for 1.5–3 s
of wall: a job a form is small (a form is 5 pieces on average), and each pays for its strand. The board merges strata
into components of 8 fixed costs; forms were not merged. They are now: see *A level's forms merged* below.

## Built: step 4 (2026-11-12): old rows read for the strata reached; a changed relation writes its delta

**Old rows read where they are read.** A maintained run with frames no longer reads every kept relation of the
dirty strata before it starts (`Retained::load` reads only the clean outputs then):
- **A quiet stratum's relations stay stored** (`Outcome::unread`) unless a stratum that runs reads them, or the caller
  does after the run (an output, or what it asked for).
- **A level reads, in one batch, what its running strata read** (`Maintain::need`, `Retained::admit`): the old rows of
  the strata it reaches, and the kept rows they read. A clean relation the part reads is bound there, with its
  statistics (`bind_kept`). Until then the planner knows its rows' count only (`plan::Unbound`).

**A changed relation writes its delta.** A relation a form maintained has its rows gone and added (`r__minus`,
`r__plus`, `Outcome::deltas`). Its frame keeps its rows, and the delta is written after it as a frame of its own
(part −1, −2, …; `FrameDelta`, first type `~`, a sign column first). The relation's digest is patched by the delta
(`rows_digest_patched`: the digest is a sum of row hashes), so its rows are not read to hash them.
- The store reads a frame with its deltas (`Data::Chain`): queued, in the snapshot read before the first wave, or
  from SQL. Every reader of the frame sees the rows after its deltas.
- A frame's stored reads end with how many deltas follow it (`CHAIN`), so the store reads only the deltas of the
  frames that have them, in one statement a batch, and none for a frame put whole. A retag keeps the count. A frame
  put whole ends its deltas: those the snapshot holds are deleted with it, in the put's batch.
- A relation is written whole again when its frame has 16 deltas (`RERAIL_FRAME_DELTAS`), or when its deltas would
  hold more than a quarter of its rows.

**Measured** (20 files, a fresh store, unprofiled, `RERAIL_STATS_KEEP_TRACE`; findings equal to the cold run's each
time):

| An edit | Before step 4 | After |
|---|---|---|
| Kept relations read from the frames | 9,671 (12,960,053 rows) | 5,586–5,600 (12,155,079–12,155,517 rows); 3,552–3,566 left stored |
| Derived frames written | 2,968 (12,227,715 rows) | 214–218 deltas (564–580 rows); 3,583–3,593 frames' reads updated |

- **The rows read fell less than the relations.** The strata reached read most of the rows: the 214 relations that
  change hold 9.0 M of them, and their old rows are what they are maintained from. A process reads them from the
  store. A session holds them, and reads none.
- **Checked:** a file deleted gives the findings of a cold run of the corpus without it (61); put back, those of the
  cold run (62). With `RERAIL_FRAME_DELTAS=2`, two edits wrote deltas and the third wrote the 212 relations whole
  that had two, with the findings equal each time; the next edits wrote deltas after the new frames, and no delta
  past a frame's count is left in the store. A store test reads a frame's deltas queued, stored, from the snapshot,
  after a retag, and after a frame put whole (`a_relations_deltas_are_read_after_its_frame`).

**Found while measuring, not changed:** the store read before the first wave (decision 0104) reads every frame of
every space. Each engine rebuild made a new space of the memo's entries (about 220 MB each), and no run uses the old
ones again, so none is forgotten: the store of the measurements above had 1.86 GB of frames, 1.75 GB of them old
spaces'. The numbers above are on a fresh store.

## Built (2026-11-12): a level's forms merged into components, and the frames read before the first wave

### A level's forms merged

A level's forms run as at most `RERAIL_STRATA_WIDTH` jobs (6), each a component of forms (`Run::strand_forms`,
`Strand::run_forms`, `Run::absorb_forms`). A component has one strand, one fork of the statistics and one prime of
the scope, and runs its forms' pieces in level order.
- **Balanced.** The forms go into the least loaded component, the largest first. A form's estimate is its fixed cost
  per piece plus, per rule, the rows of the least of its positive atoms (a delta rule's driver). The full
  cardinalities of the relations it reads counted old rows' millions, so almost nothing merged.
- **Fewer components where a level is small.** A level has no more components than its estimate holds `k` fixed costs
  (`RERAIL_STRATA_K`, 8). A component's first slot is its lead: the loop takes the component back there.
- **The width, not a size, bounds the jobs.** Cut into components of `k` fixed costs, a run made 1,700 jobs. With
  18 threads that cost 6–10 G instructions more than no jobs, and with 6 threads (`RERAIL_THREADS=6`) 1–4 G.
- **Shared state a job takes.** The plans cache and the observed statistics took a lock for each use from each job
  (2.9 s and 2.2 s of an edit's samples). Each job now writes to an overlay of its own thread
  (`OWN_PLANS`, `OWN_OBSERVED`), folded into the run's when the job ends.

**Measured** (20 files, a fresh store, unprofiled, findings equal to the cold run's each time; load 12–42, so wall is
indicative only). The edited file grows by a line each edit, so compare within a round:

| An edit | Instructions | CPU | Wall |
|---|---|---|---|
| Forms on the loop's thread (`RERAIL_STRATA_PARALLEL=off`) | 219–237 G | 14.1–19.0 s | 14.7–21.2 s |
| A form a job (`RERAIL_STRATA_MERGE=off`) | 231–243 G | 18.6–20.4 s | 12.0–15.2 s |
| Components, cut at 8 fixed costs | 229–239 G | 18.4–21.9 s | 11.0–20.2 s |
| Components, at most 6 a level (the default) | 245–251 G | 16.7–17.4 s | 15.6–17.8 s |

Per thread (`PROC_PIDTHREADCOUNTS`, two edits each): with no jobs, the main thread retires 206 G. With components, the
main thread retires 114–116 G and the pool 100 G, about 8 G more in all. Every one of the 18 pool threads retires 1–3
G, including those no component ran on. A sample of the pool's busy time shows the strata's joins and settles
(`run_join_in_each`, `Fresh::settle`), the work the main thread does with no jobs. **Not found yet:** why the same
strata retire about 8 G (4%) more on the pool.

### The frames read before the first wave

The store read before the first wave (decision 0104) read every frame of every space: 1.86 GB on the store measured,
1.75 GB of it no run would read. The frames came from three places:
- **Every engine build's memo stayed live.** A plan names the engine build, and its memo is a space of frames
  (decision 0108). A plan was live while any kept manifest named it, and manifests are kept by the bytes of their
  sources and tapes, which a small corpus never fills.
- **Killed runs' holds.** A run holds its plans for a 6-hour lease and releases them as it ends. A run the watchdog
  killed never did, so its plans stayed live for 6 hours.
- **An edited file's earlier contents.** The memo kept an entry for each content of a path for 32 runs: 30 contents
  of the edited file, 593 MB of one space.

What the store does now:
- **Only a root's protected runs keep a plan live** (`keep_corpus_runs_held`), with the plans their manifests name
  and those a running run holds. A kept manifest past them keeps its sources and tapes, which any build reads.
- **A hold whose process is gone is released** by the next collection pass (`orphaned_holds`; a run's holder names its
  process). A decision follows at once.
- **A pass collects every dead plan**, a transaction each, within its budget of rows. A dead plan's memo is a few
  frames, and the pass counted none as work, so it stopped after one plan a run.
- **A path keeps its 4 newest contents** (`MEMO_CONTENTS`, `keep_newest_by_path`), so worktrees or branches that take
  turns at a file keep theirs (decision 0108 §4).
- **The read before the first wave takes the live plans' memo outputs only** (`preload_frames_live`), each path's
  newest. A memo's entries (`local`, 15 MB for the edited file) are read by their keys where they are looked up. The
  files of a run are looked up before its first wave, from the store; the entries a wave may look up (a load's files)
  are read into the snapshot before the first wave (`admit_memo_entries`).

**Measured** on the store of 1.86 GB, findings equal to the cold run's each time:

| | Before | After |
|---|---|---|
| Frames in the store | 1,582 MB (46,958 frames) | 374 MB (48,205); every recorded plan live |
| Read before the first wave | 482 MB (45,448 frames) | 160–163 MB (45,154–48,150), 0.2–0.5 s |
| An edit | 241–282 G | 240–288 G |

Of the 163 MB still read, 107 MB are the kept strata's frames (one space a root, all of them used by the last run).
The run reads the rows of 5,595 of their 9,152 relations; the others' frames are read and left undecoded.
Tests: `only_protected_runs_keep_plans_and_a_dead_process_holds_nothing`,
`a_memo_keeps_a_paths_newest_and_preloads_live_plans_only`.

## Built: step 5 (2026-11-12): a frame found unchanged is confirmed, not written

**Before**, an edit rewrote the reads of each unchanged relation whose reads' digests had moved
(`queue_frames_retagged`): 3,589 frames, one `UPDATE` each, every edit. That was most of the end-of-run wait.

**What it does.**
- **Each space keeps one row of confirmations** (`frame_confirmed`): for each frame found unchanged under reads
  other than its stored ones, the hash of the reads it was confirmed under (`confirm_key`, `reads_hash`). A retag is
  now an entry in that row, written once per space and only if the map changed. The frame's own row is not touched.
- **A frame is current** (`from_frame_snapshot`) if its stored reads are today's, or if its confirmation names
  today's reads' hash. It is then taken with today's reads.
- **A frame written whole, given a delta, or gone loses its confirmation.** This happens in memory and in the store
  (`keep_frame_refs`, `keep_frames`).
- **The confirmations are read with the frames before the first wave** (`preload_frames_for`), so no SQL runs in the
  waves. They are kept current by the writes the process queues.
- **A frame admitted later is checked by digest** (`admission_matches`): its rows are its digest's, and its reads
  may be newer in what the run opened than as stored.
- The trace settings `RERAIL_WRITES`, `RERAIL_STATS_KEEP_TRACE` and `RERAIL_JOURNAL_TRACE` are no longer part of the
  run's key (`NOT_OUTPUTS`). A traced unchanged run was missing the whole-run cutoff.

**Measured** (20 files, findings equal to the cold run's each time, load 16–67):

| | Before | After |
|---|---|---|
| `UPDATE`s of unchanged frames an edit | 3,589 | 0; one row of confirmations |
| Frames written an edit | 25.4 MB, 2.76 s | 22.9 MB (4,669 frames, all with changed rows), 1.85–3.1 s |
| End-of-run wait (main) | 2.58 s | 1.9 s |
| An unchanged run | — | 0.8 s, 11.4 G |

**Open, found while measuring.** The remaining frame writes are mostly input granules: an edit rewrites all 65
granules of about 70 `strtest_*` relations. In `write_frames`, about 1,100 of each one's owners alternate between 20
rows and 19 rows from one edit to the next. In the same run, the kept-input patch (`patch_owners`) changes only 4
owners. So the per-owner digests stored and those recomputed for the write disagree. Tests:
`a_relations_deltas_are_read_after_its_frame` (confirmations in memory and stored).
