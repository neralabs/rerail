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

**The real cases, by the candidates' line** (the CLI's debug build, cold, on fresh stores):

- The data layer's 224-file sample, `bandit-b113` (17.5 s user CPU, 718 MB): none of `python/names`' 20 exports.
  `imported_at`, `reads_at_at`, `tc_only_at`, `fwd_any_at` and `fwd_rt_at` (12 k to 17 k rows) are each read by
  their relation's rule with the file unbound; the factorized `sym_at`, `asks_at` and `spelled_at` (15 k to 19 k)
  have no file in column 0.
- The PostHog sample's 291 files, Django's three policies that read points_to (68.4 s user CPU, 1,516 MB): none of
  `points_to`'s 188 exports, nor any other phase's or the policies'. The largest (`pt_at_at` and `pt_chk_at`,
  118,867 rows; `pt_vopen_l_0_at` and `pt_vpos_t_at`, 106,654; `pt_look_t_at`, 93,224) are each read by their
  relation's rule with the file unbound. Of the policies' own, the largest (461,298 rows) have no file in column 0,
  or readers that bind only the file.

In both, the cross part reads a file's exports by symbol, module or definition (decision 0677's keys), and the read
is what finds the file: no reader knows the file before it reads. The spike's call, a reader binding the file and
more, is the generated corpus's shape, and neither real program has it.
