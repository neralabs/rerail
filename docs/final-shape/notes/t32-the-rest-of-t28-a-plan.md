# t32. The rest of t28: a plan, with every cache as facts

## Intro

t28 made Python typing three phases: P1 `struct.rr`, P2 `types.rr`, P3 `calls_p3.rr`. P3 now answers every call
`calls_cx.rr` answers (6,410 agree), and a session maintains P2 from struct's delta (a function added: 9.4 s to
2.5 s). This note plans what is left, under two constraints:

- **Every cache is facts** (rr-align's decision 0102). An entry records what it read as `cache_read` rows, and its
  payload as rows of its relations. `lib/cache.rr` decides freshness, and the engine runs it. That holds for
  state a session keeps in memory too: it is a resident copy of facts the store also holds, not a cache of its own.
  So no cache writes SQLite in a way of its own.
- **Stay compatible with the sibling worktrees.** Several of them are building the parts this work stands on. This
  plan builds on their parts and does not make a second copy of any of them.

The plan has three parts:
- what the siblings are doing (surveyed read-only, 2026-11-10);
- the principles that follow from it;
- the work, in order, each step with where it is built, what it waits for, and when it is done.

The four decisions that are the user's are at the end.

## What the siblings are doing

| Worktree, branch | Work that touches this plan | State |
|---|---|---|
| final-shape-rr-align | **0102 "a cache is stored facts, its freshness a rule"** (b4a68891d): `corpus::cached`, `lib/cache.rr`, the architecture tests, the constants moved. Uncommitted: the whole-run cutoff moved (`cache_changed`, `World::still` gone). c66b24bb4: constants of phase exports derived once, before the chunks | not in the hub; 67 commits ahead of our merge base, matrix-offload's merge included |
| More-ergonomic-rule-syntax | **a second 0102, "write-back"** (1dedbfa32, 3731ebe12): one writer thread per store; memo entries, prepared program, choices, kept statistics and kept strata queued as closures that encode their own rows | built there; a merge of the hub in progress |
| Matrix-offload-spikes | **kept strata across processes** (470e92fde, a12ae9ebc): a cross program's input and relation digests and rows kept in the tape cache, with early cutoff; plan choices kept (c99cf4686) | in More-ergonomic and rr-align, not yet in the hub |
| Two-pass-roots-and-cross-file | 0090 §4 "derived again" (uncommitted): a maintained stratum whose deltas outgrow it is derived from empty; edits `maintain.rs` `rewrite()` and `plan::run`'s stratum loop | uncommitted, on an older base |
| wolfeel (trunk) | 0100 §11 chaining in `plan::run`'s level loop (ede95ba5d); the decisions register | 4 commits not in the hub |
| Final-shape-quality (hub) | 0101 draft (skip strata whose deltas are all empty); a merge of Parser-generators' tier 1 crate in progress | 0101 not built |
| Typing-support | per-definition node ids the default (uncommitted `syntax/granules.rs`) | uncommitted |
| Current-state-benchmarks | t31, the one-file-edit workplan: B4 keeps relations between processes "as column blocks", D4 is 0090's delta steps | a note; nothing built of B4 |
| rer-131 | edab96dda: a stratum whose rules all join an empty relation keeps its facts (near 0101 §1) | in tbelanger's integration |

Nobody else edits `typing/programs/*.rr`, `phases.rs` or `session.rs`. Nobody plans searches (0083 §4), `__all__`
or async call typing. Two numbering clashes need settling: 0102 is used for two decisions, and rer-131 has its
own t28. Our 6afe7e79f (a program reading no document has no file-local part) and rr-align's c66b24bb4 do not
conflict: ours takes P2's case, theirs a program that reads documents and phase exports.

## Principles

1. **One representation.** Every kept thing is a 0102 entry: `cache_entry`, `cache_read`, and payload rows.
   - A phase's exports, versions and deltas are entries of the phase.
   - A stratum's kept relations are entries of the cross program.
   - A search's answers are entries of the search.
   - No new table is made outside `cached`'s three. 0102's architecture test enforces it.
2. **One transport.** Write-back's single writer per store is how entries reach SQLite: memory ahead, SQLite
   behind, as decided. Its jobs carry entries, written by `cached::Entry::keep`, and no job encodes rows of its
   own. So the two 0102s compose: rr-align's says what is kept, More-ergonomic's says when it is written.
3. **Memory is the resident copy.** A session's fields hold the entries it uses, in engine form (relations,
   indexes), and are named by the entry they copy. A new process loads the same entry from SQLite, and nothing is
   kept only in memory. 0102's checks gain a third list, the session's fields and the entries behind them. The
   list of fields not yet moved may only shrink.
4. **Freshness and change are rules.** Whether an entry is fresh, what changed in what it read, and the delta
   between two versions of a relation are rules over facts, run by the engine (`lib/cache.rr`). Rust compares no
   digests.
5. **Build where the owner builds.** 0101, 0090 §4 and the levels loop belong to the branches that own
   `plan::run`. Kept strata belong to matrix-offload's work, and the cache pattern to rr-align's. This branch
   uses them and moves its own parts onto them.

## The work

Each step is measured before the next, on the 84-file corpus: P3's answers against `calls_cx.rr` and Jedi; P2's
exports against a full run; CPU and instructions retired, cold, warm and after the O125 edits.

### Stage 0. Settle and merge (no code)

- **0.1** Settle the numbers on trunk's register (0094 §9): the two 0102s, and the notes clash.
- **0.2** Merge the hub again once it has taken rr-align (0102 and kept strata) and More-ergonomic (write-back).
  Until then this branch adds no cache and no table.
- **0.3** Tell rr-align, whose 0102 step 3 moves the run record, about the fix the merge needed here. Since
  58a5243a1, a run that reads no document consults nothing. The run record deleted such a record, so P2 was
  never kept and ran again on every unchanged run. An empty record is now kept, and the key (which holds the
  upstream exports' versions) decides alone. As facts, that is an entry with no `cache_read` beyond its key's.
- **0.4** Tell Two-pass-roots that `phases.rs` reports a phase run the session maintained as maintained. A
  stratum derived again (0090 §4) inside it is still correct, and is counted as maintained work.

### Stage 1. P3 and P2 rules (this branch, now, independent)

Typing programs are this engine's (0047). Nobody else edits them.

| Step | Change | Done when |
|---|---|---|
| 1.1 | **P1 exports annotations parsed**, as facts (`struct__ann(e, i, head, arg)`), in place of P2 splitting their text with regexes. That fixes, at once: unions inside brackets (`AbstractContextManager[_T_co, bool \| None]`), nested generics (`_GeneratorContextManagerBase[Generator[_T_co, ...]]`, so `tparam` takes every base) and `Callable[P, A]` | `gin`, `grest`, `garg`, `alt` read `struct__ann`; P2's exports equal or a superset; the 5.4 s `nclass` class of cost gone |
| 1.2 | `__all__`: P1 exports a module's `__all__` list as `struct__all(m, x)`; P2's `import *` reads it when there is one | a test over a miniature stub; P3's agree count measured |
| 1.3 | Overloads by keyword: `lfit` by the keyword names a call passes, against `struct__param`'s names | `os.environ.get(k, default=d)` picks its overload in a test |
| 1.4 | Overloads by argument type, where the argument's class is known (`lv`/`pv`) and the parameter declares a class: an argument that is an instance of neither the class nor a subclass rules the overload out. It is a negation over P3's values, so it is stratified after them, by the overload choice's own stratum | a test; no answer worse on the corpus |
| 1.5 | Async: P1 flags a library `async def` (`struct__flag(f, "async")`), so `await lib.f()` is f's declared return and a plain call is a coroutine; `async for` and `async with` are `__aiter__`/`__anext__`/`__aenter__` implicit calls | tests for each; awaited library values counted on the corpus |
| 1.6 | Nested runs' profile: a rise on a phase's worker is charged to the phase's span | a profiled P3 run shows P2's workers under P2 |

### Stage 2. A phase's state as facts (this branch, after 0.2)

0102 orders the phase store as its step 3 (`phase_record`, `run_record`, `unchanged_phase`), in rr-align's hands.
The rest of the phase store is this branch's, since `phases.rs` keep and maintain are: `phase_granules`,
`phase_cones` and `export_versions`. These two steps are agreed with rr-align first, so the phase store moves
once.

| Step | Change | Done when |
|---|---|---|
| 2.1 | **A phase's exports are an entry.** Cache `phase:<name>`. Its reads: the program's text, the engine's build, the root and options, and each upstream export's version (P2 reads struct's). Its payload: each export's rows, by granule. A version is an entry's id, so "the version before" is the entry it replaced | `phase_granules` and `export_versions` gone; 0102's test lists them as moved |
| 2.2 | **A delta is a rule.** `lib/phase.rr` derives an export's `plus`/`minus` between two entries' rows, as `cache_changed` derives what changed. Granule freshness (`unit`, the derivation records) is `cache_read` of each definition's digest | `keep` and `keep_in_session` compare no rows in Rust; P2 over the O125 edits gives the same exports and deltas |
| 2.3 | **The session holds entries, not rows.** `Session::phase_rows` (added in 6afe7e79f) and `Session::phased` become resident copies of the entries of 2.1, named by entry id; write-back writes the entry behind the run | 0102's session list has them as moved; a new process given the session's store finds the same entries |
| 2.4 | The cones (`phase_cones`, `record_documents`) as `cache_read` rows of the entry | the table gone |

### Stage 3. P2 maintained across processes (after kept strata are in, and are entries)

t28's step 7 left one gap: a new process runs P2 in full after any struct change (3.8 s). Matrix-offload's kept
strata already keep a cross program's derived relations between processes, by digests. 0102's step 6 makes them
entries. P2 then needs no mechanism of its own:

| Step | Change | Done when |
|---|---|---|
| 3.1 | Kept strata as entries (0102 step 6): cache `strata:<cross program>`, read = each input's version, payload = each stored relation's rows (0099's "store" relations only) and, under counting, their supports' counts | matrix-offload's `kept_strata*` tables gone (with rr-align and matrix-offload) |
| 3.2 | **A stale entry whose reads changed only in versions with known deltas is brought up to date, not dropped.** `cache_changed` names the reads that changed. When each is an upstream export whose delta between the two versions is known (2.2), the run loads the entry as its retained state and maintains it from those deltas, as a session does. Otherwise it derives in full | a new process after E1: P2 under 3 s and its exports equal to a full run's; after E2 and E3 (no export changes) under 0.5 s |
| 3.3 | Measure what loading costs. P2's stored relations are read back whole, so loading has to cost less than the 3.8 s full run. If it does not, keep fewer relations (0099), or load lazily by the strata the delta reaches | a table of load against derive, per edit |

This is 0083 §5's stored fixpoint brought up to date, which 0102 leaves open (O131). It is built as an ordinary
use of entries and deltas, so it is the same for P2, for calls_cx's cross part and for the pack (t31 B4). t31's
"column blocks" would then be entries' payload rows in a columnar encoding, under write-back, not a format of
their own.

### Stage 4. Strata the delta does not reach (the owners' work, measured here)

E1 maintains 92 of P2's 99 strata for a 3-row delta, 2.4 s against a full run's 2.9 s. 0101 §1 (skip a stratum
whose deltas are all empty) is the fix. It goes in `plan::run`'s stratum loop, where wolfeel's chaining
(0100 §11) and Two-pass's "derived again" (0090 §4) are also landing, so it is built by the hub's owners. This
branch re-runs `spike_t28_a_session_maintains_the_typing_phase` after each lands. Done when E1's P2 costs under
0.5 s.

### Stage 5. Searches (this branch, a decision first)

Generic instantiation (`inst`) and member lookup through the MRO (`mro_find`) as 0083 §4's searches. P3's eager
`gpar`, `gbind`, `wbind`, `lgpar`, `lcbind` and `found`/`lfound` are what they replace (`wbind` alone was 2.7 M
join work units for 23 rows, t27).

| Step | Change | Done when |
|---|---|---|
| 5.1 | A decision draft, its number reserved on trunk. A search is a program kind. A request is a fact (`inst__ask(f, targs)`). The engine answers a wave's new requests in one batch, and the answers enter as the next wave's delta (0065 §2a's loop) | reviewed |
| 5.2 | **Answers are entries.** Cache `search:inst`, key = the request. Reads = the granules the answer probed (P2's export rows, by granule digest) and other searches' keys. Misses are kept too. So invalidation is `lib/cache.rr`'s, and in a session the maintenance's | no memo of the search's own |
| 5.3 | `mro_find` first: it replaces `found`/`lfound` and `need_name`'s patches, and its keys (class, name) are few per call | P3's agree count unchanged; the `found` strata's work measured before and after |
| 5.4 | `inst` for the library generics (`lgpar`, `lcbind`, `lwbind`): `inst(f, targs)` over P2's `tparam`/`tmap`/`rgen` | `wbind`/`gpar` work gone |

### Stage 6. P3 on symbols (this branch, after 1.1 and 5.3)

t28's "P3 without libraries", as far as it is left:
- workspace names resolve through `types__binds` and `types__member`, not `qv`/`rr_cx_reach`, which leaves P3
  with no cross-file reach of its own;
- targets become symbols, placed by `struct__at` at the end.

That also gives the cross-file memo its finer key. A P3 component reads export rows, so its entry's reads are
those granules, and an edit misses only the components whose granules changed (t28's memo item). This
waits for per-definition node ids (Typing-support's `granules.rs`), so positions stay out of what is compared.

## Order, and what each stage waits for

| Stage | Waits for | Where |
|---|---|---|
| 0 | the user's decisions below | register, merges |
| 1 | nothing | this branch, now |
| 2 | the hub having 0102 and write-back (0.2); agreement with rr-align on the phase store | this branch |
| 3 | kept strata merged and moved to entries (0102 step 6), 2.2's deltas | this branch, with rr-align |
| 4 | 0101 §1 in the hub | the hub's owners; measured here |
| 5 | 2 (answers as entries) | this branch |
| 6 | 1.1, 5.3, per-definition node ids | this branch |

Stage 1 can start at once. Stages 2 and 3 are the cross-process work, built only on 0102.

## What this branch must not do

- Add a table, or a digest comparison outside `lib/cache.rr`. 0102's tests will fail on either.
- Build its own kept strata, its own writer, or 0101's skip.
- Edit `maintain.rs` `rewrite()` or `plan::run`'s stratum loop while Two-pass and wolfeel have changes there that
  are not merged.
- Keep anything in a session that is not a copy of an entry. `Session::phase_rows` is such a field today, and
  2.3 moves it.

## Decisions for the user

1. **The two 0102s.** Proposed: caches-are-facts keeps 0102, and write-back is renumbered 0103 and amended so
   that its jobs carry 0102 entries only (principle 2).
2. **Who moves the phase store.** Proposed: rr-align moves the run record (its step 3), and this branch moves the
   exports, versions, deltas and cones (stage 2), agreed first.
3. **P1 exports annotations parsed (1.1).** It changes struct.rr's exports, read by P2 and by readers of struct.
   Proposed: yes, as an added export, with the text export kept until P2 no longer reads it.
4. **Session fields under 0102's checks (principle 3).** Proposed: 0102 gains the session list, so in-memory state
   is held to the same pattern as stored state.
