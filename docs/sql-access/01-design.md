# Facts and rules in SQL

Status: design, 2026-11-07. The user's decisions are marked *(user)*. The rest is proposed.

## Intro

The engine's facts and rules should be usable from SQL, so that extension work can be done as SQL
queries rather than engine changes. There are three directions:

- **Read:** see every fact and derived relation, and the rules themselves, as typed tables.
- **Feed:** let a SQL query supply a relation that rules read.
- **Explain:** ask why a fact holds.

The consumers are in-process sessions only: NAPI, the CLI executor and Rust callers *(user)*. There
is no loadable extension for the `sqlite3` shell, so every virtual table and function is registered
on connections the engine opens.

Rules keep carrying no SQL. SQL enters through relation declarations and through queries outside
the rules.

## What exists

- **`CodeRuleSqlExecutor`** (`native/src/code_rule_dsl/sql_executor.rs`, `docs/sql-executor.md`).
  - It binds core facts as virtual tables and runs `.rr` through the wave scheduler.
  - It answers read-only queries over the derived relations, which last as long as the session.
  - The product audit path does not use it.
- **Base facts are virtual tables** over Rust columns (`facts/vtab.rs`), one per relation, rebound
  per wave without DDL.
- **SQL bookkeeping tables:** `derivations`, `derivation_inputs`, `name_bindings`, `fact_stats`,
  `compiled_plans`.
- **The per-file memo** keeps a file's rows of every exported relation as a compressed frame,
  `facts_results(source_id, fact_schema, path, plan_digest, frame)`. It is not readable by row.
- **A wave step is already a SQL statement:** `temp.wave_step(step, stratum, output, sql)`.

The gaps:
- rule text is stored nowhere;
- derived relations vanish with the connection;
- columns are positional (`c0, c1, …`);
- raw source cannot be read from SQL.

## 1. The catalog: rules and relations as data

There is one eponymous virtual table per row type, built from the prepared program and persisted
by program digest beside `compiled_plans`:

```text
rr_relation(name, arity, kind, file_local, stratum, lattice, producer)
  -- kind: base | derived | lattice | native | demand | sql
rr_column(relation, ord, name, type)        -- type: node | doc | atom | int | text | …
rr_rule(id, pack, file, span, head, digest, text)
rr_atom(rule, ord, relation, negated, aggregate)
rr_plan(step, rule, stratum, mode, sql, decisions)
```

`rr_plan` turns today's `wave_step.sql` and explain output into rows.

## 2. Reading relations: `rel_<name>` *(user: option C)*

Every relation is readable as `rel_<name>`. Its column names and types come from its `.decl`.

- **Storage is the frames the engine already writes**, read per (relation, document) and merged
  with the in-memory overlay. Nothing new is written for SQL's sake.
  - File-local relations come from the per-file memo (decision 0052).
  - Cross-file relations (rounds, the library stage) are stored the same way: frames partitioned
    by the document of their first location column.
- **Pushdown.**
  - A constraint on a document, or on a node (whose document is `n >> 32`), reads one frame.
  - Other constraints scan the relation's frames.
- **Validity.** A frame is valid while its derivation is valid. That is one join over the name
  bindings, as in 03 §14. A stale frame is not returned silently: the query errors, or re-runs the
  derivation, as the session chooses.
- **Materialisation.** For heavy ad hoc analysis, `SELECT rr_materialize('calls')` copies a
  relation into a real, indexed table, scoped to the session. This is the only path to real
  tables.
- **Decoding identity types:**
  - `node_path(n)`, `node_span(n)` and `node_text(n)`;
  - `source_text(doc, start, end)`, which decompresses from the content store and caches;
  - `kind_name` and `field_name` on every engine connection;
  - `sym_text` and `sym_atom`, which exist already.

## 3. SQL producers: relations whose rows come from a query *(user: kinds 1 and 2)*

A producer is declared like a Soufflé `.input` with an IO parameter. Rules see only the relation.

```text
// Kind 1: reads ordinary tables (store tables, attached databases).
.decl deprecated(name: symbol, replacement: symbol)
.input deprecated(IO=sql, query="SELECT qualified_name, replacement FROM deprecated_api")

// Kind 2: reads a relation the rules derive.
.decl module_size(module: symbol, functions: number)
.input module_size(IO=sql, query="SELECT module, count(*) FROM rel_def GROUP BY module")
big_module($m) :- module_size($m, $c), $c > 500.
```

- **It is a wave step.** A producer is a `wave_step` row whose statement is
  `INSERT INTO new_<rel> <query>`. The scheduler runs it like any lowered rule, on the owner
  connection.
- **Stratification.**
  - The `rel_*` relations a query reads are its dependencies. The SQL text is parsed for them, and
    the authorizer callback confirms them at prepare time.
  - The producer sits in a stratum above everything it reads.
  - A cycle through a producer is refused with a named error. SQL is not monotone, so iterating it
    may never settle, the same reason negation inside recursion is refused.
- **Invalidation.**
  - The producer's derivation names the digest of its SQL text and the engine build.
  - Its inputs are what it actually read: the tables the authorizer saw, plus the derivations of
    the `rel_*` relations it read.
  - An unchanged output digest stops propagation, as everywhere else.
- **Typing.** Result columns are checked against the `.decl`. Text in `symbol` columns is interned
  on the way in (`sym_atom`).
- **Restrictions.** A producer is read-only and deterministic. `random()`, `now()` and the like are
  refused.
- **Demand.** A producer's query is opaque, so magic sets cannot restrict it, and it is computed
  whole. Later, a bound parameter (`:module`) could take its demand.

## 4. Provenance *(user: design for it now, build later)*

This follows Soufflé's provenance evaluation (Zhao, Subotić and Scholz, TOPLAS 2020).

- **Every derived row has two hidden columns on `rel_*`:**
  - `_rule`, the rule that first derived it;
  - `_height`, its derivation depth.
- **A proof tree is rebuilt on demand:** `rr_why('calls', a, b)` looks up the rule's body through
  `rr_atom`, then finds body rows of lower height. It recurses until it reaches base facts.

| Row | What `_rule` points to |
|---|---|
| derived row | the rule |
| match fact | the match record's program, via `rr_plan`, which maps it to the matcher clause |
| SQL producer row | the producer, whose derivation names its SQL and its read set |
| row replayed from a memo | the memo's derivation. A deeper "why" re-runs that file's local program |
| lattice value | the rule of the last raise only, one level deep |

The format reserves room for the two columns now:
- the frames have space for them;
- semi-naive evaluation keeps the first derivation's rule.

They are filled only when provenance is turned on.

## 5. Driving evaluation from SQL

- **Step 1.** `SELECT rr_run(program, demand)` evaluates on the session's owner connection and
  leaves its relations readable as `rel_*`.
- **Step 2, later.** A `rel_*` constraint seeds demand directly: `xBestIndex` turns it into the
  magic-set seed and runs the missing waves. This waits on re-entrancy, because waves would run
  inside an `xFilter` on the same connection.

## Open questions

1. Producer granularity. A whole relation per run is proposed. A file-local producer, one whose
   query groups by document, could be split per document.
2. How a stale frame is reported to a query: an error, or a re-run.
3. Where `rr_materialize` tables live: the session's temp schema, or the store with collection
   (decision 0053).
4. Catalog persistence: by program digest beside `compiled_plans`, or rebuilt per session.
