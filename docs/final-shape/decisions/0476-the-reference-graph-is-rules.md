# 0476. The reference graph is rules: `python/references.rr`, per file and then linked

Status: accepted (2026-11-19). Follows 0466 and 0472 (their next step "the summary as a tape pass").

## Intro

`reference_edge(src, dst, via)` and `reference_open(n)` are the Python reference graph: which definition's body
refers to which definition, in any file, and which definitions refer to something no static name denotes. Dead-code
checks walk it. Until now the engine made them in Rust:

- **In the parse workers.** `logic::plan::corpus::references` ran W's call summary (`calls::summarize`) on each
  file: a scope table (`scope::python`, Ruff's model) and W's walk (`calls::summary::python`, 1,665 lines), which
  writes each reference as a marked name (`m.<bare>.f=>m.f`, `C.<self>.run`, `m.<type>.T.x`).
- **After them.** W's linker (`calls::closure::Graph::link_references`) resolved the names across files.

On all (3,089 files) that was 5.9 s of CPU, 1.9 ms a file, six times the parse (0466, 0472). The user asked: "please
fix it. however, shouldn't this be implemented in .rerail files? or at least, use all of those mechanisms, including
AST matchers, and the per-file fixpoint executor to get its information out? having a hand-rolled Rust Python
extractor feels wrong".

It is wrong, for three reasons:

- **The engine already has Python's names as rules.** `python/names.rr` is the scope lookup the logic engine's
  matchers use: `binds` lowers to it (`json_rules::lower`). `python/imports.rr` handles imports, relative imports
  and re-exports. W's scope table is a second model of the same thing, and the call summary a third.
- **The graph's two parts are what the engine's run already splits.** A file's references are a function of the file
  alone: the program's file-local part, evaluated in the chunks and memoised per file (0052). Linking names across
  files is the cross-file part.
- **A Rust walk is the shape the engine is built to replace.** Its reads go through ast-grep's node API, one node at
  a time, and its interned names wait on a process-wide table (0466).

## Decision

1. **`lib/python/references.rr` derives `reference_edge` and `reference_open`.** `core.rr` declares the two
   relations but no longer marks them `.input`; a program that reads them includes the library. The library builds on
   `python/names.rr` and `python/imports.rr`.
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
| R1 | The library derives both relations; `core.rr` declares neither as an input | the converter's programs and the prelude include it; `check-rr` on the library |
| R2 | The extraction is file-local, the linking cross-file | the planner's split of a program reading the graph: every extraction relation in the chunks |
| R3 | Rows equal W's on the fixtures and the projects, every via label and open definitions included | `reference_edge_is_ws_reference_graph_on_the_fixtures`, `…_on_projects` |
| R4 | Rows equal W's on all | the probe's row diff (below): no row missing or extra |
| R5 | The `.references` setting's sibling links, constructor fields and scope act as in W | the fixtures' events that set them (R3) |
| R6 | The engine's run makes no summary | `references.rs` deleted; no caller of `calls::summarize` on the run's path |
| R7 | The findings on all are unchanged | rows ed502024, native and Metal |
| R8 | The parse workers' CPU on all is measured before and after | this record's Measured section |

## Measured

To be filled as the steps land.
