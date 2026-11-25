# 0693. A walk across files is over the records' names

Status: decided (2026-11-25), by the user's plan for the rules the position report flagged after 0665, step 4: "E and
F, read against the originals." It follows 0692 (plan-helpers-allowlist, versioned-merged-identity) with
pure-lazy-runtime-purity, which 0686 left open ("purity-calls' `pc_tgt` holds pure-lazy-runtime-purity's analysis
cross-file"). It amends the split's keys (0385), flags applied last (0686), and the data-layer's purity call graph.

## Intro

pure-lazy-runtime-purity analyses the defs a closure over the call graph reaches: from the `@pure_lazy` and `@lazy`
defs, through the calls each makes (to a def of its file, an imported one, a method of its class or a base), stopping
at a `@legacy_*` def. The analysis of each def is its own file's: the body walked in `ast` order, a local tainted
before a read (`plp_tnt_at`), an opaque assignment before a use (`plp_opq_before`), breadth-first order (`plp_bfs`).
Those read positions. Only the choice of defs crosses files.

The port keyed the call graph by node: `pc_tgt(f, g, via)`, the def `g` a call of `f` reaches, in whichever file.
So every relation the closure reached was cross-file, the analysis with it, and the cross-file part read positions.

The linter keys its records by name. A record is `module::qualname` (`symbol_id`, a dict), a call resolves to such an
id (`_resolve_fqname`, `_resolve_method_target`), and both closures (`_pure_lazy_closure`, `_runtime_owner_closure`)
walk ids. A node never leaves its file there. The port now does the same, and this is also the shape the node-ID
invariant asks for: a walk across files carries names, and each file reads back its own records'.

## Decision

### 1. The call graph is over the records' ids

purity-calls resolves to ids:

- `pc_sid(s, f)`: record `f`'s id, `module::qualname`. `pc_qof(t, q)`: id `t`'s qualname. `pc_fqid(p, t)`: the record
  of `module.qualname` `p`.
- `pc_res(s, t)`, `pc_method(m, cq, x, t)`, `pc_mans(m, cq, x, t, via)` and `pc_tgt(f, t, via)` give the callee's id.
  `pc_edge(f, t, via)` is record `f`'s call of id `t`. `pc_sedge(s, t, via)` is the same between ids.

### 2. The walks are over ids, the step between files declared

pure-lazy's walks read names alone:

- `plp_shared(s)` (scan_paths' narrowing), `plp_colored_id(s)` and `plp_owner_id(s)` are sets of ids.
- Each seed is one record in its own file: `plp_colored_id(s) :- pc_sid(s, f), plp_in_rec(f), plp_explicit(f)`.
- A step is `plp_step(s, t)`: record `s` calls id `t`, whose record is in a file scan_paths reads
  (`plp_id_in(t, d)`) and is no boundary. It joins a call in one file to the record it reaches in another: a crossing
  meant, declared `cross_file` (0652).
- Each file reads its own records back: `plp_colored(f) :- pc_sid(s, f), plp_colored_id(s)`.

The walks hold names and read no relation of their readers, so they are staged (0675): a stage over every file, its
ids read by each file's part as settings. `plp_an`, `plp_in` and everything under them are per-file.

A closure in the old port was cross-file only by reading the flag `plp_docin`, and that hid its own join of two files
(0652, "a crossing can hide behind another"). Applying the flag last (0686) showed it, and flags-last refused the
flag ("plp_colored_id would join documents undeclared"). The step is that join, named and declared. The seeds read
one file each for the same reason: an id from one file's seeds joined with another file's records by name is a
crossing too.

Two records of one id (two files of one module name and qualname) are one to the walks, as they are in the linter,
whose dict keeps the last: a seed or a boundary in either holds for both, where the linter reads the last one's.

### 3. PLP-007's resolution through re-exports is over names too

PLP-007 resolves what a `data_layer/*.py` module exports, following re-exports, to the def (`plp_xq`, `plp_xres`). It
gave a node in another file. Now:

- Each file gives names at its own document: `plp_fqin(s, t, d)`, the record of id `t` has `module.name` `s` in a
  file scan_paths reads; `plp_reexp(s, q, d)`, module file `d` re-exports `s`'s name from `q`; `plp_xseed(s, d)`, data
  layer module `d` exports `s`.
- The step is `plp_xstep(s, q)`: no file scan_paths reads defines `s`, and its module re-exports the name from `q`. A
  module's import joined to every file's records, declared `cross_file`.
- `plp_xq(s)` and `plp_xres(s, t)` walk names, and the finding is read back at the record's own node
  (`plp_xres(_, t), pc_sid(t, g)`).

With the findings per-file, the plugin's allowlist reads them across files: `plp_list_error`, an entry no finding in
any file matches, compares the whole list with every file's findings. It is declared `cross_file` (a join product
demand pushed into the files' part for it was refused by 0665's check, its rows read back from their located form).

### 4. A flag's choice reads a document's id as it is

A cone's relation read outside it is chosen by the flag at its location column: through `docof` when the column is a
node, and as it is when the column is a document's id (0686). Flags-last told the two apart by the column type
`number`. The image spells a declared `number` as `int`, so a document's id was read through `docof`, which never
matches one: `plp_id_in` was empty under the flag, and the walks reached nothing. A location column that is not a
node is a document's id now, in the cone's check and in the rewrite.

### 5. The split reads the outputs

An output is never covered, staged or a guard's to hold, so the split depends on which relations are outputs. Its
`READS` did not name them, nor did its keys (0385): an edit that set the outputs alone (a part's fields,
`passes::PartOf`) carried a split made for other outputs. The debug build's check caught it on pure-lazy ("analysis
file-local split carried over part's fields's edit (Reads(16)) is not what it computes"). The split now reads
`OUTPUTS`, and its global key hashes the outputs, so such an edit splits again.

## Tests, written first, one per requirement

| Requirement | Test |
|---|---|
| pure-lazy finds what its linter finds over a closure across files: a seed through an imported helper (PLP-003), through `self.` to a base class's method in a file that names a trigger (PLP-001), stopping at `@legacy_sync`, and a def nothing reaches unanalysed | `pack_split_tests::pure_lazy_closes_over_the_calls_and_analyses_each_file_on_its_own` |
| Its analysis (`plp_an`, `plp_in`, `plp_tnt_at`, `plp_opq_before`, `plp_bfs`) is per-file in every split | same test |
| PLP-007: a public `*_lazy` def a data-layer module exports, directly or through a module that re-exports it, is reported at its def, and `plp_find` and `plp_kept` are per-file | `pack_split_tests::pure_lazy_closes_over_the_calls_and_analyses_each_file_on_its_own` |
| A cone's relation placed by a document's id and read across files at an open location is chosen at that id: a run's rows are the original program's, and an edit turning the flag over gives a fresh run's | `corpus::tests::a_flag_over_every_file_is_applied_last` (`named_in`, `both`) |
| A split is not carried over an edit of the outputs | the debug build's check of every carried analysis, on the first test |

## Results

- The position report (every data-layer policy over the 334-file corpus, every pack over a small corpus, one Linux
  host, every run completed): 23 flagged rules after 0692, 16 now, none newly flagged. pure-lazy-runtime-purity has
  none: its cross-file part reads no position. `plp_bfs`, `plp_opq_before`, `plp_tnt_at`, `plp_found`, `plp_report`,
  `rr_ctx58` and `rr_demand_agg_rr_agg57` are per-file, and its flag `plp_docin` is applied last as in 0686 (12
  relations under each value; `rr_agg52` stays per-file).
- pure-lazy over 120 files of the 9,010-file tree, old text and new, one Linux host: the same 6 findings (PLP-005,
  PLP-006, and the allowlist's absence). Cold, 11.7 s and 9.2 s wall, 35.4 s and 30.1 s CPU; warm, 0.16 s either way.
  Over all 1,560 files both texts pass the run's 4 GB cap.
- The 36 pack split tests, the deferred and split tests, and `rerail-logic`, `rerail-typing`, `rerail-rr-root`,
  `rerail-corpus-prep`, `tests_logic`, `tests_corpus`, `tests_rr`, `tests_lang`, `tests_findings` and the device
  crate's tests pass (1,499). The device crate's two GPU tests that fail on a Linux host fail as in 0686.
- `pack-python-main` was once killed at the report's 120 s cap. It takes about 100 s, nearly all of it preparing the
  program from a cold image store, with the split's new keys or without them (98.5 s and 104.6 s): it sits at the cap.

## Left open

- `pc_mans`, `pc_fqlen`, `pc_first` and `pc_sedge` are not staged on their own: a number column (`via`, a base's index)
  under rules that read positions may be one. They are inside the walks' stage, and nothing per-file reads them.
- A memory cap for pure-lazy over 1,560 files: both texts pass 4 GB.
- `pack-python-main`'s cold preparation of about 100 s.
