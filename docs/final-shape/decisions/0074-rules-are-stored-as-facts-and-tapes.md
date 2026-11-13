# 0074: Rules are stored as facts, and as a tape of those facts

Status: built (2026-11-07); §1–3 and §4's facts entry superseded by 0155 (a stored program is its `.rr` text)

## Intro

Decision 0073 kept a prepared program in the store as JSON: an encoding of its Rust value, which nothing but the
engine could read. This record stores it the way the engine stores everything else. The program is written as
facts of a fixed schema, so a rule can read it. Those facts are a `.rr` document, kept as a source with its tape
like any rule file. The rows the run reads back are the ones read off that tape, memoised per document.

The schema is the part that matters beyond the cache. It is how a rule, or a program a run compiled, becomes data:
the same data whether it is checked by a security rule, compared between two builds, or read back to run.

```
Prepared ──► facts (rule, body, atom, arg, term, decl, …, prepared_*)
                │
                ▼
           .rr document (decls + facts) ──► source (sha256) ──► tape (rr grammar, decision 0061)
                                                  │                   │
                                                  │                   ▼
                                                  │            facts read off the tape
                                                  │                   │
                                                  ▼                   ▼
   pointer (program text, plan) ──► document digest      facts memo (document, rows)
```

## Decision

### 1. A program as facts

`logic::program_facts` writes a `front::Program` as rows and reads them back as the same program. Every list is
indexed in its rows, so the rows are a set and any order reads back the same. Ids are numbered per program, and a
column `p` names the program, so one document holds several. Strings are symbols and every other value a number.

| What | Relations |
|---|---|
| Rules | `rule(p, r, head)`, `body(p, r, k, lit)` |
| Literals | `lit_atom(p, l, a)`, `lit_cmp(p, l, left, op, right)`, `lit_agg(p, l, var, op, a)` with `agg_over`, `agg_join`, `agg_by`; `lit_nearest(p, l, n, a, set)`; `lit_ast(p, l, doc, node, matcher)` with `ast_capture` |
| Atoms, terms | `atom(p, a, rel, neg, gated)`, `arg(p, a, i, t)`, `term(p, t, kind, n, s)`, `call_arg(p, t, i, u)` |
| Declarations | `relation(p, rel, arity)`, `decl(p, rel, i, col, ty)`, `nullary`, `lattice` with `lattice_merge`, `lattice_widen`; `input(p, k, rel)`, `output(p, k, rel)`; `kindset`, `kindset_kind` |
| Facts | `fact_row(p, rel, row, width)`, `fact(p, rel, row, i, kind, n, s)` |
| What preparing made | `demand`, `demand_dropped`, `magic_extras`, `magic_report` with `report_list`; `gate` with `gate_src`, `gate_quiet`, `gate_dep`; `native_dfs`, `native_match` and their parts; `rx_span`, `policy`, `setting` |

A term is one row with a kind (`var`, `wild`, `int`, `str`, `top`, `call`), a number and a string. A call's
arguments are rows of `call_arg`. So `x != "g()"` is a `lit_cmp` row and two `term` rows, and `kind(c, "call")`
is an `atom` row, two `arg` rows and two `term` rows.

A rule about rules reads these relations directly. This one, run as any rule is, lists the relations a program
negates (from the test `a_rule_reads_a_program_as_facts`):

```
// The program as facts, then the question.
#include "program.facts.rr"
.decl negated(rel: symbol)
negated(rel) :- body("rule", _, _, l), lit_atom("rule", l, a), atom("rule", a, rel, 1, _).
.output negated
```

### 2. A prepared program as facts

`corpus::prepared` writes both programs a run prepares: `front` (pruned, simplified, demanded) and `lowered` (as
lowered, its joins pushed, located and merged). It adds what preparing reported as `prepared_*` relations: column
domains, the operator atoms, the nearest-ancestor sets, the relations demanded, pushed and located, the merge
counts, every intern call with its atoms, and the text tests with their patterns.

The file-local split is not stored. It is a function of the lowered program (`file_local::programs`) and is made
again on reading. It is 6.7 ms of the 11 ms that reading a program back takes (`ruff-up006`). Storing it as two
more programs would double the entry to save part of that.

### 3. The document and its tape

The rows are printed as a `.rr` document: the schema's declarations and one fact per row (`program_facts::document`,
through `rr::print`). The document is stored as a source. It is read through the store as a rule file is
(`rr::parse_stored`, which takes the tape from `core::build_cached`), so its tape is kept by content and grammar
version. The facts the run keeps are the ones read off that tape. A document whose tape does not give back the
facts written is not stored, and the profile says so.

### 4. Two memo entries

- **Pointer** (schema 101): under the program text's source, for the plan of decision 0073 (engine build, grammar
  version, dialect, additions), the document's sha256.
- **Facts** (schema 102): under the document's source, for the `rr` grammar's version, its rows in a compact
  encoding (`program_facts::encode`: a string table, then each value a tag and a number).

A run reads the pointer, then the facts. When the facts entry is gone it reads them off the document again. When
the pointer is gone it prepares. The interned atoms are checked as before (decision 0073 §2).

### 5. The rr parser no longer counts lines from the start

Reading a document back found that the `.rr` parser was quadratic. It asked each statement's line by counting
newlines from the start of the file (`line_of`), and the tape reader did the same for each node. A line index
(`syntax::Lines`, a binary search over newline offsets) replaces both. For a document of 43,412 lines (637,270
nodes), taping went from 7.1 s to 54 ms, and parsing from 13.3 s to 91 ms. Every large rule file gains the same.

## Measured

`ruff-up006`, prepared: 43,355 rows; a document of 1.6 MB; the encoded facts 2.1 MB before the store's zstd. Wall
time at load 93: decoding 1.8 ms; reading both programs 4.4 ms; the split 6.7 ms; reading back in all 11 ms.
Preparing the same rule takes about 38 ms.

On 18 Python pack policies (every 25th, sorted, without b005), each `run-rr` was a fresh process over
`fixtures/python/calls/project`, all three passes on one fresh store. CPU is user plus system, at load 36 to 41:

| pass | stats phase (holds preparing) | process CPU |
|---|---|---|
| 1: prepared and stored | 2.12 s | 4.23 s |
| 2: read from the store | 0.23 s | 2.07 s |
| 3: read from the store | 0.24 s | 2.16 s |

A program read from the store costs about 13 ms of the stats phase, against about 52 ms prepared. Storing one costs
about 65 ms more than preparing it, mostly reading its document back off the tape to check it. That is paid once
per program and engine build.

The converted-rules replay, run twice on one fresh store, gave the recorded findings (3988 303 298 0) both times:
10.46 s user + 1.97 s sys preparing and storing its 17 programs, 7.99 s + 1.76 s reading them (load 75).

## What this does not do yet

- **Lowering is not a rule program.** The facts are written by Rust from the lowered program. A lowering written
  as rules over a rule file's facts would derive them (decision 0062).
- **Source rules are not written as facts.** A rule file's tape gives syntax facts (`node`, `field`, `text`), and a
  prepared program gives these semantic ones. Writing a rule file's own program as facts when it is read would let
  a rule about rules ask the same questions of a rule as written.

## Prior work consulted

- **Lessons ledger 1:** the compact tape is the source of truth, and the rest derives from it. The document's tape
  is stored, and the facts the run reads are derived from it and memoised.
- **Lessons ledger 25:** restoring a stored derivation beats redoing it. A program read back takes 13 ms where
  preparing it takes 52.
- **ast-index-spike history index L919-920:** key memoised analysis by what it is a function of. The facts are keyed
  by the document's content and the grammar's version; the pointer by the program's text and the run's plan.
- **ast-index-spike history index L417 (DH L2185-2189):** "storing data in memory... is dishonest". The program is
  in the store, as data a rule can read.
- **ast-index-spike design index S112 (the compiled artifact):** a compiled query is an artifact of its own. Here it
  is a document, with a tape and facts like any other.
- **Decisions 0052, 0061, 0062 and 0073:** the facts memo, the rr tape, reading statements off it, and the stored
  prepared program this record changes the form of.
