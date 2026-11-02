# 0025: the tape format stays v8; v9np is not locked until a consumer needs its access pattern

Status: proposed (2026-11-01)

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

The tape format stays v8 (`TMPTAPE8`), and v9np is not locked now.
- v9np meets the size requirement: no increase. It is regular apart from the one non-uniform
  `extra` kind.
- Its gains are random node and span access (1.6–4.3×) and column upload. Neither is on a hot
  path of the engine as built:
  - waves read tapes sequentially through `TapeDoc`'s `transducer_*` accessors;
  - the GPU's tape build is 0.7% of a cold audit;
  - no SQL reads spans from tapes (the §17 pure-SQL fallback is not built).
- A format switch touches every stored tape and `tape.rs`, for no measured benefit today.

**Revisit when** a consumer needs v9np's access pattern:
- the §17 SQL fallback, whose span extraction the spike measured 2.7–8× faster;
- or Phase 9's resident slabs, where the at-rest and resident formats are designed together
  (O8's requirement).

Then rerun these five measurements on that consumer's path, and lock v9np with the minority
`extra` code, supertypes as a sidecar keyed by the tape digest, and no parent column unless SQL
parent climbs prove hot.

## Agrees with / departs from

- **Agrees with** P 0003's size and regularity findings, now measured on Rerail's grammars:
  0.96–1.04× v7, against the spike's 0.93–1.01×.
- **Departs from** 0003's direction to adopt v9np now. The spike's access-time gains are real, but
  nothing in this engine reads tapes that way yet. This follows the ledger's measure-on-the-path
  rule: spike 2.18, measure at deployment size, not by proxy.
