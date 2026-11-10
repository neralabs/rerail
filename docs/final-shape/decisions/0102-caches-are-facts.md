# 0102: A cache is stored facts, and its freshness is a rule

Status: proposed (2026-11-10). Number not reserved in trunk's register (0094 §9). What is built is named in each
section; the rest is the open items at the end.

## Intro

The engine keeps about fifteen caches between runs: tapes, the per-file memo, the phase store, the constants' rows,
prepared programs, the text tests' automata, the statistics keeper, the kept strata and others. Each was built on
its own, so each has its own key and its own test of whether it is still right:

```text
constants.rs   key = sha256(programs, root, options − inputFiles, wanted, build)   then World::still(consulted)
phases.rs      run_key = sha256(text, upstream digests, options, root, build)      then unchanged_phase(consulted)
memo.rs        plan_digest = sha256(build, grammar version, node-id scheme, program)
reuse.rs       run key, then inputs::still(consulted)
```

A key is a hash of what its author remembered to put in it. When an input is forgotten, the cache gives a stale
hit, and nothing reports it: the per-file memo once kept a run's findings after its text patterns changed, because
the patterns were not in `program_digest` (O-stored-patterns). Each `still` is Rust code that compares digests
one by one, outside the engine that maintains every other fact.

The architecture already says what a cache should be (03 §1): "There is no separate 'file layer' or 'cache
layer'; a cache entry is a fact with a version." 0092 adds: "the key must also be facts in our own system". This
record sets the one pattern every cache follows, so that caching and invalidation are the engine's ordinary work
on facts, and adds checks that keep new caches from going around it.

## Decision

### 1. What a cache entry reads is facts

An entry records each thing it read, with the digest it had then, as a row of one relation:

```text
// cache_read(cache, key, kind, name, digest): entry `key` of `cache` read `name` (a file, a directory's
// listing, a setting, the engine's build) when its digest was `digest`.
cache_read("constants", "3f2a…", "file", "/ws/.venv/pyvenv.cfg", "9c1e…").
cache_read("constants", "3f2a…", "dir",  "/ws/.venv",            "b07d…").
cache_read("constants", "3f2a…", "build", "rerail-engine",       "e5d0…").
```

- **Every input is a read.** The engine's build, the grammar's version and the program's text are reads like any
  file, so they cannot be left out of a key. The key itself names the entry and nothing more. It never has to hold
  an input to make the entry right.
- **Misses are reads too.** A path that was missing is read with the digest `none`, so the entry goes stale when
  the path appears (0094 §5: "The loader records what it read, misses included").

### 2. What is there now is facts

Phase 0's observer gives each name an entry read its digest now, as rows of `observed(kind, name, digest)`. It
answers by a `stat` and reads only what changed (t24). It is asked only for the names that some entry read
(demand): a run does not observe the whole tree to check one entry.

### 3. Freshness is a rule, run by the engine

One library program, `lib/cache.rr`, says when an entry is fresh. The engine runs it like any other program:

```text
.decl cache_entry(cache: symbol, key: symbol)
.decl cache_read(cache: symbol, key: symbol, kind: symbol, name: symbol, digest: symbol)
.decl observed(kind: symbol, name: symbol, digest: symbol)
// An entry is stale when something it read is not as it was.
.decl cache_stale(cache: symbol, key: symbol)
cache_stale(c, k) :- cache_read(c, k, kind, n, d), observed(kind, n, now), now != d.
cache_stale(c, k) :- cache_read(c, k, kind, n, _), !observed(kind, n, _).
.decl cache_fresh(cache: symbol, key: symbol)
cache_fresh(c, k) :- cache_entry(c, k), !cache_stale(c, k).
```

No cache has a test of its own. A lookup reads `cache_fresh`. When a run's observations change, the change
reaches `cache_fresh` by the engine's maintenance (counting, 0090), not by code a cache owns. So invalidation
costs nothing to write: it is the same mechanism that keeps a rule's outputs right.

### 4. One recorder and one reader

`corpus::cached` is the only code that writes `cache_entry` and `cache_read` and reads `cache_fresh`:

- `cached::Entry::begin(cache, key)` collects reads (`read(kind, name, digest)`), and `keep(payload)` stores the
  entry, its reads and its payload in one transaction.
- `cached::fresh(cache, keys)` observes what the entries read, runs `lib/cache.rr` over them, and returns the fresh
  entries' payloads.

The store holds the reads in one table per relation: `cache_entries` and `cache_reads`. These are facts in the
store, in the shape 03 §2 gives the lattice: a row per fact, in SQLite.

### 5. The payload is relations where it is rows

Where an entry's payload is rows (the constants' rows, a phase's exports, a file's exports in the per-file memo),
it is kept as rows of its relation under the entry, not as text or a blob. 0099's choice of what a relation keeps
(store, inline or demand) decides which derived relations are kept at all. A payload that is not rows (a tape, a
compiled automaton) stays a blob under the entry. Its freshness is still §3's.

### 6. Checks that keep it the one pattern

`corpus::cached::tests::architecture` fails when:

- **a cache is made outside the pattern.** Every table in the store's schema is listed under what it is: a
  pattern table, a relation's stored rows, an observer's table, or one of the caches not yet moved (below). A new
  table that is in none of these lists fails the test.
- **a cache tests its own freshness.** The functions that compare digests outside the pattern (`World::still`,
  `inputs::still`, `unchanged_phase`, `derivation_valid`) may be called only from the call sites on the
  not-yet-moved list. A new call site fails.
- **freshness is decided in Rust.** `cached.rs` reads `cache_fresh` from the engine's outputs. It holds no digest
  comparison of its own.

The not-yet-moved list may only shrink. Each item on it names the open item that moves it.

## Order of the work

1. The pattern, `lib/cache.rr`, the recorder and reader, and the checks. Built: see the commit that adds this
   record.
2. The constants' rows (0094 §6): the first cache moved. Built with step 1.
3. The phase store's run record and the whole-run cutoff (`reuse.rs`): the same consulted reads. Their freshness
   is built: `corpus::changes_of` asks `cached::changed`, which reads `cache_changed` (the documents that changed,
   so the run is brought up to date by them), and `World::still`, `inputs::still` and `git::still` are gone; each
   is now only an observer (`now`, `observer`). The records are still their own tables (`phase_record`,
   `run_record`), moved next.
4. The per-file memo: its reads are the file's content source and the program, whose rows the memo already holds.
   Its plan digest goes.
5. Tapes and the projected relations (the stored-projection spike, O-names-cost): an entry per (tape, relation),
   with node ids relative to the document, after RER-131's named-only tapes merge.
6. Prepared programs, the stored automata, the kept strata, the statistics keeper's blob.
7. Collection (0053) as reachability over `cache_entry`, in place of the manifests.

## Open

- **The engine's maintenance between processes.** §3 is maintained within a run. Across runs, the freshness program
  runs again over the entries looked up. That is cheap, because it reads only the entries a run asks for, but it is
  not yet the stored fixpoint brought up to date (0083 §5, O131).
- **Statistics.** 0096 keeps them in memory with one blob per plan. As facts they would be a maintained aggregate
  relation. Not decided here.
- **DRed against counting.** §3 has a negation (`!cache_stale`). Counting handles it within a stratum order, as
  0090 does. The choice of mechanism stays with 0094 §4.
