# SQL fact and rule execution

The SQL executor exposes facts, derived relations, and the content store on one SQLite connection.
It lowers `.rr` fact and rule programs through the existing typed SQL planner.
The existing wave scheduler runs those statements until their relations stop changing.
Planning reads bound row estimates and recorded wave counts. An SQL input without a recorded count uses a default estimate.
AST matching stays in the native matcher. Its output can enter SQL as another fact relation.

## Standalone command

Build the engine:

```sh
cargo build --locked --bin rerail-engine
```

Save this request as `request.json`:

```json
{
  "relations": [
    {
      "name": "edge",
      "columns": ["src", "dst"],
      "rows": [[1, 2], [2, 3], [3, 4]]
    }
  ],
  "program": ".decl edge(src: number, dst: number)\n.input edge\n.decl reach(src: number, dst: number)\nreach(x, y) :- edge(x, y).\nreach(x, z) :- reach(x, y), edge(y, z).\n.output reach\n",
  "dialect": "python",
  "max_waves": 1024,
  "queries": [
    {"sql": "SELECT c0, c1 FROM reach ORDER BY c0, c1"},
    {
      "sql": "SELECT dst FROM edge WHERE src = ?1",
      "parameters": [2],
      "max_rows": 100
    },
    {"sql": "SELECT wave, rel, rows FROM wave_delta ORDER BY wave, rel"}
  ]
}
```

Run the request:

```sh
target/debug/rerail-engine sql /tmp/rerail-extension.sqlite request.json
```

The response contains the emitted SQL plan, the wave count, and each query result.
The `reach` query returns six pairs, including `[1, 4]`.
The connection reads the supplied content store as `main`.
Virtual fact tables and derived tables belong to this session.
The command closes the session after the request.

The package CLI accepts the same request:

```sh
rerail engine sql /tmp/rerail-extension.sqlite request.json
```

For a corpus audit through the existing logic command, select SQL rule execution with `--sql`:

```sh
target/debug/rerail-engine logic rules.rr /path/to/corpus /tmp/sql-results --sql
```

This disables native rule components. Native fact producers and tree matching keep their existing execution path.
The corpus runner retains its chunking, symbol decoding, and output files.
Both SQL entry points use the same wave scheduler and lattice folds.
The extension session also exposes intermediate relations for queries.

## Native session API

The Node addon exports `CodeRuleSqlExecutor`.
The Rust tools API exports the same type as `rerail_engine::tools::CodeRuleSqlExecutor`.

```js
import { loadNativeBinding } from "@neralabs/rerail";

const binding = loadNativeBinding();
const session = new binding.CodeRuleSqlExecutor(storePath);
session.bindRelation(JSON.stringify({
  name: "edge",
  columns: ["src", "dst"],
  rows: [[1, 2], [2, 3]],
}));

const plan = JSON.parse(session.lower(programText, "python"));
const execution = JSON.parse(session.run(programText, "python", 1024));
const explanation = JSON.parse(session.explain());
const result = JSON.parse(session.query(
  "SELECT c1 FROM reach WHERE c0 = ?1 ORDER BY c1",
  "[1]",
  100,
));
```

| Method | Behavior |
|---|---|
| `bindSnapshot(snapshotJson)` | Replaces inputs with core fact virtual tables over a repository snapshot. |
| `applySnapshot(snapshotJson, maxWaves?)` | Rebuilds changed files and maintains retained SQL results atomically. |
| `applyFileChanges(changesJson, maxWaves?)` | Applies file additions, replacements, and deletions without another complete inventory. |
| `bindStored(scope, paths?)` | Binds current core documents from the content store without reading source files. |
| `bindRelation(relationJson)` | Adds or replaces facts with integer values or string symbols. Duplicate rows collapse. |
| `lower(program, dialect)` | Returns the setup SQL and wave program without executing its rules. |
| `explain()` | Returns logical proofs and retained execution choices without evaluating facts or executing SQL. |
| `run(program, dialect, maxWaves?)` | Runs an inline `.rr` program through the SQL wave scheduler. |
| `runFile(path, dialect, maxWaves?)` | Reads a `.rr` file, its includes, and `core.rr`, then runs it. |
| `applyChanges(changesJson, maxWaves?)` | Applies an atomic fact batch and maintains retained results and dependent programs. |
| `query(sql, parametersJson?, maxRows?)` | Returns columns, row arrays, and a `truncated` flag. |
| `reset()` | Clears generated outputs and keeps fact bindings and registered extensions. |
| `setLimits(limitsJson)` | Sets per-operation `max_steps` and `timeout_ms`; `{}` removes limits. |
| `createIndex(relation, columns)` | Retains an index on a session-owned result. Columns are zero-based positions. |
| `registerLattice(name, specificationJson)` | Registers SQL aggregate, merge, and widening functions for a custom lattice. |

An inline program declares its inputs. `runFile` uses the normal file reader and its core prelude.
The dialect controls the kind and field constants that the planner lowers to integer atoms.
It does not select a SQL dialect: execution uses SQLite.

Results remain available for later SQL queries and rule programs on the same session.
A later program can declare a previous output as an input.
The `.output` directive selects the derived relations to compute. Their intermediate relations also remain available for queries.
Declared outputs retain SQL tables, including their indexes and publication hooks.
Fully retained programs expose intermediate relations through immutable native query facades.
These facades support `createIndex()` and inputs to later programs. Their publication shares retained roots instead of copying SQL rows.
Output names must not overlap persistent SQL objects or bound inputs.
A later run can atomically replace session-owned outputs. It invalidates their dependent programs.
Rebinding an input invalidates programs that read it and their dependents. Independent outputs remain available.
Calling `bindSnapshot` clears all generated results. `applySnapshot` maintains them from native fact deltas.
Rebinding a relation preserves its column names and arity.

A failed run rolls back its SQL writes. It does not publish partial derived results.
The default wave limit is 1024.
Queries accept positional JSON scalar parameters or tagged integers and return at most 10,000 rows by default.
Queries require one read-only statement with result columns.
Blob values return as `{"blob": [byte, ...]}`.
Integers outside JavaScript's safe range return as `{"integer": "9007199254740993"}`.
Pass that same tag in query parameters and fact rows to preserve signed 64-bit values.
Convert the tag with `BigInt(value.integer)` when a JavaScript integer is required.

Fact bindings can include `types`, with one entry per column: `number`, `symbol`, `node`, or `loc`.
Strings become interned symbol IDs; numeric columns reject strings. Integer values can also supply existing symbol IDs.
For example, `{"name":"label","columns":["id","text"],"types":["number","symbol"],"rows":[[1,"hello"]]}` binds one label.

Limits apply independently to each rule run, query, or fact-change batch.
Work budgets check every SQLite VM step, including short statements; deadline-only limits check every 1,000 steps.
Work-budget checks add callback overhead. Unlimited operations do not install a progress callback.
They cover setup, indexes, and wave statements. A limit failure rolls back a rule run and preserves its previous outputs.
The progress callback is removed before rollback. Later operations can reuse the session.
These limits do not interrupt Rust compilation or a native function before it returns control to SQLite.
The request-file API accepts `limits` and an `indexes` array of `{relation, columns}` objects.

## Retained fact updates

Use `applyChanges` to update facts supplied through `bindRelation`:

```js
const update = JSON.parse(session.applyChanges(JSON.stringify([
  {name: "edge", insert: [[3, 4]], delete: [[1, 2]]},
])));
const result = JSON.parse(session.query("SELECT * FROM reach ORDER BY c0,c1"));
```

Each batch is a JSON array of `{name, insert?, delete?}` objects.
Rows use the binding's logical arity, types, symbols, and exact integer encoding.
An empty row represents a Boolean fact. Duplicate insertions and missing deletions have no effect.
If a batch inserts and deletes the same row, insertion takes precedence.
Several objects for one relation combine into one change set.

The session retains checked query plans and result indexes.
Positive rules propagate inserted rows through delta queries.
For recursive deletions, the engine removes possible consequences, then rederives facts with surviving support.
This removes unsupported cycles and preserves facts with alternate derivations.
Actual output changes propagate through dependent programs. Independent results remain available.

Eligible nonrecursive aggregate and negation rules retain reducer state and indexed positive rule contexts.
Counts and sums consume signed fact deltas. Integer sums fail the batch on signed 64-bit overflow.
Min and max retain indexed value multiplicities. Deleting one support preserves a value with other supports.
Negation updates its output only when the matching count crosses zero.
Per-context contributions and head support counts preserve outputs with alternate derivations or multiple rules.
Actual output deltas continue through later strata and dependent programs.

The retained path supports scalar `count`, `sum`, `min`, `max`, and ordered string `join` over one relation, including global aggregates and empty groups.
String joins retain source tuple multiplicities. They preserve text ordering, duplicate elimination, and the empty string default.
An edit rebuilds the changed string group; it does not rebuild unrelated groups.
Correlated subqueries require equality between inner and outer expressions. Constant filters and repeated inner variables also work.
Positive rule bodies can join several relations. Reducers can read base inputs or completed derived relations.

Computed equality correlations use retained group keys and expression indexes over their outer contexts.
When a correlation depends on an earlier aggregate, a separate dependency table indexes its current key.
Changes probe the old dependency index and update it for dirty contexts before evaluating new contributions.
Nonrecursive lattice outputs retain candidate multiplicities and fold only changed keys.
This includes flat, min, max, and registered deterministic joins. Retractions can restore a concrete value from top.
Min/max folds use indexed extrema. Registered joins use a persistent AVL summary tree for each key.
A candidate support crossing zero changes only a logarithmic number of tree nodes.
The registered merge must obey the lattice laws: associativity, commutativity, idempotence, and an absorbing top.
The declared aggregate and merge must compute the same join.

The shared `logic::analysis` module computes uniqueness and scalar-summary compatibility without an executor, database connection, or row statistics.
It propagates input keys through equalities, constant filters, joins, and projections.
Its reports distinguish unique logical tuples from unique rule witnesses, and include the evidence for inferred keys.
A full cache build omits head support grouping only when one rule has a proven unique witness for each head.
Multiple producers, hidden input columns, and opaque operations retain grouping. Incremental signed contributions still require reconciliation.

Compatible scalar `count`, `sum`, `min`, and `max` sites across rules share summary maintenance within one checked execution.
They must read the same input version, non-null bag, filters, grouping keys, and value expression.
Different outer lookup mappings keep their own contexts and probes while sharing the stored summary.
Count prefers a compatible local value projection and cannot connect families with different value expressions.
Separate wave gates retain independent input versions and maintenance histories.
Shared sums retain exact 128-bit state and check the public 64-bit range only when a rule reads the result.
Min and max share value multiplicities and retain indexed extrema probes after deletions.
Empty count and sum remain zero. Empty min and max remain absent.
Ordered joins, existence tests, nullable contracts, dependent keys, and opaque expressions keep separate summaries with explicit reasons.

A compatible family uses one indexed left join per retained context, including its head checks and projected values.
Missing groups preserve the original defaults. Exact sum range checks stay at their original expression uses.
The shared analysis reports the complete grouping key as the proof for a single summary row.

Pure, independent scalar summaries can also produce the first answer during cold execution.
The scheduler waits for upstream strata, initializes retained support once, and publishes its result through the ordinary wave fold.
Checked outer expressions, dependent aggregates, opaque producers, and lattice folding retain the ordinary cold path.
Walk lowering can expose additional checked rules. Those descriptors are planned after lowering without rebuilding already initialized summaries.
The independent cold verification mode continues to execute the original scalar queries.

Compatible consumer transitions share one SQL support phase for contexts, contributions, signed support counts, and output deltas.
Materialization fences retain each checked evaluation at its original use. SQL still computes every rule and aggregate result.
Immutable dirty-key snapshots survive scratch reuse within a maintenance pass. Later passes and batches start with an empty cache.
Failed transactions restore summaries, contexts, contributions, outputs, and native roots before retry.

Ordinary summaries stay in compact SQLite tables. SQL computes a shared update phase and stages its result in native scratch storage.
Bulk SQL statements publish those rows in the fact batch's transaction. Host code does not evaluate reducers.
Recursive wave caches use native versions when they need retained history.
Logical proofs use shared immutable storage, so copying an execution does not copy each proof.
Empty native destinations publish validated bulk roots directly. They do not insert every row a second time.
Identical ordered replacements reuse their complete roots. Other replacements preserve row IDs and share unchanged subtrees.
Native positive phases can reconstruct pure contribution projections from retained contexts instead of storing duplicate rows.
Computed heads and checked predicates keep materialized results. Explain reports the proof and the applied storage choice.
The reconstruction query uses shared immutable text across frames. Tree priorities use each key's existing digest without a duplicate node field.
Validated unique primary prefixes already order complete rows, so bulk construction omits a second sort.
A unique witness proof also permits incremental head-set differences without support grouping.
Those differences supply output deltas directly. Other paths retain signed reconciliation.
Equal ordered initial outputs can share validated roots within a phase. Version differences retain existing row cells and row IDs.

`lower()` and `run()` include an `analysis` object with relation keys, key inferences, rule uniqueness, and scalar compatibility checks.
`explain()` adds retained cache choices, cold reuse decisions, shared lookup counts, native publication strategies, and recursive operator analyses.
Its `scalar_storage` reports include global families, representative witnesses, candidate partitions, and named storage groups.
Rule reports identify context stores, reconstructed contributions, retained contribution stores, and support-phase admission decisions.
It also reports opaque SQL producers. Rejected optimizations include their reasons.
The native planner also applies the shared bag analysis to count, sum, min, and max steps.
Compatible consumers share one fold per demanded group and input version, including consumers in separate rules.
Each consumer keeps its own lookup mapping. Count can adopt one compatible value family without connecting different value expressions.
Rebound inputs own new caches. Component caches distinguish the complete relation from each visible delta range.
Dependent scalars retain separate evaluation. Ordered strings and unsupported expressions retain their SQL fallback.

Native sums retain wide values through aliases and check the final range when an operation observes the value.
Filters can remove rows before that use. Unused sum bindings retain no range check.
Native execution and SQL fallback use the same final sum range check.
The generated `rr_exact_sum` aggregate preserves cancellation before that check. SQLite's built-in `sum` keeps its usual behavior.
Native explain records the compatibility analysis, applied cache boundary, and sum contract.
Profiling records the native summary fold and its inspected values.
SQL profiling receipts include these proofs as `logicalAnalysis` and `logicalAnalyses` context fields.
The generated Markdown and HTML profiles show them beside each query's SQLite plan and bytecode.

The SQL catalog exposes the same information through `rr_analysis(execution, analysis)`, where `analysis` contains JSON:

```sql
SELECT execution, json_extract(analysis, '$.logical.key_inferences') FROM rr_analysis;
```

Catalog queries refresh invalidated metadata on demand. Runs, removals, resets, and successful fact edits invalidate it.
Ordinary fact queries do not require a catalog rebuild. Metadata SQL still counts against the complete-job execution allowance.

Native builtin joins and SQL planning share a reduction descriptor and grouping compatibility rules.
The descriptor identifies flat, min, max, or opaque operations. Flat descriptors include their top value.
SQL wave folds reuse a single operator's grouped result when its reduction and prefix keys match.
Multiple operators still need a fold. Builtin folds accept duplicate inputs, so their candidate unions use `UNION ALL`.
Flat reductions detect conflicting values with min/max, without a separate distinct-value count.
These changes preserve support multiplicities, empty groups, previous-wave merges, and widening order.
External SQL callbacks remain opaque. JSON registration cannot supply the planner's builtin proof.
Support counts group complete candidate tuples; lattice results group their key prefix. These groups still require separate work.

Nonrecursive rules with generated tree walks retain their seed, walk, and result relations.
Their fact changes use overdelete/rederive through indexed SQL relations. AST matching remains native.
Walk seeds and outputs can contain supported scalar subqueries and negation.
The lifted relations use separate dependency components, so reducers complete before their recursive walk consumers.
Nullable values carried through a walk use validity columns, including empty min/max results.
Recursive lattice and aggregate components can retain a proven independent partition key.
Every recursive read, including aggregate reads, must stay in its head's partition.
Changes discover affected keys from old and new witnesses, clear those partitions and their widening counters, and rederive them from bottom.
The wave kernel keeps its original step order and delta variants. This removes unsupported cyclic support without touching unrelated partitions.
The key can occupy different column positions in different relations. Probe indexes are prepared during the initial run.
Components without this proof and unsupported subqueries still refresh their affected strata.
Constant fact steps use immutable private tables and checked queries, so they share support maintenance with other rules.
The compiler selects eligibility from the checked query tree. It does not parse rendered SQL.

Each program report includes `outputs`, `positive_waves`, `overdelete_waves`, `refreshed_strata`, and `migrated_views`.
`incremental_strata` lists relations maintained through retained reducers or candidate lattice folds.
Positive tree walks use `positive_waves` and `overdelete_waves`.
`recursive_partitions` counts partitions rederived across recursive components.
`reducer_groups` counts processed reducer groups and lattice keys. `context_rows` counts rule contexts reconsidered by this path.
The batch report includes `inserted_facts`, `deleted_facts`, `migrated_inputs`, and `programs`.
`run()` reports `preparation`, with initialized relation names, scheduled projection count (`projection_relations`), constant tables, and scratch tables created.
`materialized_views` remains a compatibility field. Projection rules enter the checked relation graph before execution.
Change reports retain `initializations` and `migrated_inputs` as empty compatibility fields; `migrated_views` is zero.
`scratch_tables_created` reports new scratch schemas required by the operation.
A no-op batch returns no program reports.

`bindRelation()` builds the mutable fact source and its indexes. Initial execution prepares projection relations and retained support.
Projection rules use the same dependency graph and wave gates as other checked relations.
The executor does not convert live views into a separate maintenance pipeline.
Declared outputs remain SQL tables, so SQL triggers, indexes, and transactional publication keep their existing behavior.
These costs remain proportional to input size, distinct values, and positive body combinations, and occur before the first edit.
A large join can require many retained contexts. Compatible reducers share their bag state and value indexes.
A single eligible count, sum, min, or max uses the same grouped SQL publication protocol as a compatible family.
Lookup sharing requires its own proof. Dependent, opaque, nullable, and ordered cases preserve their required evaluation boundaries.
The engine collects statistics on materialized projections before preparing support joins.
First edits and later updates operate on changed candidates, affected contexts, and retained indexes.
Typed scratch tables remain empty between operations. An operation reuses existing tables and allocates more only when it exceeds the previous capacity.
Rollback restores both scratch rows and pool metadata. Reset drops the pool. Peak schema capacity remains allocated until reset.
A changed global aggregate can affect every outer context that reads it. That fan-out remains real work.
Deleting a highly connected fact can still affect most of a result.
Overdelete/rederive can also revisit many rows that ultimately keep alternate support.

A failed batch restores fact bindings, SQL rows, reducer support, projection views, and ownership metadata together.
The connection must have no active cursor or caller transaction.
The wave limit applies to each propagation phase; SQL work and deadline limits cover the batch's SQL statements.
Native input migration and atom interning run outside the SQLite VM budget.
Successful symbol interning may persist even when the fact batch fails.

Core snapshot facts remain native virtual tables. Use `applyFileChanges` or `applySnapshot` to maintain results across source edits.
Row changes currently require an explicit `bindRelation` input.
Replacing a binding still invalidates its dependents; use `applyChanges` to maintain them.
Rust clients can query the underlying connection, but direct writes to owned results bypass maintenance.

Both CLIs accept `changes`, an array of batches, after the initial program and before `queries`:

```json
{
  "changes": [
    [{"name": "edge", "insert": [[3, 4]]}],
    [{"name": "edge", "delete": [[1, 2]]}]
  ]
}
```

The response includes one `updates` report per batch.
For queries between changes, use the retained session API.

## Core facts and symbols

For facts already in the Rerail content store, bind their scope:

```js
session.bindStored("/path/to/worktree");
// Or select explicit relative paths:
session.bindStored("/path/to/worktree", ["src/a.py", "src/b.py"]);
```

This reads stored tapes and checks their core digests. It does not parse source files again.
The binding captures the current facts. Another call refreshes it after the store changes.
Missing or incomplete requested documents fail the binding.

The request-file equivalent is `"stored": {"scope": "/path/to/worktree", "paths": ["src/a.py"]}`.
Without `paths`, the binding includes every current core document in that scope.
One request accepts either `stored` or `snapshot`.

`bindSnapshot` accepts the engine's repository snapshot shape:

```js
session.bindSnapshot(JSON.stringify({
  format: "rerail.repository-snapshot/v1",
  revision: 0,
  files: [{path: "a.py", language: "python", source: "answer = 42\n"}],
}));

const documents = JSON.parse(session.query(
  "SELECT d, sym_text(path), sym_text(module) FROM doc",
));
```

The snapshot binds the `core.rr` projection, including `node`, `child`, `field`, `tape`, `name`, and source-location relations.
These relations read native columns through virtual tables. Core input relations remain virtual.
Retained rule maintenance can materialize positive contexts from these columns.
Incomplete parses fail the binding instead of supplying partial facts.

After `bindSnapshot`, use `applySnapshot` to retain SQL results across source edits:

```js
const report = JSON.parse(session.applySnapshot(JSON.stringify({
  files: [
    { path: "a.py", language: "python", source: "def renamed():\n    return 43\n" },
  ],
})));
```

This is a complete replacement snapshot. An omitted file is removed. An unchanged file reuses its native fact sources.
Only changed files rebuild their core facts. The scheduler receives tuple deltas for families that retained programs read.
Other families are rebound without enumerating their rows. SQL queries also see the new facts.
The snapshot includes native `within` and `span_within` tables; their rows are produced on lookup.
AST matching remains native.

The response includes `changed_files`, `rebuilt_files`, journaled fact counts, and the usual per-program maintenance reports.
Native fact counts cover families read by retained programs; other families are rebound without tuple journals.
Generated tables and their indexes survive successful edits. Independent `bindRelation` inputs also survive.
A parse error or SQL failure preserves the old bindings, document membership, and results. Content-cache entries and interned atoms may remain.
`applySnapshot` requires an initial `bindSnapshot`; `bindStored` does not establish editable snapshot membership.
A native snapshot family cannot be overwritten with `bindRelation`.

For editor updates, use `applyFileChanges`:

```js
const report = JSON.parse(session.applyFileChanges(JSON.stringify({
  upsert: [
    {path: "a.py", language: "python", source: "def renamed():\n    return 44\n"},
  ],
  delete: ["obsolete.py"],
})));
```

Unmentioned files keep their sources and metadata. `upsert` uses the same file shape as `bindSnapshot`.
An existing path is replaced. A missing deletion is a no-op.
Duplicate paths, including a path in both lists, fail the entire batch. Parse and SQL failures preserve the previous state.
Both lists are optional. `applyFileChanges` requires an initial `bindSnapshot` and accepts the same optional wave limit.
The request-file API accepts a `file_changes` array of these batches.

Each fact family retains an immutable balanced document index with cached subtree estimates.
An edit copies only the changed search paths. Shared literal facts retain document reference counts and indexes on both columns.
These indexes preserve old versions during SQL maintenance and rollback. They do not copy AST rows into SQL.
Complete snapshots use these same indexes. Reading the submitted inventory and finding omitted paths still requires a complete inventory scan.

The response includes `examined_files` and `metadata_nodes`.
`examined_files` counts submitted file entries and delete paths. Complete snapshots count both old and submitted inventories.
`metadata_nodes` counts rebuilt index nodes across document families and shared literal indexes.
One-file patches rebuild a number of metadata nodes proportional to the index depth, rather than the number of documents.
Initial program preparation can read all participating facts. File edits do not migrate projections or initialize retained support.
Queries without a document constraint can probe several document shards to find their results.
A retained program that reads containment can enumerate the changed file's containment pairs for its journal.
JSON parsing and native fact preparation remain outside SQL instruction limits.

Symbols are integer atoms. `sym_text(atom)` reads their text, and `sym_atom(text)` interns text.
The connection also registers native lattice and text functions, including Unicode `lower`, `sha256`, and `rr_join`.
Kind and field atoms have separate domains. The rule planner lowers their string constants using the selected dialect.
Use `kind_name('python', kind)` and `field_name('python', f)` to decode those domains in SQL.
An unknown ID returns SQL `NULL`; an unknown dialect fails the query.

Arbitrary SQL can join these virtual tables, persistent store tables, and completed derived relations.
For example, a query can count names per document or compare findings from two derived relations.
Additional producers can bind integer facts without changing the SQL compiler.
Rust extensions can register SQL functions or virtual tables through `session.connection()`.
The connection belongs to the session, so Rust statements and cursors borrow that session.

## Catalog and typed views

Runs, removals, resets, and successful fact changes invalidate the catalog.
A catalog query refreshes these temporary tables:
- `rr_relation`, `rr_column`, `rr_rule`, `rr_atom` and `rr_plan` (`sql_executor/catalog.rs`);
- for each relation, the typed view `rr_rel_<name>`, whose columns are the `.decl`'s names.

For example, `SELECT src, dst FROM rr_rel_reach` reads `reach`, whose own columns are `c0, c1`. The `rr_` prefix is reserved.
Design: `docs/sql-access/01-design.md`.

### A corpus run's rows

`bindCorpus(root, relations?)` binds the relations that the newest corpus run of `root` memoised per file:
- each export, from the rows the run wrote, under its declared columns;
- each file-local output, from the text lines the run wrote. Number and node columns are parsed;
  other columns are interned as symbols;
- `corpus_document(doc, path)`, which maps each document key to its file. A node's document is
  `n >> 32`.

Nothing is evaluated: SQL and rules read the bound relations as read-only inputs. `applyChanges`
does not change them.

Frames are read lazily:
- binding reads only each frame's document key, from its first eight decompressed bytes;
- a lookup on a node or location column decodes the one document it names;
- a scan, or a lookup on another column, decodes every document once.

The result names each relation's form: `{"files": 2, "relations": {"def": "rows", "named": "lines"}}`.

### SQL producers

A relation can take its rows from a SQL query:
- `.input r(IO=sql, query="SELECT …")`.
- The query may read session tables and the program's own relations, by name or through `rr_rel_<name>`.
- The relation's step runs once every relation the query reads is complete. Rules that read `r` run after it; strata are renumbered so their numbers follow this order.
- Recursion through a producer is refused.
- `symbol` columns are interned.
- `applyChanges` recomputes a producer when a relation or session input its query reads changes, and maintains its readers from the difference.
- A change made by plain SQL to another table is not seen: run the program again.

## Execution boundary

The planner emits seed statements and recursive delta statements.
The cold kernel selects pending steps through `wave_step`, `wave_read`, `wave_after`, and `wave_pending`.
Retained execution preserves those dependency gates across immutable wave versions.
SQL evaluates rule expressions and folds candidate rows into completed relations.
Both paths record initial wave deltas in `wave_delta`.
Negation and aggregation wait for their input strata to finish.
Derived columns use positional names: `c0`, `c1`, and so on.

The extension API supports set relations and flat, min, and max lattices.
The corpus runner uses the same SQL folds, physical Boolean representation, and top propagation.
It retains its statistics, native dispatch, file chunks, and cross-file round limits.
A custom lattice in the native registry becomes deterministic SQL aggregate, merge, and widening functions on either connection.
Min and max require the rule's normal `widen` declaration. Improved values form the next wave's delta.
Zero-column Boolean relations use one physical sentinel column, `c0 = 1`.
Zero-key lattices also use that sentinel; their value is `c1`.
Plans and execution responses include `logical_arities` so clients can distinguish logical and physical columns.
An aggregate-only rule can compute a global result, including a count of zero over an empty input.

Custom lattices use registered deterministic SQL functions:

```js
session.registerLattice("bits", JSON.stringify({
  aggregate: "bit_or_aggregate", // One value argument, one combined value per key.
  merge: "bit_union",          // Two present values, their least upper bound.
  widen: "bit_union",          // Old and improved values, a terminating upper bound.
  top: 7,
}));
```

Register the functions through the Rust connection before this call.
The aggregate and merge must implement the same associative, commutative, idempotent join; top must absorb every value.
The widening function must terminate ascending chains. Registration checks function presence, arity, and deterministic flags.
The caller supplies the algebraic contract. The executor does not prove it.
Unknown custom lattices fail explicitly, and registering new lattice operations clears previous results.

Deletion reads old native inputs through temporary indexed overlays over current facts and actual deletions.
The overlays preserve lazy native lookups and are removed after each stratum, including on failure.
For an old SQL result, the executor emits separate current/deleted query branches that retain the table's indexes.
Many changed SQL atoms in one rule can produce many branches; this case still needs a compact indexed representation.

Mixed derived joins leave their ordering to SQLite. Single-delta joins keep the delta anchor and native input probes.
Base-only joins retain the planner's source estimates. Checked rule equality constraints request reusable indexes.
`createIndex` adds indexes for ad hoc access patterns. These indexes survive queries and disappear with their owning result.
Compiler projection views receive private names when their public name already belongs to a bound fact table.
An AST matcher atom must become native match facts before the rule enters this executor.
The standalone session does not publish derivation records or replace the product audit path.
The product path continues to use the same SQL scheduler with its provenance and incremental storage.

## Retained wave execution

Supported recursive lattice and aggregate components now use retained iteration state, including components with proven partition keys.
Each frame reads preceding-wave versions, so cycles cannot preserve their own initial support.
Edits propagate through cached operators and publish changed rows without whole-stratum or whole-partition rederivation.

The internal backend provides versioned integer relations through writable temporary virtual tables.
Frames share fixed SQL slots. Forking a frame shares persistent indexes without copying support rows or creating another schema.
The session restores native roots alongside SQL savepoints after failure.
Cached improvements preserve the cold scheduler's pending gates, constant-seed timing, and widening counters.
A closing frame retains the empty final frontier. Clearing its positive contexts preserves scalar input indexes.
Sum indexes retain exact 128-bit totals in two integer columns. A rule lookup checks the public 64-bit range, so unused groups do not raise overflow errors.

The update report includes `recursive_frames` per program and `version_nodes` for the batch.
The latter counts native version-diff traversal, which SQLite instruction counts do not include.

Generated walks support scalar queries and negation in their seed and output blocks; their traversal arms remain positive.
Eligible generated walks remain recursive SQL kernels inside a shared phase.
They preserve the original UNION and NULL semantics, without separate SQL submissions for each traversal iteration.
Equal input versions and empty journals skip kernel evaluation.
Opaque calls, unsupported functions, and lattice heads retain their existing specialized paths.
Aggregate bodies with joins lower to retained helper relations. `split_dot` retains its immutable integer domain.
Acyclic positive CTEs can use arbitrary names when the final SELECT uses DISTINCT.
Their arms can use UNION or UNION ALL. The final DISTINCT makes duplicate derivations safe to retain as sets.
This path currently excludes function calls, scalar subqueries, and aggregate consumers.
Bag-sensitive CTEs and unsupported scalar shapes keep the explicit refresh or partition fallback.
Supported recursive programs construct retained state and publish initial answers through one global wave schedule.
The schedule preserves dependency gates across components, constant seeds, frontiers, and widening counters.
Preparation reports `initial_execution: "retained"` for this path and `"cold"` for the established fallback.
Mixed frames compose compatible support and fold queries around separate operator boundaries.
A scalar boundary does not force compatible neighbors onto statement-by-statement reconciliation.
Earlier gate activation accounts for adopted template rows before it computes improvements.
Specialized walkers, pooled acyclic scalar programs, and unsupported graphs retain their established kernels.
Explain includes those boundaries and the actual retained operator plans.

Initial loads bulk-build complete-row, primary-key, row-id, and required secondary roots.
Dense initial joins reuse unchanged rows and tree branches. Sparse edits keep their delta path.
Frozen lookups share an immutable version. Contiguous scans stream through that version without repeated rank lookups.
Scratch deltas share roots and prepare secondary indexes only when a lookup needs them.
Empty pending journals also share incoming roots. Equal opposite journals cancel directly. Large journal inputs compose through a bulk set merge.
Native journal composition has its own profile operation, including incoming and retained row counts.
Bulk loads preserve the integer checks of the writable virtual tables.

Positive caches and eligible recursive SQL kernels combine their outputs in bounded SQL batches.
Each batch stays below SQLite's 500-arm compound-query limit. Sizing includes key journals and publication results.
Every operator reads the same frozen wave inputs, even when its caches publish in an earlier batch.
Immutable key journals survive scratch reuse. Relation folds run after all operator caches finish.
Frame `phase_batches` and profile `phaseBatch` contexts expose these boundaries.
Exact zero native row counts can clear nonscalar positive-body caches without scanning their contexts.
Scalar input cursors and their positive contexts remain consistent during suspension. Protected journals survive until reactivation.
Each operator's `empty_positive_body` reports the selected shortcut.
Materialized CTEs evaluate the expressions. A tagged result stream returns the affected rows and keys.
Native code validates every replacement before publication and maintains persistent storage roots.
Complete cache rebuilds use inline views for pure projections and set operations.
They replace the full improvement relation and omit old rows that the rebuild already covers.
Checked expressions and native functions retain materialization fences, so shared consumers do not repeat their evaluation.
Sparse updates retain their existing materialization and prepared query shapes.
Scalar reducers, registered lattice joins, and other checked shapes retain their existing SQL paths.
Merge and widening functions still execute through SQLite.

Private phase outputs and fact deltas use read-only native bindings, which avoid CREATE and DROP statements.
Integer constants also use immutable native bindings. Their typed values come from the compiler, without SQL text parsing or scratch-table replay.
The compatibility field `constant_tables` includes these native constant bindings.
Initial execution creates public results and wave diagnostics first. The cold fallback declares its remaining control tables when needed.
Widening counters retain their `__sql_changes_RELATION` names and `changes` columns as read-only diagnostics.
These diagnostics share retained roots. Additional indexes requested by extensions survive sparse publication and rollback.

An earlier frame can become active when an edit shortens an upstream dependency gate.
Immutable initial templates supply its retained roots. Accumulated journals reconcile external edits before that frame evaluates.
Native savepoints restore roots, bindings, and schemas if initial execution fails.
See the [implementation plan](sql-unification-and-updates-plan.md#retained-recursive-iterations) for measurements, root causes, and remaining work.

## Explain and profiling

SQL sessions use the shared query profiler, including the standalone headless engine.
Set `RERAIL_SQL_PROFILE_DIR` before the process opens a session:

```sh
RERAIL_SQL_PROFILE_DIR=/tmp/sql-profile/raw rerail-engine sql store.sqlite request.json
node scripts/profiling/sql-report.mjs --sql-session /tmp/sql-profile/raw /tmp/sql-profile/report
```

The report contains SQL text, EXPLAIN plans, bytecode, CPU time, VM instructions, and native table counters.
Explain includes disjoint-producer proofs, dependent bag-storage decisions, and the requirements for shared transitions across wave gates.
Shared transitions require the same input snapshot, prior summary roots, pending delta journal, projection, layout, and ordering.
Each consumer retains its own lookup mapping and dependency index. Nullable values and opaque callbacks retain their existing effect boundaries.
Compatible acyclic positive stages pass replacement values and signed deltas through one SQL phase.
Each recursive step still reads frozen preceding-frame inputs.
Checked expressions retain their scheduler gate and materialized context fence while sharing compatible SQL publication.
Independent count, existence, and count/sum/min/max transitions can share the consumer support and wave-fold statement.
Staged summary lookups probe the persistent old-group index and the small replacement sets separately.
Independent gates retain separate cursor roots, even when a proven transition shares its evaluation.

Cold summary initialization can share the consumer and publication phase.
Group and value-index construction use one projection when their input versions, projection, layout, and ordering agree.
Only successful publication adds reusable initialization state.
Dependent, ordered, computed-key, and nullable reducers use native roots with explicit evaluation fences.
Their dependency refresh and ordered head evaluation retain the original sequence.
Registered lattice support indexes use persistent native nodes and roots. SQL still invokes each merge callback.
The profiler records native index CPU under `registered lattice support index`.

Ordinary cold execution also uses native candidate, delta, and widening-counter roots.
Its readiness gates and emitted step order preserve the cold SQL wave contract.
The invariant `rdl_int` split-position domain also uses a native query source without helper DDL.
SQL evaluates rules, merges, and widening. Native scheduling needs no private pending-step tables.
Public result tables keep their SQL triggers and extension indexes.
Explain reports this adapter under `cold_frontiers`; the original compiled relation inventory appears under `relation_interface`.
The independent cold oracle and explicit legacy adapter tests retain writable SQL storage.
Opaque-producer fallback keeps generated private projections in native roots.
SQL ANALYZE applies to SQL-owned relations. Native sources supply their own cardinality and index statistics.
Opaque producers still prevent whole-program retained composition, so mixed programs can require substantially more SQL submissions.

The explain output includes `dependency_components`, `acyclic_stages`, `acyclic_sql_depth_bound`, and per-operator `common_wave_phase` decisions.
Dependent incremental batches currently contain at most four stages because SQLite expands nested CTE references during preparation.
Result-arm bounds can introduce further SQL batches. These boundaries retain immutable roots and pending signed deltas.
Closing and inactive history frames carry native roots without SQL. Equal versions and synchronized cursors permit complete frame reuse.

Frame work records separate acyclic stages, recursive evaluation, closing, and retained history.
Component records distinguish seeds, recursive inputs, recursive candidate improvements, and output frontiers.
A productive-feedback frame has nonempty recursive inputs, candidate improvements, and a new component frontier.
This metric describes the current algorithm and does not prove a semantic lower bound on SQL submissions.

Contexts identify the API operation, preparation stage, recursive component, frame, relation, and rule step.
Native version differences have separate operator records. Failed operations and rollback remain in the trace.
Profile checkpoints retain an encoded prefix of immutable query records and append only new records.
The profiler copies that prefix and replaces metadata files atomically.
An unchanged checkpoint serializes zero query records. Filesystem copies can still require I/O.
A failed metadata write preserves the previous complete snapshot.
Retained support initialization records exclusive native CPU, excluding child SQL and child operators.

Native table counters include compound-key prefix probes and candidate inspections, including candidates that SQLite rejects.
The benchmark probe reuses this profiler when capture is active, preserving SQLite's single callback.

Fully retained schedules expose immutable row-count metadata through `temp.wave_stats`.
Edits publish those counts without helper SQL writes. Rollback restores the previous metadata snapshot.
A later cold program atomically restores the writable SQL table before its scheduler runs.
SQL views and text predicates can read the same count interface.

Fused phase preparation has its own CPU span and report category.
Its context records SQL bytes, CTE definitions, result branches, and logical reads.
`explain()` describes result sharing and its evaluation boundary.
A phase can distribute one materialized result to several destinations.
Each destination retains its own key width and constraint checks.

Shared analysis proves containment between key sets through source identities, projections, copies, and unions.
It describes set membership only. It does not discard bag multiplicity.
The SQL emitter omits old-row branches when the replacement keys cover every affected key.
`explain()` reports these decisions under each frame's `key_set_decisions`, including rejected proofs and omitted branches.
Unproven membership subtraction uses a materialized SQL `EXCEPT`, avoiding correlated scans of the replacement set.
Ordered extrema use indexed probes with non-null tuple membership filters. Nullable values retain their separate path.
The shared analysis has no SQL or storage dependencies, so native execution can use the same proofs.

Phase publication selects bulk rebuilding when at least 512 old rows exist and the affected-key count reaches half of the old row count.
Sparse updates retain persistent patches. Both paths preserve existing row identities, constraints, and atomic publication across roots.
Frame `publication_decisions` and SQL profile receipts report the selected strategies.

The session driver defaults to a five-minute timeout per verifier process.
`RERAIL_SQL_VERIFY_TIMEOUT_MS` permits a longer debug diagnostic. It does not change the 100-submission allowance.
The driver records this setting and reports process timeout errors.

The standalone report is diagnostic. It does not supply an output oracle or grant performance acceptance.
The existing complete-job execution allowance and missing-parity checks remain visible in the report.

The headless verifier supplies complete-state parity for explicit fact requests:

```sh
cargo build --profile dev --no-default-features --features headless,lang-python,lang-rr --example sql-session-verify
python3 benchmark/sql-executor.py --session-request 1000 /tmp/request.json
node scripts/profiling/sql-session.mjs target/debug/examples/sql-session-verify /tmp/request.json /tmp/sql-session-1000
```

The output directory must be fresh. The command writes the report before it enforces acceptance.
It exits unsuccessfully when parity, capture coverage, or the fixed 100-execution allowance fails.
The independent cold kernel runs in a separate process without SQL capture.
Every checkpoint includes all explicit facts, derived relations, and widening counters.
The adapter also compares output deltas, failed-operation rollback, recovery, and final queries.
Integer cells use decimal strings, so JavaScript does not round them.
The verifier also accepts an initial `snapshot` and snapshot/file publication steps.
Each step supplies exactly one action: `changes`, `snapshot`, or `file_changes`.
`file_changes` uses the session API's `upsert` and `delete` fields.

Every checkpoint includes the native inputs that the program reads, derived tables, projection views, and widening counters.
Document checkpoints also compare every path, language, and source text.
Symbols compare by text. Nodes compare by document path and local identity. Direct document-key copies compare by path.
Numeric cells retain exact decimal strings. Queries that return symbols can use `sym_text` to compare their text across stores.
Failure checkpoints preserve the previous facts and documents. Successful retries must match a fresh cold execution.

The document generator covers replacement snapshots, file additions and deletions, no-op edits, restoration, and empty inputs:

```sh
python3 benchmark/sql-executor.py --document-session-request 32 /tmp/documents.json
node scripts/profiling/sql-session.mjs target/debug/examples/sql-session-verify /tmp/documents.json /tmp/sql-documents
```

`--projection` adds projection-view coverage. `--scalar-consumers` selects the mixed set/scalar fallback case.
The report enforces the same complete-job allowance for every variant. Correct parity alone does not satisfy that allowance.

Verification retains the compiler's distinction between interned names, grammar kinds, and grammar fields.
Grammar identifiers remain stable integers across stores. They do not require symbol-arena entries in an empty snapshot.
Normalization failures identify the checkpoint, relation, column, domain, and raw value.

The corpus generator includes failure rollback, retry, unchanged publication, restoration, empty input, and recovery for both original CPython rule programs:

```sh
python3 benchmark/sql-executor.py --corpus-session-request workload /tmp/workload.json
python3 benchmark/sql-executor.py --corpus-session-request calls /tmp/calls.json
```

Native tape identities compare as nodes even when the compiler introduces the tape input implicitly.
The negative parent sentinel remains an integer.

`steps` supplies changes, optional limits, and an optional expected error class.
The generated request includes instruction-limit and wave-limit failures, followed by a successful retry and six edits.
The report records verification reads in their own phase. Those reads still count against the complete-job allowance.
The parity bundle pins the request, runner, observations, and SQL receipts through SHA-256 digests.

An existing bundle can reproduce acceptance with this command:

```sh
node scripts/profiling/sql-report.mjs --sql-session /tmp/sql-session-1000/raw /tmp/sql-session-report --parity /tmp/sql-session-1000/parity
```


`RERAIL_PROFILE=/tmp/logic-profile` also records compiler search, CPU spans, and the logical rule plans.
The SQL request tool writes the standard logic reports and places shared SQL traces in the `sql` subdirectory.
SQL EXPLAIN runs outside callbacks. Temporary old-input views receive plans before cleanup releases them.

Retained operator tables now keep the secondary indexes required by their checked queries.
Leading prefixes use the complete-row tree. Required secondary indexes use bulk construction during preparation.
Compound-key `IN` filters descend through existing prefixes without constructing the lists' Cartesian product.
SQLite rechecks tuple correlation, affinity, and remaining predicates.
Frozen cursors and transaction rollback preserve the previous rows and index configuration.
This reduces initial CPU and memory. Supported recursive programs use the single retained schedule described above.
