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
The extension session uses the SQL wave scheduler and exposes its intermediate relations for queries.

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
| `bindStored(scope, paths?)` | Binds current core documents from the content store without reading source files. |
| `bindRelation(relationJson)` | Adds or replaces an integer fact relation. Duplicate rows collapse. |
| `lower(program, dialect)` | Returns the setup SQL and wave program without executing its rules. |
| `run(program, dialect, maxWaves?)` | Runs an inline `.rr` program through the SQL wave scheduler. |
| `runFile(path, dialect, maxWaves?)` | Reads a `.rr` file, its includes, and `core.rr`, then runs it. |
| `query(sql, parametersJson?, maxRows?)` | Returns columns, row arrays, and a `truncated` flag. |

An inline program declares its inputs. `runFile` uses the normal file reader and its core prelude.
The dialect controls the kind and field constants that the planner lowers to integer atoms.
It does not select a SQL dialect: execution uses SQLite.

Results remain available for later SQL queries and rule programs on the same session.
A later program can declare a previous output as an input.
The `.output` directive selects the derived relations to compute. Their intermediate relations also remain available for queries.
Output names must not overlap existing SQL objects or bound inputs.
Rebinding any input clears derived results, so consumers cannot read stale rows.
Rebinding a relation preserves its column names and arity.

A failed run rolls back its SQL writes. It does not publish partial derived results.
The default wave limit is 1024.
Queries accept positional JSON scalar parameters and return at most 10,000 rows by default.
Queries require one read-only statement with result columns.
Blob values return as `{"blob": [byte, ...]}`.

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
These relations read native columns through virtual tables. The executor does not copy nodes into SQL tables.
Incomplete parses fail the binding instead of supplying partial facts.

Symbols are integer atoms. `sym_text(atom)` reads their text, and `sym_atom(text)` interns text.
The connection also registers native lattice and text functions, including Unicode `lower`, `sha256`, and `rr_join`.
Kind and field atoms have separate domains. The rule planner lowers their string constants using the selected dialect.

Arbitrary SQL can join these virtual tables, persistent store tables, and completed derived relations.
For example, a query can count names per document or compare findings from two derived relations.
Additional producers can bind integer facts without changing the SQL compiler.
Rust extensions can register SQL functions or virtual tables through `session.connection()`.
The connection belongs to the session, so Rust statements and cursors borrow that session.

## Execution boundary

The planner emits seed statements and recursive delta statements.
SQL selects each wave's pending frontier from `wave_step`, `wave_read`, `wave_after`, and `wave_pending`.
The scheduler folds new rows into completed relations and records their deltas in `wave_delta`.
Negation and aggregation wait for their input strata to finish.
Derived columns use positional names: `c0`, `c1`, and so on.

The extension API supports set relations and flat lattices.
Other lattice joins require a native producer and receive an explicit refusal.
Zero-column wave relations also receive an explicit refusal.
An AST matcher atom must become native match facts before the rule enters this executor.
The standalone session does not publish derivation records or replace the product audit path.
The product path continues to use the same SQL scheduler with its provenance and incremental storage.
