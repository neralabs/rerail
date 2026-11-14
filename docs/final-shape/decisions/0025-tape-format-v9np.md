# 0025: the tape format becomes v9np

Status: accepted (2026-11-01, user decision)

## Context

O8 asks for five pre-lock measurements of v9np on Rerail's own grammars, then a decision. v9np is
P decision 0003's proposal (Pulling-it-all-together `docs/rule-engine-direction/decisions/`): a
structure-only, regular, column-stored tape. W runs `TMPTAPE8`, which is v7 plus N's supertype
table. The spike's harness is placed as `tape::v9np`. `tape::v9np::tests::measure_on_a_corpus` runs
its `measure` mode on a corpus through Rerail's vendored and patched grammars.

The user's requirements (O8): the format must be completely regular, and stay processable in SQL
with ast-index-spike techniques. A change must show clear benefits; a size increase was not
acceptable.

**Corpora.** The harness holds every node in memory: 31.5 MB of source peaked at 783 MB RSS. So
each corpus is a sample, not the whole repository:
- the data-layer repository (`~/orca/projects/nera-data-layer-gruel-main`, without `.venv`): every
  fifth Python file (1,903 files, 31.5 MB), and all of its JavaScript, TypeScript and Rust;
- CPython 3.15's standard library: every third file (673 files, 14 MB).

The two counts are grammar properties, and the width distribution is a proportion, so a sample of
this size measures them.

## Measurements

| Corpus | Grammar | Nodes | v7 B/node | v9np B/node (×v7) | zstd v7 | zstd v9np (×v7) |
|---|---|---|---|---|---|---|
| data-layer, 1/5 | python | 6,382,485 | 8.476 | 8.485 (1.00) | 1.869 | 1.979 (1.06) |
| CPython, 1/3 | python | 3,213,618 | 8.484 | 8.399 (0.99) | 1.727 | 1.766 (1.02) |
| data-layer | javascript | 177,176 | 8.450 | 8.253 (0.98) | 2.023 | 1.910 (0.94) |
| data-layer | typescript | 17,689 | 8.578 | 8.261 (0.96) | 2.518 | 2.358 (0.94) |
| data-layer | rust | 425,120 | 8.480 | 8.829 (1.04) | 1.815 | 1.828 (1.01) |

The rejected v8 SoA proposal is 11.0–14.1 B/node on the same corpora.

1. **Alias and folded code-space counts** (≤ 1023): 168–227 folded codes per grammar. No
   grammar has an alias node on these corpora. Kinds: 270–392 of the 1024-code space.
2. **`extra` uniform per kind:** uniform in every grammar but one place. Python on CPython has
   one kind whose `extra` varies: 1 node of 3,213,618. Folding `extra` into the code space needs
   a code for that kind's minority case.
3. **At-rest size:** v9np is 0.96–1.04× v7 logical and 0.94–1.06× v7 after zstd.
4. **GPU upload against `MetalNode` expansion, end to end:**
   - S7's Metal audit spends 12.1 ms building the GPU's tape buffers (`gpu.tapes`, 250 files), of
     a 1.83–1.88 s cold audit (06 S8).
   - The harness's v7 restore is 2.9–3.3 ns/node.
   - So columns that upload as they are save at most ~12 ms cold, 0.7%. A one-file edit saves
     microseconds.
5. **Width distribution:**
   - 4-byte node widths: 0–0.5% of documents (9 of 1,903; 3 of 673).
   - 4-byte source widths: 0–8% (67 of 1,903; 55 of 673; 5 of 101 Rust).
   - In the data-layer sample, those documents hold a third of the nodes.

**Access** (ns, random over the corpus, v7 → v9): node 16.8–43.8 → 7.1–27.7; span 15.1–31.7 →
3.1–12.5. Sequential scans are no faster (2.0 → 3.7 ns on the large Python sample).

## Decision

The tape format becomes v9np (user decision, 2026-11-01): random access is needed for proper anchor
indexing. The measurements above meet the user's requirements: no size increase (0.96–1.04× v7,
0.94–1.06× after zstd) and a regular format. Random node access is 1.6–2.6× faster, random span
access 2.5–4.3× faster.

How it is locked:
- **Flags folded into the code space** (named, extra, missing, error), with a code for the one
  Python kind whose `extra` varies.
- **Supertype memberships stay in the tape, folded into the codes.** Amended 2026-11-02 (user
  decisions): no sidecar keyed by the tape digest. Memberships are 0.23–0.54 per node: 1.35–3.2
  B/node in v8's table, and a SQL table with a row per membership and its index would be ~25–40 B
  a row, double or triple the tape. A membership is not a function of the node's kind: the
  grammar's supertype map by kind holds every stored membership but equals the stored set for only
  74–81% of nodes (an `identifier` listed under both `expression` and `pattern` is parsed as one).
  By (parent kind, field, kind) a lookup learned from the corpus misses 8–152 nodes per corpus, but
  no grammar file gives that table. So each node's code carries its own memberships, as the
  parser's cursor reports them; nodes in the same context share a code. The lookup is the code:
  exact, with no exceptions list and no shipped table.
- **No parent column**: parents are rebuilt with a stack where a consumer needs them. Revisit only
  if SQL parent climbs prove hot.
- **Reads stay behind `TapeDoc`'s `transducer_*` accessors**, so the switch is local to `tape.rs`.
  `TAPE_FORMAT_VERSION` is bumped, so stored v8 tapes re-derive.

The measurement harness keeps every node in memory (783 MB for 31.5 MB of source). The production
encoder must stream within the 500 MB budget.

## Implementation (2026-11-01)

`tape.rs`'s `TapeImage` is v9np (`TAPE_FORMAT_VERSION` 9, magic `TMPTAP9N`):
- columns `code` (u16), `size`, `base` (u32 per 16 nodes, the block's least start), `off`, `tail`,
  each 2 or 4 bytes wide per document (widths in the header);
- `code` indexes a per-tape table of folded codes: v8's kind and field, the flags (extra, error,
  missing) and the contextual alias. Per tape, not per grammar: no grammar-wide code catalog
  exists to number them deterministically, and a table of a few hundred codes costs 5 bytes each;
- `end = start(last descendant) + tail`; where some node ends before its last descendant starts,
  the tape measures every tail from the node's own start instead (a header bit), so encoding is
  total;
- no parent column: `restore_tree` rebuilds parents from the sizes with a stack;
- `TapeDoc` and its accessors are unchanged, so nothing outside `tape.rs` changed.

Format 10 (2026-11-02): each code also holds its node's memberships (a count and up to 8
abstract symbols, ascending); the image has no membership table. Nothing reads the stored columns randomly yet (the
anchor index's SQL reads, `tape/v9np/sql.rs`, are the spike's harness).

Checks:
- the tape tests (41, adapted from v8's tables to the columns), with corruption cases for the
  folded memberships;
- `stored_memberships_are_the_cursors_on_a_corpus`: after a round trip through the stored bytes,
  every node's memberships equal the parser cursor's, and its kind, field, span, parent and flags
  the native tree's, on CPython's whole standard library (9.4M nodes) and the whole data-layer
  repository (Python 32.1M nodes, and its Rust, TypeScript and JavaScript), plus once with its
  `.venv` (76.5M nodes); a self-check shows a dropped membership is detected;
- S7 710 keys = W on CPU and Metal; the prefilter, slice, facts, workspace storage, packed
  transducer and supertype-matching tests.

Stored size (`stored_tape_bytes_against_v8`, against v8 as stored, with its 6 B per membership):
0.72–0.92× v8 on the sampled corpora (CPython 0.77×, data-layer Python 0.80×, Rust 0.92×,
TypeScript 0.78×, JavaScript 0.72×); the code table is 0.17–0.53 B/node.



- **Agrees with** P 0003's size and regularity findings, now measured on Rerail's grammars:
  0.96–1.04× v7, against the spike's 0.93–1.01×.
- **Agrees with** 0003's direction to adopt v9np. Anchor indexing needs its random access.
- The measurements follow the ledger's spike 2.18: measured at deployment size, on Rerail's
  grammars, not by proxy.

## Amendment (2026-11-02): no parent column

Decided after list step 12's lab, with ast-index-spike's design and history indexes (rec6 stores
`par`, a parent delta, because its SQL climbs read the blob in place: D S047; HI L240, L271).
Rerail's tapes are zstd frames decoded whole before any read, so parents are rebuilt by one stack
pass at decode: `TapeDoc::restore` and the SQL fallback's `prefilter::tapes`. On the lab's
captures, that halves the fallback's execute CPU against descent from the root (1.55–1.63 s vs
3.53–3.63 s, same VM steps). A stored delta would add bytes to every tape for nothing a decoded
reader lacks. Details: decision 0026, "Parent lookups".
