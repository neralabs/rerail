# 0725. Callable per-file relations: the spike

Status: built as a spike (2026-11-27), by the user's ask on the "Callable Per-file Relations" page: "go ahead with
the spike", scoped "1. explicit callables, although eventually we will want to provide feedback as to which should
be 2. no device, although eventually we will want it to run on the device too". CPU only; `.callable` declared by
hand.

## Intro

A relation a file's rules make can be large while its readers want only a few of its rows: `same_globals(d, f, g, v)`,
the pairs of functions in a file that touch a name, is n(n-1) rows a file, and the cross part that reads it asks
only about the pairs one function calls together. Made whole, every file makes every pair and hands them on. Called,
the cross part asks for the rows it binds, and each ask is answered in the file it names, over that file's
relations as they stand. Nothing makes the rows no one asks for.

## What is built

- **The declaration**, `.callable R PATTERN` (`.callable same_globals bbbf`): a `b` or `f` a column, column 0 bound
  (it names the file). Refused at load, each naming what it refuses: a pattern of the wrong length or letters; column
  0 free; a `node` column (decision 0677: no node crosses); a callable with no rules, or an input; a negated read; a
  reader that does not bind a bound column before its read; a callable whose rules read a callable or a relation
  made from calls' answers (`preparation::callable`).
- **The split** (`callable::split`): the caller loses R's rules, and each rule reading R gains a request rule,
  `R__call(bound) :- the rule's other positive atoms and the filters they bind`, answered `R__call -> R` (decision
  0065 §2a). The callee is R's rules, each reading its request first; its inputs are the request and what R reads.
- **Calls in the corpus run** (`corpus::calls`): the requests are made in the cross part's cone and answered inside
  its fixpoint, a batch at a time. A batch is grouped by what holds each file's rows; each holder is entered once a
  batch, and the callee runs on a connection of its own over the holder's relations, bound where they lie. The
  per-file part answers no call.
- **Held state**: what a callee reads joins the asked phases' held relations (decision 0705), kept to the end of the
  cross part. A chunk's rows are held where its engine made them (`component::derived`, read in place, decision
  0353); a file the memo gives is read from its entry's rows, moved into a flat source, not copied. A call into a
  partial memo hit's file is refused: its rows are not held.
- **The memo**: each request asked is kept for the run, its key alone. Asked again, it enters no file and adds no
  row: its answers are already the caller's rows of R, the one place they lie.
- **The counters**, a profile line: batches, requests, those from the memo, files asked, holders entered, answer
  rows, and the time in µs (and per holder entered).
- **The candidates** (`callable::candidates`, the feedback's first version): each run writes to the profile the
  exports its files make that the cross part could call (column 0 the file's, every reader a cross-part rule reading
  them positively and binding the file and at least one more column, at least one column free), by their rows, with
  the reason for each of the largest it leaves out.

## Tests

| Requirement | Test |
|---|---|
| A callable relation is answered in its readers' join | `corpus::calls::tests::a_callable_relation_is_answered_in_its_readers_join`; on a corpus run, `rr::callable_tests::a_callable_relation_is_answered_in_its_readers_join` (the findings equal the relation made whole) |
| A batch enters each file once | `corpus::calls::tests::a_callable_enters_each_file_once_a_batch` |
| The memo holds what was asked | `corpus::calls::tests::a_callables_memo_holds_what_was_asked` |
| A call across two files' definitions enters neither | `rr::callable_tests::a_call_across_two_files_definitions_enters_neither` |
| A callable answering a node is refused | `corpus::calls::tests::a_callable_answering_a_node_is_refused` |
| A call with its pattern unbound is refused | `corpus::calls::tests::a_call_with_its_pattern_unbound_is_refused` |
| A callable reading an incomplete cross result is refused | `corpus::calls::tests::a_callable_reading_an_incomplete_cross_result_is_refused` |
| A call leaves its file's state as it was | `corpus::calls::tests::a_call_leaves_its_files_state_as_it_was` |
| An edit invalidates what called into the file | `rr::callable_tests::an_edit_invalidates_what_called_into_the_file` (the run after an edit, its other files from the memo, equals a fresh run) |
| The pattern is linted | `corpus::calls::tests::a_callables_pattern_is_linted` |
| A run names the relations it could call | `rr::callable_tests::a_run_names_the_relations_it_could_call` |

## Results

**The generated corpus** (`rr::callable_tests::callable_measure_*`, ignored, one a process): 3 Python files of n
functions, each touching `x`, and one function a file calling 4 of them. Made whole, `same_globals` is n(n-1) rows a
file and a few more; called, the cross part asks for the 12 ordered pairs a file. The tests' debug build on an M3
Max, profiled in every mode:

| n a file | whole: rows made, CPU | magic sets (`.pragma "magic-transform"`) | called: rows answered, CPU |
|---|---|---|---|
| 10 | 294, 0.31 s | 294, 0.31 s | 36, 0.36 s |
| 100 | 29,724, 2.61 s | 29,724, 2.73 s | 36, 0.38 s |
| 1,000 | over 600 s (the test watchdog) | over 600 s | 36, 1.08 s |

The same 12 findings in every mode that finished; peak 52 to 56 MB in each. Magic sets change nothing: the readers
are in the cross part, and the files' part has no demand from them.

**The cost per batch**: one batch of 48 requests, entering one chunk that holds the 3 files, took 933 to 1,336 µs,
20 to 28 µs a request, against t61's 47 µs to open one engine run.

**The real cases, by the candidates' line** (the CLI's debug build, cold, on fresh stores). The split hands a
file's rows of a cross-read relation R on as `R_at(d, ..)`, its document in front, and relays them,
`R(..) :- R_at(_, ..)`: the line looks through the relay to R's readers.

- Routed by the file (the spike's call): none, in either corpus. The cross part reads a file's exports by symbol,
  module, qualified name or term (decision 0677's keys), and the read is what finds the file: no reader knows the file
  before it reads.
- Routed by the columns the readers bind (by key), over the PostHog sample's 291 files with Django's three policies
  that read points_to (58.7 s user CPU, 1,459 MB): 133 of `points_to`'s 188 exports, the largest `pt_at_at` (`fbf`,
  118,867 rows), `pt_chk_at` (`fb`, 118,867), `pt_look_t_at` (`ffbb`, 93,224), `pt_vopen_l_8_at` (`fb`, 79,319) and
  `pt_btype_l_0_at` (`ffb`, 50,764); 11 of `py_names`' 30, the largest `reads_at_at` (`fbf`, 11,117); 17 of the
  policies' 38, the largest `violation` (`fbff`, 461,298, with a node column). Not callable: `pt_vopen_l_0_at` and
  `pt_vpos_t_at` (106,654 each) and `imported_at` (10,505), read with no column bound; `pt_bound_in_t_at` and
  `pt_rbound_t_at`, negated.
- Over the data layer's 224-file sample, `bandit-b113` (16.6 s user CPU, 702 MB): 7 of `py_names`' 20 exports by
  key, the largest `reads_at_at` (`fbf`, 14,521 rows); `imported_at`, `fwd_any_at` and `defined_at` read with no
  column bound.

A call routed by key needs what the spike does not have: each file's keys, so that a request is sent to the files
that can answer it. The rows here are the exports' rows, not what readers ask for: how many of the keys a run asks
for is not measured.

**Rows a key**, by the run's totals of each candidate's bound columns (the same PostHog run, 49.2 s user CPU): the
largest of `points_to`'s have about one row a key. `pt_at_at` 118,867 rows, 117,008 distinct terms; `pt_chk_at` the
same; `pt_vopen_l_8_at` 79,319 and 77,604; `pt_btype_l_0_at` 50,764 and 50,122; `pt_callee_t_at` 27,335 and 27,382.
The most rows a key: `pt_qn_t_at`, 38,077 rows over 4,720 keys (8 a key); `pt_look_t_at`, 93,224 over 11,396 values
of one of its two key columns (8 a key at most). The policies' `violation`: 461,298 rows, 461,298 keys. A file's
keys handed on for routing would be as many rows as these exports are: the exports are already keyed summaries
(decision 0677), and no export of these programs is the generated corpus's shape, many rows a key.

## The asks of the exported relations

Recorded (`RERAIL_ASKS=1`, `preparation::asks`, a measurement aid): for each rule of the cross part and each
relation of the files it reads (through the split's relays), the keys it uses, by the columns the rest of its body
binds (its shape), and the rows of the relation it uses, each from the reader's whole body; written to the profile
beside the relation's rows made. A key that finds no row is not counted, nor a read inside an aggregate; several
readers of one relation use between the largest one's rows and their sum. The tables, a row a (relation, reader,
shape): `rerail-shared/callable-asks` (`asks-posthog.tsv`, `asks-dl.tsv`); the samples, `rerail-shared/bench/
posthog-291` and `dl-224`.

| | PostHog's 291 files, Django's points_to policies | The data layer's 224 files, `bandit-b113` |
|---|---|---|
| Asks (relation, reader, shape) | 425, over 259 relations | 37, over 24 relations |
| Exported relations' rows made | 2,848,456 | 149,589 |
| Rows their readers use | 427,804 to 524,952 (15 to 18%) | 54,326 to 55,498 (36 to 37%) |

By shape, over PostHog (rows made summed over asks, so a relation read twice counts twice):

| Shape | Asks | Rows made | Rows used |
|---|---|---|---|
| every column bound (a test) | 104 | 1,310,079 | 128,916 |
| 1 bound, 1 free | 102 | 1,266,438 | 134,106 |
| 1 bound, 2 free | 53 | 756,286 | 31,281 |
| nothing bound | 77 | 498,952 | 154,598 |
| 2 bound, 1 free | 23 | 496,145 | 82,055 |
| 1 bound, 3 free | 16 | 482,212 | 245 |
| 3 bound, 1 free | 10 | 135,312 | 0 |
| negated test | 23 | 128,587 | 15,875 |
| the rest (2 bound and 2 or 3 free, 1 bound and 4 free, 3 bound and 2 free) | 17 | 53,104 | 284 |

The largest relations: the sync-orm policy's `violation` and its `rr_diagnostic`, 461,298 rows each (53% and 40% of
the run's 28 MB of values), whose one reader each uses none (the run's 5 findings are get-handler-writes'):
candidate findings made in every file that the cross part rejects. `pt_at` and `pt_chk`, 118,867 rows, used
30,524 to 42,121 (shapes `bf`, `b`); `pt_vpos_t`, 106,654, used 44,471 to 72,138 (`b`, `f`); `pt_vopen_l_0`, 106,654,
used 987 (`ff`); `pt_look_t`, 93,224, used 44,979 to 61,385 (`fbb`); `pt_vopen_l_8`, 79,319, used 1,193 (`b`);
`pt_btype_l_0`, 50,764, used 16,601 (`fb`). Over the data layer: `imported`, 16,916 rows, used 10,941 to 11,877
(`bb`, `fb`, `ff`); `asks_at`, 18,853, used 3,512 (`bb`); `reads_at`, 14,521, used 516 (`bf`).

**Each relation's disposition** (the "Exported relations' asks" page, https://claude.ai/artifact/KujXwtdjeevwMFLKoEmW2t;
the tables `rerail-shared/callable-asks/asks-*-disposed.tsv`, made by `asks_classify.py` and `asks_dispose.py` there).
A relation is made whole when any reader needs it whole, so it takes one disposition from all its asks; the largest
were read rule by rule. Over PostHog's 2,848,456 rows:

| Disposition | Relations | Rows made | Rows used |
|---|---|---|---|
| A finding's per-file half (`fl…_violation`, `fl…_rr_diagnostic`, the sync-orm walk's keys) | 12 | 958,678 | 6 to 8 |
| Read by no reader here (mostly lifted `_l_i` relations of library values) | 77 | 333,513 | 0 |
| A term's document stored a row a term (`pt_at`, term to path; `pt_chk`, term to checked document) | 2 | 237,734 | 61,048 to 84,242 |
| Read by key, a term bound (the answering file's) | 61 | 615,754 | 144,016 to 182,362 |
| Read by key, no term of the answering file bound (`pt_qn_t` by a qualified name) | 28 | 147,240 | 16,919 to 17,937 |
| A negated test | 11 | 60,525 | 8,807 to 8,817 |
| A filter on file-part relations left in the cross part by 0677's lift (`pt_vopen_l_0`, `_l_6`: `!pt_tracked(k)`) | 2 | 128,415 | 987 |
| A projection: the reader drops the location (`pt_lwant(q) :- pt_qn(_, q)`), so its rows are no file's | 7 | 94,373 | 16,407 to 20,784 |
| A key the rule computes after the read (`res`: `defined(_, q), split_dot(q, c, a)`) | 1 | 4,160 | 1,987 to 2,823 |
| Scanned whole by a reader | 51 | 133,202 | 126,048 to 127,739 |
| Scanned under negations of cross-part relations, an anti-join (`pt_vpos_t` for `pt_type_open_all`) | 6 | 133,879 | 51,546 to 79,220 |
| Scanned, partly used | 1 | 983 | 33 |

Of the keyed and negated reads, 255,565 rows lie in relations under 5% asked (at most 4,047 asked) and 180,007
in relations 50% or more asked; 360,244 in between (at most 76,812). Not measured: a request's cost at scale (the
spike's 20 to 28 µs is a 48-request batch's, mostly its fixed cost), the projections' per-file distinct values, and
the keys sent that find no row.

**Each probe's keys and their files** (the "Probe Routing" page, https://claude.ai/artifact/63DtrhU31j5PvjPGmfTe5U;
`rerail-shared/callable-asks`: `asks-keys-*`, `probes-*`, `keys-home-*`). For a keyed ask of a located export
(`R_at(d, ..)`), `RERAIL_ASKS` also makes the files answering each key asked (`rr_kfile__`, with `doc`'s path), the
relation's every (key, document) pair (`rr_kpair__`) and its keys (`rr_kall__`), and the run writes `asks-probes.tsv`
(a probe a row: keys, files a key, an index's keys and pairs, samples) and `asks-keys.tsv` (every key found, with its
files) beside the profile. Over PostHog's 291 files, 162 probes found 353,614 keys (every round's): 97.61% answered
by exactly one file, the one the key names (its path, or the longest module prefix of a term's or a definition's
scope); 0.04% by that file and more; 1.92% by files that use a name another file of the run defines; 0.43% name
no file of the run (an outside module, a guest's term). Over the data layer's 224, 92.80% of 16,830. Of the
8,452 not at home, 2,138 are answered by several files (71 by 32 or more; at most 96, `structlog`). The 21
relations they are of make 165,954 rows; an index of their every (key, file) pair is 58,165 keys and 107,580 pairs.
The import graph, read from the sources, as a router (a key to the importers of its longest imported prefix):
8,234 of the 8,452, sending to 50,335 files for 18,275 answers; 218 keys missed (re-exports, guests). The
probes' relations peaked the run at 2.34 GB (1.61 GB without; 54.4 s user CPU).

**Every keyed ask names its file.** The probes above read only the split's relays (`R_at(d, ..)`): 13 keyed asks of
PostHog's and 3 of the data layer's had none, as their relations reach the cross part with a location column of their
own: a document (`asks_at`'s, `star_of`'s, the pair caches' `sym_at` and `spelled_at`), or a node (`violation`, the
`rr_diagnostic_` relations, `pt_def`, `class_def`). `RERAIL_ASKS` now reads that column (a node's document by
`docof`), makes its index relations in the cross part (`cross_file`), and writes `asks-located.tsv`: how each probe
names its file (`relay`, `document column C` or `node column C`, with `, key J` where the asked key holds it). On both
corpora no keyed ask is left without its files (PostHog 193 of 193, the data layer 20 of 20); no relation asked by key
lacks a location column. Of PostHog's 358,006 keys found, 1.19% hold their document or node themselves and 96.45% are
at home, 97.64% in all; 1.90% are answered by files elsewhere, 0.42% name no file of the run. The data layer: 21.30%
and 73.03% of 21,386 (94.33%), 2.67% elsewhere, 2.92% none. The pair caches are asked by their document alone (`fb`, the
document bound): a whole file's rows. The findings are the same (PostHog's 5); 52.0 s real, 2.32 GB.


**A call routed by its key's file (`.callable R PATTERN by H [C]`).** For the 97.6% whose key names its file, the call
is sent to the file the key names. `H(k, d)` is a router of column C (default: the first bound one): the callable split
writes `H__ask(k) :- <the reader's body>` and `R__call(d, key..) :- <the reader's body>, H(k, d)`, and the callee
reads `R__call(_, key..)`, column 0 bound by the router. A router reads every file's table, so it is declared
`cross_file`; the run refuses a request made in each file (there a router reads one file's rows and routes none).
Nothing but the split reads a router before the split runs, so the passes that drop what no output reads take it as a
root: `prune`, the relations no output reads, a part's outputs (decision 0345) and demand's closure (decision 0048),
which had dropped `pt_tm_file`'s rules and left every call empty. A file-local relation the cone of requests reads is
exported, not projected again in the cross part from a stored input, whose rows no load answers (the router's table
of the files). The library's router is `pt_tm_file` (python/points-to-solve): a term's file, by its scope walked up
with `@pkg_up` to a module of `doc`'s, and a guest's term by the document its string holds (`pt_tm_guest_doc`); it
routes `pt_vopen_l_8 b`. PostHog's 291 files, get-handler-writes: 2,260 requests to 163 files (3 holders entered), 0
of no file, 1,193 answer rows; the cross part's `pt_vopen_l_8` is those 1,193 rows, against 79,319 made whole. The
findings are the same 5, identical. Measured again with `time -l` (instructions retired do not move with the
Mac's load; CPU did, the same `pt_tm_same` steps over the same rows taking 1.44 times the CPU a row in a run more
loaded): 799.7 G instructions against 780.4 G, +2.5%; process CPU 53.06 s against 50.77 s. The first figures here
(CPU 29.4 s against 24.8 s, one run each) were the load's, not the call's. Peak memory 1,220.5 MB against 569.2 MB:
with the call, points_to's cross part makes a request, so its cone holds the solve that makes the key (the
66-relation component `pt_acls/pt_ainst/…`), and what the rest reads of the cone is handed on as rows
(`held_rows`, then columns again for the cross part's bind): 640 MB allocated in 0.3 s at the cone's end and freed
after; the cone 0.78 s CPU (473 MB allocated), the hand-on 0.54 s (715 MB). The calls themselves are 20 ms. Each file still makes the callee's memo entry
(`pt_vopen_l_8_at`) whole. Tests: `callable_tests` (a routed call, under demand, and a router's table of the files
exported).

**Calls in bulk: every callable's requests, one call into each file.** The callees are lowered together, one program
(`Split::program`: every callable's rules, every request an input, the union of what they read in a file), and the
runner answers a list of requests at once (`Calls::answer_all`): their new keys grouped by the file holding them, each
file entered once, every request bound in it (the ones with no key there empty), one run of the program, each
request given the rows its keys select. The cone thread holds a callable's posted requests, not answering each as it
comes: when the run is about to wait (`Asks::waits`, a marker posted once after posts, before the run blocks), every
held request is answered in the one call, and each ticket gets its rows. The run posts its requests, so they
accumulate, in both its loops: the scoreboard's, and the serial loop of a run whose strata are kept across runs
(decision 0077 §4), which had waited for each; there a stratum still asking is not taken as evaluated until it has
resumed, and its posted answers are logged with its stratum's reads (`Run::logs_posted`). A produced relation's
stratum (decision 0065 §2a) posts, with its own request, the requests of every produced relation whose request is
made and not yet answered, and waits once for them all (`posted_together`). Two callables, each routed, asked in one
program (`callable_tests::two_callables_asked_together_are_answered_in_one_call`): 1 batch, 24 requests, 2 files
entered (2 batches, 4 files entered before), the findings and both readers' rows the whole program's. PostHog's 291
files, get-handler-writes (one callable): 1 batch, 2,260 requests to 163 files, 3 holders entered, 1,193 answer rows,
the same 5 findings. Tests: `rerail-tests-rr` (298 passed), `calls::tests`, `answers::tests`, `complete_answer_tests`.

**The cone's relations handed on in place.** The memory the call cost was two copies (decision 0353) of what the
cone reads and makes, neither on 0353's list nor marked. At the cone's end, each relation the rest of the cross-file
part reads was copied out as rows (`held_rows`, a heap vector a row) and copied again into columns for the bind; it
is now handed on as the engine holds it (`held_source`: `component::derived`, the relation itself), or as the rounds
hold it (`Retained::derived_source`, the kept relation or the frames' block, `InRel::source`); only answers and the
rounds' views, which are rows when made, are made once into a relation. And the digest of each document's part of
what the cone reads (`cone_digests`, the cone's reuse) collected every row of those exports per document, copied, to
sort and hash them: it now reads them where they lie, twice, the document's nodes first (a node is digested as its
rank among them) and then each row's hash summed per document, an order-free digest of the document's rows (the
digests differ from the last run's once). PostHog's 291 files, get-handler-writes: peak memory 614.9 MB, against
1,220.5 MB with the copies and 569.2 MB with no call; 795.3 G instructions against 780.4 G with no call (+1.9%); the
same 5 findings.
Calls in bulk had posted a request once per produced relation of it that was ready (`posted_together`): a request
producing several, each its own stratum (the C preprocessor's `c_pp_tu_req`), was asked again for each (`corpus::tests::
a_header_edit_asks_again_only_for_the_units_that_read_it`: each unit 7 times); each request is posted once now, and a
produced relation once asked is not asked again. Tests: `rerail-tests-corpus` (281 passed, with the coverage ledger's
one more `to_number`, the router's, and `RERAIL_ASKS` classified a measurement aid), `rerail-tests-rr` (298).

**The call's two hot spots, made cheap.** A profile of the call against no call (PostHog, get-handler-writes, each
span's own CPU) found its extra cost in two places. The cone's digests (`cone_digests`) read each export a value at
a time, a part search for every value, and hashed each row with a SHA-256 block: it now reads part by part
(`Exports::visit`, `Rows::each_run`) and hashes a row with SipHash-1-3's 128 bits under fixed keys (77 samples, from
119). And what a callable reads of a file (`pt_callee`, `pt_tm`, `pt_vpos`: 552,420 rows of `pt_tm` alone), held in
every file's entry so that a later run's call into a file it does not build finds its rows (`calls::Held::add_entry`),
was gathered by file into rows of its own before the entries took a second copy of them (`by_file_of`, decision 0705's
held relations with it): 0.151 s and 239 MB a cold run. It is now written into the entries once, where it lies
(`held_into_entries`): the location column counted first, each file's rows made at their size, each run of a
document's rows appended to its files' rows, which move into the entries: 0.023 s and 2 MB. The same 5 findings. Two
runs of the same program differ by 6.4 G instructions and 43 MB of peak, more than either change, so neither is
claimed in the run's totals (785.8 G and 792.3 G instructions, against 780.4 G with no call). Tests: `rerail-tests-corpus` (281
passed), `rerail-tests-rr` (298), `calls::tests`, `answers::tests`, `complete_answer` (14).

**The inputs' frames: the cone's relations, kept as inputs.** The call's remaining cost in the inputs' frames (+0.149
s, 52 more jobs) is points_to's cross part having 63 more inputs (378,613 rows): 224 without the call, 286 with it.
They are no new exports (the split exports 189 relations, against 188). They are the cone of requests' relations
(decision 0065 §2a): `pt_vopen`'s request is asked with the column `pt_ext_attr` binds, and that relation's cone is
much of the points-to solve (`pt_found`, `pt_pv`, `pt_asks`, `pt_look_t` and the rest), which the cone derives and
hands to the rest of the cross part as its inputs (`loading_rows`, bound with `cross_inputs`). Without the call the
cross part derived them itself, and kept each as a derived relation's one entry (452 relations handed off, 0.119 s);
with it, 236 are handed off (0.088 s), and the cone's are kept as inputs are: by granule, digested by document
(`an input's frames`, `digest inputs`). The solve's rows are kept once either way, in a costlier form with the call.
An input's frame found each row's owner, its granule (an atom owner's content key, a search of the atoms) and its
place in a map of (granule, owner), twice a row (`picked_ids`, about 165 of the jobs' samples): it now does each once
a run of one owner's rows, a granule once an owner (12 samples). The inputs' frames: 0.261 s without the call (0.400
s before), 0.445 s with it (0.598 s before); the same 5 findings. Tests: `rerail-tests-corpus` (281 passed), `rerail-tests-rr` (298).

**A warm edit with the call.** PostHog's 291 files, get-handler-writes, a cold run and then one file edited
(`posthog/api/user.py`, a function appended that reads `request.user.email`), in one store: the edit took 109.1 G
instructions with the call, 91.7 G without (+19%; cold, 788.7 G against 779.9 G), the same 5 findings each time. Of
its +1.0 s of span CPU: the cone of requests derived its recursive stratum again whole (66 relations, `pt_alook`,
`pt_found` and the rest: the cone's kept strata, decision 0077 §4, skip a stratum only when all it reads is as it
was; 98 of 119 strata skipped, 21 derived again), +0.68 s; the requests' side on the main thread, +0.46 s (the
calls asked again of all 163 files, about 0.15 s of it); the cone's relations' delta found from their input frames
(their changed documents' old rows read, the other owners' rows hashed), +0.34 s. The rest of the cross part did
less, −0.47 s: without the call the solve is in it, maintained from deltas (87 of 255 strata run, 57 from deltas).
