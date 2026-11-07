# 0044: A run starts from seeded facts; everything else is derived

Status: accepted (user agreed, 2026-11-06); step 1 built for library files

## Intro

A rule run needs many facts: which files exist, what each contains, its parse tree, its module
name, which library rules apply, and the rules' results. Today some of these are recorded as
facts with hashes and dependencies, and some are made by Rust code with no record at all.

This record makes every layer a fact derived from the layer below. At the bottom are a few
**seeded facts**, the only things a run is told rather than computes:

```
// seeded: what the repository and the engine hand the run
file("impl/common/query.py", "sha256:9f2c…").       // a workspace file and its content hash
library_file("lib/python/names.rr", "sha256:41ab…"). // a rule library file and its content hash
config_file(".swallowed-exceptions-ignores.json", "sha256:77d0…").
grammar("python", "tree-sitter-python 0.23.6").     // what parses a language

// derived: each from the layer below, recorded with the hashes it read
tape("sha256:9f2c…", "python", "tape:3e1a…").       // parsing depends on content only
module("impl/common/query.py", "impl.common.query"). // naming depends on the path only
```

What this buys:

- **One answer to "is this stale?"** A fact is stale exactly when a hash it was derived from
  changed.
- **Reuse across paths and branches.** Facts that depend on content alone are keyed by content,
  so a file copied, renamed or seen on another branch reuses them.
- **Rules are inputs like files.** Editing a library file invalidates what read it, and nothing
  else.

## What there is today

The push path is already most of the way there (`facts/families.rs`, `content_store/derivation.rs`):

- A Git tree gives `{scope}#path:{p}` bindings to object digests (`facts/observe.rs:12-42`).
- `#tape:`, `#core:`, `#prefilter:` and `#kinds:` are recorded derivations: (output, producer,
  inputs with their digests, output digest).
- `propagate` recomputes what read a changed name, wave by wave, and stops where an output digest
  is unchanged (`facts/propagate.rs`).

The gaps:

1. **Rule files are not inputs.** `.rr` files are read from disk and never hashed
   (`logic/rr.rs:32-79`). The only identity is the lowered program's SQL digest
   (`waves/schedule.rs:93-105`), so a library edit is seen only by comparing SQL.
2. **Two entry paths.** The corpus runner (`logic/plan/corpus.rs`, used by the replay and the
   measurements) reads files with `read_dir` and numbers documents by path order, with no store
   and no hashes.
3. **Node ids follow paths, not content.** A document's id `d` is the atom of its path name
   (`waves/matches.rs:556`), and a node is `d << 32 | preorder`. Identical content at two paths
   shares nothing, and a rename invalidates.
4. **The base relations are one Rust step.** `Batch::build` (`facts/vtab/syntax.rs:304-464`)
   makes `doc`, `pkg`, `path_part`, `file_name`, `text`, `name`, `str`, `value`, `op` and `op_at`
   in one pass per batch, with module and package naming hard-coded per language (`:49-67`).
5. **Rule evaluation is recorded per batch.** A warm run evaluates only changed documents (0014),
   but the derivation lists every document's `#core:` binding as an input
   (`waves/schedule.rs:362, 446-476`).
6. **Two hash kinds.** Files are keyed by Git oid; content by sha256, with an alias table between.

## Decision

### 1. Six layers, each a fact family

| Layer | Facts | Keyed by | Made by |
|---|---|---|---|
| L0 seeded | `file(path, hash)`, `library_file(path, hash)`, `config_file(path, hash)`, `grammar(lang, version)` | path | the Git tree or the file system; never computed |
| L1 content | `content(hash, bytes)`, `tape(hash, lang, tape)`, the tree relations (`kind`, `child`, `field`, `text`, `op`, `op_at`, `tape`) in **local** ids | content hash (+ grammar) | the parser and `Batch::build`'s content half |
| L2 path | `doc`, `module`, `pkg`, `path_part`, `file_name` | path | `.rr` rules over `file` (moved out of Rust) |
| L3 program | parsed library, lowered plan | the library files' hashes | the reader and the planner |
| L4 evaluation | each stratum's relations | (stratum digest, the hashes it read) | the waves |
| L5 findings | `(path, line, code)` rows | the findings layer's inputs | the findings layer (0013) |

- **One hash for a file's content.** The seeded fact carries sha256 of the bytes. A Git oid stays
  as an alias the observer uses to skip unchanged subtrees, not as the key.
- **Every fact family is a derivation** in the existing tables (`name_bindings`, `derivations`,
  `derivation_inputs`), including L0 → L1, L3 and L4. Nothing is computed outside one.

### 2. Content facts use local ids; documents map them

- In L1, a node is `(tape, preorder)`. Nothing about the path is in it.
- A run gives each document a dense number `d` and keeps the same `d << 32 | preorder` global ids
  for joins (`within` stays a range test). The mapping `doc_tape(d, tape)` is an L2 fact.
- So content facts are shared by every path with that content, and only the small `doc_*`
  mapping changes on a rename.

### 3. Path facts are rules

`module`, `pkg`, `path_part` and `file_name` move from Rust (`syntax.rs:49-67`) to `.rr` rules
over `file(path, _)` in `core.rr`'s library, per language. They are cheap, and as rules they are
visible, testable and changeable without a rebuild.

### 4. The program is a derivation of the library

- `library_file` hashes are inputs to the reader. The parsed and lowered program is cached by
  them (and by the engine's version).
- A library edit invalidates the lowered program and, through its digest, the strata it changed.
  The SQL digest stays as early cutoff: a comment-only edit lowers to the same SQL and stops there.

### 5. Evaluation is recorded per stratum and per scope

- **A file-local stratum** (it reads one document's facts only: matchers, `inside`, per-file
  typing) is keyed by (stratum digest, tape hash). Its rows are reused for any path with that
  content, on any branch. This is where content keys pay off.
- **A cross-file stratum** (imports, exports, layering) is keyed by (stratum digest, the hashes of
  what it read). It re-runs by dependency, with digest cutoff (0035's rounds unchanged).
- Each file owns its rows, so an edit deletes and rewrites only that file's rows (ledger row 14).

### 6. One entry path

The corpus runner goes through the store: it seeds `file` facts from the directory, then runs
like the push path. The replay and the measurements then exercise the code that ships.

## Not decided here

- The id scheme's details beyond §2 (how dense `d` is assigned across warm runs) depend on 0045's
  set containers, which are keyed by document. The two are settled together.
- Whether L4 rows of a file-local stratum are kept for content not currently in the workspace (a
  branch switch back) is a cache policy, under the 10 GB disk budget.

## Steps

1. `library_file` and `config_file` as seeded facts; the program cached by them (§4).
   *Library files done:* `rr::read_file` returns each file a program read, with the sha256 of its
   bytes, and each place an `#include` looked and found nothing. The lowered program is kept while
   all of them hash the same (`programs_are_kept_by_the_hashes_of_what_they_read`). The cache is
   per process; it goes into the store with step 2. *`config_file` waits for O76:* the runner
   refuses to read `.input` files today, so there is nothing to hash yet.
2. The corpus runner seeds `file` facts and reads through the store (§6).
3. L2 path facts as `.rr` rules; `Batch::build` keeps only the content half (§3).
4. Local ids in L1 with `doc_tape` (§2): measure the join cost of the mapping on the 9,010-file
   repository against today's ids.
5. Per-stratum derivations; file-local strata keyed by tape hash (§5): measure warm-run reuse on a
   branch switch and on a rename.

Each step keeps the replay and the converted gate unchanged.

## Prior work consulted

- **Spike design index §20 and topic 10, "Incremental indexing allows no corpus-wide statistic"
  (DESIGN S124, L512-592; S130, L593-622).** *Agrees.* Per-document facts and per-document
  derivations; no layer here needs a statistic over the whole corpus to be maintained.
- **Spike history index 2.20 (DH L18-26, L3319-3357, L8572-8578):** load repeatable and
  indistinguishable from one call; process-held state rebuilt with a fingerprint; layout
  parameters persisted. *Agrees.* §1's derivations are the fingerprints, and §4 persists what the
  program was built from.
- **History index line 874 and S235/S237 (DH L6705-6758): content-addressed relations,** "name
  relations by content after their dependencies are renamed". *Agrees, and extends it* from rule
  relations to every fact family, file facts included.
- **Lessons ledger row 14:** each file owns its rows; whole-graph artifacts must not force
  whole-repository rebuilds. *Agrees* (§5). Row 10, "measure the code that is actually running":
  *agrees* (§6, one entry path).
- **0014, 0029, 0032, 0035.** *Kept.* Changed documents only (0014); one derivation per document
  over its core name (0029), now over its tape hash; tapes keyed by tape digest (0032), now the
  L1 key throughout; cross-file rounds with digest cutoff (0035).
- **Data flow and GPU** are the spike's known gaps; it has no evidence on layered derivations
  beyond indexing.
