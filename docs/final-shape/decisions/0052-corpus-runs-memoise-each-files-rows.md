# 0052: A corpus run memoises each file's file-local rows; an unchanged file is neither parsed nor evaluated

Status: accepted (2026-11-06; built and measured)

## Intro

After decision 0051, a warm full-corpus run of every rule still took about 49 s of CPU. Almost all
of it was the file-local part: the rules evaluated per chunk of files. That part is a function of
each file alone, so its result for an unchanged file is the same as last time. Only the cross-file
part, which took 0.01 s, needs every file.

This decision keeps each file's file-local result and replays it. A run then evaluates only the files
that changed, plus the cross-file part.

## Decision

### 1. What is memoised

For each file, the memo holds what the chunks hand on to the cross-file part:

- its rows of each exported relation;
- its lines of each file-local output;
- its node count, for the run's report.

The store's facts memo holds these entries (`content_store::facts_memo`, built for "one clause's
per-file results under content, plan, schema and path"). An entry's key has three parts:

- the file's stored source (its content);
- its path;
- a plan: the engine build (`RERAIL_ENGINE_BUILD_ID`), the grammar version, a digest of the
  file-local program as lowered, and the file's document key.

A rebuilt engine, a changed rule, a new grammar or a moved file therefore misses. A memo row dies
with its source, as the facts memo provides.

### 2. Every row names its file

A file's rows can be separated only if each row says which file it came from. The split already
computes a relation's location columns: columns every rule fills from a node or a document. A row's
file is read off its first location column. With document keys below 2^31 and node ids at
`key << 32` or above, that's `v >> 32` for a node and `v` for a document.

Relations without a location column get one (`file_local::locate`). Each such relation that the
chunks hand on is derived by them as a twin with a leading document column. The document comes from
a column the rule binds (`node`, `doc` and the like), or from a node it binds, through
`node(d, n, …)`. The cross-file part then reads the twin with that column ignored:

```text
// The chunks derive the twin; its first column is the row's document.
fl_v_at($d, $p, $l) :- node($d, $n, …), doc($d, $p, _), line($n, $l).
// The cross-file part drops the document again.
fl_v($p, $l) :- fl_v_at(_, $p, $l).
```

The memo is off for a run when any of these hold:

- a relation can't be given a document (file-local rules also read it, or it is a lattice);
- the cross-file part reads base relations, since their rows come only from evaluated chunks;
- the run reproduces T's facts, or uses typing.

The profile says which applies.

### 3. Values that mean the same in every run

A replayed row is inserted as it was stored, so every value in it must mean the same in the later
run:

- **Document keys** come from the path's hash, in `1..2^31`, taking the next free key on a
  collision. They used to be positions in the sorted file list, which shift when a file is added.
  The key is part of the plan, so a key that a collision moved only misses.
- **Nodes** are `key << 32 | index`, with the index taken from the tape. The tape is a function of
  the content and the grammar.
- **Kinds** are grammar codes.
- **Symbols** are atoms of the persistent store, which never gives an atom out again (decision
  0049). The run interns into that store. Its SQL connection (a fresh `store.sqlite`) shares the
  store's arena in the process (`Atoms::share`), so `sym_text` and `sym_atom` agree with it.
  Earlier, the run's atoms were its own, so a symbol's id changed every run. Storing symbols as
  text was tried first, but kinds and atoms share a type and overlapping values, so it can't be
  done by column type.

### 4. The run

Before parsing a file, the worker reads and hashes it, then looks it up in the memo:

- **A hit** isn't parsed. Its entry goes to the main thread with the next chunk. There its rows are
  inserted into the cross-file tables and its lines into the outputs, with the statistics an
  evaluated chunk would add.
- **A miss** is parsed and batched as before. After its chunk runs, each of the chunk's files gets
  its entry, an empty one included.

`RERAIL_CORPUS_MEMO=off` turns the memo off.

## What the prior work says

- "Key memoized analysis by what it is a function of (pattern, language), never by object
  identity" (history index §2.16(d)), and "Language must be part of every cache key" (S238). The
  key here is content, path and plan; the plan carries the engine build, the grammar version and
  the program. No run-specific id is in it: document keys come from paths, and atoms are durable.
- "Representation must not vary with batching" (R17; history index §2.17). A file's entry is the
  same whichever chunk the file was evaluated in: rows are attributed by their own location
  values, not by chunk.
- "Incremental indexing allows no corpus-wide statistic", and "planning statistics must survive a
  restart" (design index §10 and §5, S124). The cross-file statistics are rebuilt from the replayed
  rows, as for evaluated chunks. An export that no evaluated chunk measured is entered as empty.
- A comparable system's "all-or-nothing cache invalidation" (design index §17, LA 6, F3). Here
  invalidation is per file and per plan.

## Measured (2026-11-06)

All 100 converted rules in one run. Machine load 16–52.

| Corpus | Run | Instructions | Peak RSS | Findings |
|---|---|---|---|---|
| s200 | memo empty | 31.9e9 | 287 MB | 4 |
| s200 | warm, unchanged | 6.2e9 | 45 MB | 4 |
| s200, 3 files edited (one shifted, one added, one deleted) | memo on | 12.2e9 | 74 MB | 8 |
| the same | memo off | 28.5e9 | 289 MB | 8, the same rows |

The full corpus's rows are in the notes (`t10-corpus-profile-opportunities.md` §2c).

The replay is unchanged, and the same on a second run that reads every check's files from the memo:
3988 273 266 0 with hand-written rules, 3988 175 169 0 converted.

## Open

- Nothing removes old memo rows yet: each engine build or program writes a new plan's rows. They die
  with their sources when the store collects them.
- A run whose cross-file part reads base relations isn't memoised. Pushing reads into the chunks
  (decision 0051) leaves none in the combined run, but some single rules keep them.
