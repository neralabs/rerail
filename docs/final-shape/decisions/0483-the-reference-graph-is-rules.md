# 0483. The reference graph is rules: `python/references.rerail`, per file and then linked

Status: accepted (2026-11-19). Follows 0476 and 0477 (their next step "the summary as a tape pass").

## Intro

`reference_edge(src, dst, via)` and `reference_open(n)` are the Python reference graph: which definition's body
refers to which definition, in any file, and which definitions refer to something no static name denotes. Dead-code
checks walk it. Until now the engine made them in Rust:

- **In the parse workers.** `logic::plan::corpus::references` ran W's call summary (`calls::summarize`) on each
  file: a scope table (`scope::python`, Ruff's model) and W's walk (`calls::summary::python`, 1,665 lines), which
  writes each reference as a marked name (`m.<bare>.f=>m.f`, `C.<self>.run`, `m.<type>.T.x`).
- **After them.** W's linker (`calls::closure::Graph::link_references`) resolved the names across files.

On all (3,089 files) that was 5.9 s of CPU, 1.9 ms a file, six times the parse (0476, 0477). The user asked: "please
fix it. however, shouldn't this be implemented in .rerail files? or at least, use all of those mechanisms, including
AST matchers, and the per-file fixpoint executor to get its information out? having a hand-rolled Rust Python
extractor feels wrong".

It is wrong, for three reasons:

- **The engine already has Python's names as rules.** `python/names.rerail` is the scope lookup the logic engine's
  matchers use: `binds` lowers to it (`json_rules::lower`). `python/imports.rerail` handles imports, relative imports
  and re-exports. W's scope table is a second model of the same thing, and the call summary a third.
- **The graph's two parts are what the engine's run already splits.** A file's references are a function of the file
  alone: the program's file-local part, evaluated in the chunks and memoised per file (0052). Linking names across
  files is the cross-file part.
- **A Rust walk is the shape the engine is built to replace.** Its reads go through ast-grep's node API, one node at
  a time, and its interned names wait on a process-wide table (0476).

## Decision

1. **`lib/python/references.rerail` derives `reference_edge` and `reference_open`.** `core.rerail` declares the two
   relations but no longer marks them `.input`; a program that reads them includes the library. The library builds on
   `python/names.rerail` and `python/imports.rerail`.
2. **The file's part is file-local rules**, over the tape's relations and matchers: the definitions and their
   qualified names, each definition's references with how each is made, and the names the file binds for others to
   read. The planner places these in the chunks. The linking part is cross-file rules: names to definitions,
   re-exports, the bases breadth first, declared parameter and field types, and the sibling links.
3. **The `.references` setting becomes facts.** The engine reads the setting as before (sibling links, constructor
   fields, scope) and hands its parts to the rules as facts. It computes nothing from the corpus.
4. **The graph is W's.** For every program, on the replay's fixtures and the projects
   (`reference_edge_is_ws_reference_graph_on_*`), and on all, the rows equal W's. W's own evaluator keeps its
   summary: it is the oracle these tests compare against.
5. **The engine's run calls no summary.** `logic::plan::corpus::references` is deleted.

## Requirements and their tests

| | Requirement | Test |
|---|---|---|
| R1 | The library derives both relations; `core.rerail` declares neither as an input | the converter's programs and the prelude include it; `check-rr` on the library |
| R2 | The extraction is file-local, the linking cross-file | the planner's split of a program reading the graph: every extraction relation in the chunks |
| R3 | Rows equal W's on the fixtures and the projects, every via label and open definitions included | `reference_edge_is_ws_reference_graph_on_the_fixtures`, `…_on_projects` |
| R4 | Rows equal W's on all | the probe's row diff (below): no row missing or extra |
| R5 | The `.references` setting's sibling links, constructor fields and scope act as in W | the fixtures' events that set them (R3) |
| R6 | The engine's run makes no summary | `references.rs` deleted; no caller of `calls::summarize` on the run's path |
| R7 | The findings on all are unchanged | rows ed502024, native and Metal |
| R8 | The parse workers' CPU on all is measured before and after | this record's Measured section |

## Measured

`all`, native, cold (the cost store, the image store and the tape cache fresh for each run), `--profile profiling`,
on this machine (Mac17,6, an M5), at load 6–15. "Before" is W's Rust summary and linker (the commit before
de541884e); "after" is `python/references.rerail` on the run's path. The program is `internal_dead_code` in both runs,
with the library included after. Every output relation's row count is equal (one new relation, `rr_embed_req`, holds the
library's 4,323 requests to parse string annotations and is no finding).

| Run | CPU s | wall s | instructions | peak MB |
|---|---|---|---|---|
| before (W's Rust) | 38–40 | 9–12 | 495 G | — |
| after, first run | 338.5 | 75.5 | 4,924 G | 2,007 |
| after, `rf_forward` from the annotations, `rf_dec_code` over decorators | 131.3 | 26.9 | 1,910 G | 2,002 |
| after, a guest round keeps the files' matcher rows | 134.6 (load 15) | 36.0 | 1,820 G | 1,983 |
| after, `rf_in_annot` down from annotations, `tp_ref` gated by `tp_doc` | 114.2 | 25.3 | 1,663 G | 1,988 |
| after, `param_list` (a function's or a lambda's) | 105.5 | 24.1 | 1,599 G | 2,002 |

What each step removed:
- **`rf_forward`**: the first run spent 173 s of 251 s of join CPU in one rule. It joined the module's latest binding of
  each name with every identifier of its document and then tested the text (1.15 G rows, none kept). The rule now
  starts from the annotations' names (`rf_annot_head`).
- **`rf_dec_code`**: the `min` over a decorator's children ranged over every node's children: 14 s.
- **The guest rounds** (decision 0055): each round that made guests (the string annotations) built the batch again
  and ran the ast matcher wave again over every file of the chunk: 17 s. The wave reads no guest, and a node's id is
  its document's key and local id, so `with_matchers` now takes the rows of the build before (`kept`). The rest of
  that rebuild, projecting the nodes again (about 6 s), remains.
- **`within(i, a)` from every name** (`rf_annot_head`, `tp_ref`): each walked every name's ancestors to find a few
  annotations or PEP 695 definitions. They now walk down from the annotations (`rf_in_annot`), or test only the
  documents that declare type parameters (`tp_doc`).
- **`param`**: the prepared plans start a rule at `kind(p, "identifier")` (3 M rows) rather than at
  `field(f, "parameters", ps)`. `param_list` names the two kinds that have the field, so the plans start at the
  functions.

**Where it stands: after costs about 2.7 times before's CPU (105 s against 38–40 s).** The rest is no single
explosion. It is the volume of the scope rules `python/names.rerail` derives for every name in the corpus (`walk`,
`wstart`, `nameref`, `bind_only`, `rf_bind`, `rf_in`: 1–2 s each, 7–19 rows a row kept), at the engine's 70–100 ns a
join row. W's Rust walked each file once. Open:
- the cross-file `reach_edges` rule of the program (6 s): the planner orders `structural_protocol` last on equal
  sizes;
- `rf_seen_at`, which starts from its demand (2.2 M rows) rather than from `rf_value_last` (17 k);
- the projection a guest round makes again.

**Metal (R7).** At `--metal-mb 2048` the rows equal the before run's (plus `rr_embed_req`): after, 76.3 s of CPU,
26.3 s of wall and 1,123 G instructions; before, 37.0 s, 31.6 s and 478 G (load about 5). At the default 512 MB the
run fails: the derived relations' row blocks hold the Metal budget, and the ast matcher wave is refused (Q413).
