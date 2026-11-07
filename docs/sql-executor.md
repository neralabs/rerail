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

The retained path supports scalar `count`, `sum`, `min`, and `max` over one relation, including global aggregates and empty groups.
Correlated subqueries require equality between inner and outer expressions. Constant filters and repeated inner variables also work.
Positive rule bodies can join several relations. Reducers can read base inputs or completed derived relations.

Computed equality correlations use retained group keys and expression indexes over their outer contexts.
Nonrecursive lattice outputs retain candidate multiplicities and fold only changed keys.
This includes flat, min, max, and registered deterministic joins. Retractions can restore a concrete value from top.
Min/max folds use indexed extrema. A registered join can scan all candidates at an affected key.

Nonrecursive rules with generated tree walks retain their seed, walk, and result relations.
Their fact changes use overdelete/rederive through indexed SQL relations. AST matching remains native.
Walks with scalar subqueries or negation inside their generated blocks use the refresh fallback.
Recursive lattice rules, recursive aggregate rules, and unsupported subqueries also refresh their affected strata.
Other unsupported cases include string aggregates and correlations that depend on another aggregate result.
A relation with a constant fact and a nonmonotone rule also uses this fallback.
The compiler selects eligibility from the checked query tree. It does not parse rendered SQL.

Each program report includes `outputs`, `positive_waves`, `overdelete_waves`, `refreshed_strata`, and `migrated_views`.
`incremental_strata` lists relations maintained through retained reducers or candidate lattice folds.
Positive tree walks use `positive_waves` and `overdelete_waves`.
`reducer_groups` counts processed reducer groups and lattice keys. `context_rows` counts rule contexts reconsidered by this path.
The batch report includes `inserted_facts`, `deleted_facts`, `migrated_inputs`, and `programs`.
`initializations` lists `{program, relations}` entries for newly initialized reducer or walk state, including conservative downstream initialization.
A no-op batch returns no program reports.

The first change copies each changed binding into an indexed mutable fact source.
Projection views in potentially affected programs become retained tables so the engine can compare their previous values.
The first change that can affect a relation initializes its eligible reducer groups, rule contexts, and output support against the old facts.
Unrelated aggregates keep their previous results without building support state.
These one-time migrations cost time and memory proportional to inputs, distinct values, and positive body combinations.
A large join can require many retained contexts. Min/max state and related reducer expressions currently keep separate value indexes.
Later updates operate on changed candidates, affected contexts, and retained indexes.
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
First-edit reducer initialization and projection migration can still read all participating facts.
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

After every run, removal and reset, the session rebuilds a catalog of its retained programs as temporary tables:
- `rr_relation`, `rr_column`, `rr_rule`, `rr_atom` and `rr_plan` (`sql_executor/catalog.rs`);
- for each relation, the typed view `rr_rel_<name>`, whose columns are the `.decl`'s names.

For example, `SELECT src, dst FROM rr_rel_reach` reads `reach`, whose own columns are `c0, c1`. The `rr_` prefix is reserved.
Design: `docs/sql-access/01-design.md`.

## Execution boundary

The planner emits seed statements and recursive delta statements.
SQL selects each wave's pending frontier from `wave_step`, `wave_read`, `wave_after`, and `wave_pending`.
The scheduler folds new rows into completed relations and records their deltas in `wave_delta`.
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
