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

**Built (step 1, `sql_executor/catalog.rs`).** These are temporary tables of the executor session,
rebuilt from the retained programs after every run, removal and reset. Every program the session
retains is one `execution`:

```text
rr_relation(execution, name, kind, arity, stratum, output, file_local, view)
  -- kind: input | derived | lattice | native
rr_column(execution, relation, ord, name, type)     -- the .decl's names and types
rr_rule(rule, execution, ord, head, text)
rr_atom(rule, ord, kind, relation, negated, aggregate, text)
  -- kind: atom | aggregate | compare | nearest | ast
rr_plan(execution, step, stratum, output, reads, sql)  -- reads: a JSON array
```

`rr_plan` holds the program's wave steps, which `wave_step` keeps only until the next run.

The `rr_` prefix is reserved: a program cannot declare a relation named `rr_…`.

**Not built yet:**
- the rules' source file and span, which the logic AST does not keep;
- persistence by program digest beside `compiled_plans`;
- `sql` kinds, which come with SQL producers (§3).

## 2. Reading relations: `rel_<name>` *(user: option C)*

Every relation is readable as `rr_rel_<name>`. Its column names and types come from its `.decl`.
The prefix is `rr_rel_` rather than `rel_` because rule packs already declare relations named
`rel_base`, `rel_expr` and so on.

**Built (step 1):** inside an executor session, `rr_rel_<name>` is a temporary view over the
session's relation, for every input and derived relation whose width matches its `.decl`. A
logically nullary relation has one column, `present`. The derived view wins over an input of the
same name from another program.

**Built (step 3): a corpus run's memo frames, through `bindCorpus(root, relations?)`.**
- A corpus run records, per memo plan, the relations its entries hold, in the additive table
  `corpus_plan_relations`. It is collected with the plan.
  - An export's row holds its `.decl` columns: its entries hold rows of values.
  - A file-local output's row holds `{"lines": columns}`: its entries hold rendered text lines.
- `bindCorpus` finds the root's newest manifest and that plan's frames for the manifest's sources.
  It binds each relation as a read-only input under its own name and `.decl` columns, plus
  `corpus_document(doc, path)`.
- Nothing is evaluated. SQL and rules both read the bound relations. A node's document is `n >> 32`.

**Built: lazy per-document reads (`sql_executor/corpus.rs`).**
- The frames stay compressed. Binding decompresses only each frame's first eight bytes, which
  hold its document key.
- Each relation's rows of a document are decoded when a statement first reads them, and kept.
  - A lookup on a `node` or `loc` column decodes the one document the value names. The virtual
    table takes a source's length only for a scan, so a lookup never makes every row.
  - A scan, or a lookup on another column, decodes every document once.
- The source says its rows are made on lookup, so joins probe it rather than scan it.
- A file-local output's lines are parsed when decoded. Its `int`, `node` and `loc` columns are
  parsed as decimals. Every other column is interned as a symbol, including kind and field names,
  which the lines hold as text.

Limits:
- Cross-file relations are not framed, so they cannot be bound.
- A frame that fails to decode fails the statement that reads it, naming the file
  (`Source::failure`, checked when the virtual table filters). The failure is the run's: later
  statements over it fail too, until the corpus is run again and bound again.
- A lookup scans its document's rows. There is no per-document index yet.

**Not built yet:** the remainder below, which is the target. That covers cross-file relations,
staleness, the overlay, materialisation and the decoding functions.

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

**Built (step 2, `sql_executor/producers.rs`), in executor sessions:**
- `.input r(IO=sql, query="…")` takes `r` out of the inputs and adds one step of its own stratum.
  The strata are then renumbered so that each stratum's number is above every stratum it reads
  from. Rules may read a producer, and maintenance and the wave count walk strata by number.
- SQLite resolves what the query reads. Inside a savepoint that is rolled back, each wave relation
  gets a placeholder table and its typed view, and an authorizer records the tables the prepared
  query reads. Emitter views count as the relations they read. The query must give the
  declaration's number of columns.
- A cycle through a producer is refused: "no recursion through SQL".
- `symbol` columns go through `sym_atom`.
- Pruning keeps every relation a producer's query names, so rules feeding a producer are not
  dropped.
- The catalog lists producers as kind `sql`. `rr_plan.reads` is the list SQLite resolved.
- **Incremental updates recompute the producer.** When anything a producer's query reads changes,
  `applyChanges` and friends recompute its stratum whole: they snapshot it, rerun its step and
  take the difference. This is the path maintenance already takes for non-monotone strata. Rules
  that read the producer are then maintained from that difference.
  - The query's reads come from the step's resolved wave relations, plus any session input it
    names (a relation bound with `bindRelation`, `bindCorpus` and the like). Those inputs join
    the execution's inputs, so a change to one reaches the program.

**Not built yet:**
- watching ordinary tables. A change made by plain SQL to a table a producer reads, such as a
  temporary table, is not seen: run the program again;
- the corpus path. Its input readers still refuse `IO=sql`.

**Target design:**

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

- **Every derived row has two hidden columns on `rr_rel_*`:**
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
