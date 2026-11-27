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
| Asks (relation, reader, shape) | 425, over 260 relations | 37, over 24 relations |
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

