# Fact schema v1

Owner: S4. Status: **v1, interface frozen for S2 and S3** (additions only; any change to a defined relation bumps `SCHEMA_VERSION`). The Python-derived relations are **SQL views over the syntactic core**, compiled and memoized per content by the same compiler as any rule; the extractor produces only the core and the facts that need a parser (§ 1.3). Measured decisions (§ 5) and exactness evidence live in `docs/facts/reports/phase-2-s4-schema.md`.

Conventions (PHASE-1 § Contract, unchanged):

- One fact set per **content** (blake3-256 of the bytes, lowercase hex). Facts are path-independent; no relation reads the path.
- Node ids: `u32` preorder over every tree-sitter node of the native tape (named and anonymous), root = 0; descendants of `id` are exactly `id+1 .. subtree_end` (half-open). Byte offsets half-open `[start, end)`. Lines 1-based.
- Strings: process-interned `u32` symbols (`sym`). Node text is never stored: it is `source[start..end]`; in views, `source_text('', s, e)` is the content's slice `[s, e)` (the compiler's `text` built-in; the empty path means "this content").
- `opt_X`: `u32::MAX` is none (SQL `NULL`).
- `SCHEMA_VERSION` = **1** (`facts.rs`, `factfile.rs`).

## 1. Syntactic core (extracted; per content)

### 1.1 Stored columns (Spike A § 5.3, unchanged in meaning)

| relation | columns | notes |
|---|---|---|
| `node` | `id:u32` (row), `kind:sym`, `parent:opt_u32`, `start:u32`, `end:u32`, `start_line:u32`, `end_line:u32`, `subtree_end:u32`, `flags:u8`, `field:opt_sym` | `flags` = named \| extra<<1 \| missing<<2 \| error<<3; `field` = the grammar field the node hangs under |
| `identifier` | `id:u32`, `name:sym` | named `identifier` nodes and their text |
| `line` | `start:u32`, `end:u32` (before the terminator), `indent:u16` | the native `LineTable` (terminators `\r\n`, `\r`, `\n`) |
| `syntax_error` | 0 or 1 row | the tree has an error or missing node |

Derived on read, never stored: the children CSR and the kind postings (`facts::index`).

### 1.2 The core as SQL relations (what views read)

| relation | columns (SQL) | definition |
|---|---|---|
| `node` | `id, kind, parent, start, "end", start_line, end_line, subtree_end, named, flags, field` | one row per node, preorder; `kind`, `field` as text; `named` = `flags & 1`; `parent`, `field` `NULL` when none |
| `child` | `parent, idx, child` | every child, `idx` 0-based in order |
| `field` | `parent, field, child, rank` | every child that hangs under a field; `rank` 0-based among the parent's children under that field |
| `identifier` | `id, name` | as stored |
| `line` | `no, start, "end", indent` | `no` 1-based |

All columns are integers or text with no declared affinity. `descendants(x)` is the id range `x < d < subtree_end(x)`, never a relation.

### 1.3 Facts that need a parser (extracted, per content)

| relation | columns | definition | status |
|---|---|---|---|
| decoded string value | per `string`/`concatenated_string` node | Python literal decoding (`pyvalue.rs`, port of native `value/python.rs`) | on demand, not persisted |
| f-string parts | per string node | `fstring.rs` (port of native `literal/python_fstring.rs`) | on demand, not persisted |
| `string_type_definition` | `host:u32` (+ the parsed annotation tree) | the string literals Ruff 0.16.9 reads as type definitions (`string_type_definitions`), as native's scope table computes them (`scope/python/typedefs.rs`, `ScopeTable::annotations`), extracted by **calling** native's builder (visibility only). Per content: the scope table reads one file and never crosses files. | § 6 of the report |

## 2. Python-derived relations: views over the core

### 2.1 Rules

- Each view below is **normative SQL** (byte-for-byte the text Spike A's engine runs: `views.rs` reads this file's `-- view:` lines). Views are evaluated per content in document order; a view reads the core and the views above it. Ids are node ids (integers); no view copies node text except where a predicate needs it.
- `py_node` is the node set **`N`** = the named nodes whose kind is not one of `comment`, `string_start`, `string_content`, `string_end`, `escape_sequence`, `line_continuation`. This is exactly the row set of the `syntax` spec table of `quality-contracts-1/2/3` (`excludeKinds`) and of every dump-built `nodes` stage (`syntax_N`/`raw_nodes` arms unioned by `syntax_rows_k`) **with the clause's shared selection (gate) and ancestor predicate replaced by `true`**.
- Each view is defined to return exactly what the named policy stage text returns over a `nodes` (and `ownership`) built from all of `N`, after rendering (§ 2.3). The stage texts are byte-identical (after whitespace normalisation) across the clauses listed with each view.
- Row order: each view's own order (FROM-order nested loops over a preorder `py_node`). A clause whose `nodes` is in arm order sees the same rows ordered by its `nodes` order of the key node(s) (§ 2.3).

### 2.2 The views

"Stage" gives the policy stage the view stands for and the clauses that hold that exact text; "renders" gives how the stage's columns follow from the view's.

#### `py_node`

`N` with `ordinal` = 0-based position among `N`-siblings by `(start, end, kind)` (the `nodes` stage's `row_number()`). Stage: `nodes` (dump-built, gate `true`; syntax spec table). Renders: `id`/`parent`/`edge_parent` are `kind:start:end` texts in dump clauses, integers in syntax-spec clauses; `n.*` columns are the binder's column view of the node.

```sql
-- view: py_node
SELECT id, kind, parent, start, "end", row_number() OVER(PARTITION BY parent ORDER BY start,"end",kind)-1 AS ordinal FROM node WHERE named=1 AND kind NOT IN ('comment','string_start','string_content','string_end','escape_sequence','line_continuation')
```

#### `py_field`

the first child under each field, when it is named (the dumps' field captures; the syntax spec's `<f>_*` columns). Stage: `<f>_start`, `<f>_end`, `<f>_kind`, `<f>_source`, `<f>_id` columns of `nodes`. Renders: `<f>_source` = text of `child`.

```sql
-- view: py_field
SELECT f.parent, f.field, f.child FROM field f JOIN node c ON c.id=f.child WHERE f.rank=0 AND c.named=1
```

#### `py_owner`

per node of `N`: nearest ancestor-or-self `function_definition` (`fn`), `class_definition` (`cls`), outermost `function_definition` (`top_fn`). Stage: `ownership` (11 clauses; syntax spec `ownership` with owners `fn`, `cls`, `top_fn` outermost). Renders: ids rendered.

```sql
-- view: py_owner
WITH RECURSIVE o(id,fn,cls,top_fn) AS (SELECT id,CASE WHEN kind='function_definition' THEN id END,CASE WHEN kind='class_definition' THEN id END,CASE WHEN kind='function_definition' THEN id END FROM node WHERE parent IS NULL UNION ALL SELECT c.id,CASE WHEN c.kind='function_definition' THEN c.id ELSE o.fn END,CASE WHEN c.kind='class_definition' THEN c.id ELSE o.cls END,CASE WHEN o.top_fn IS NULL AND c.kind='function_definition' THEN c.id ELSE o.top_fn END FROM o JOIN node c ON c.parent=o.id) SELECT n.id,o.fn,o.cls,o.top_fn FROM py_node n JOIN o ON o.id=n.id
```

#### `py_function`

`function_definition` nodes; `name` = the `name` field child. Stage: `functions` = `SELECT n.*,n.name_source AS name FROM nodes n WHERE n.kind='function_definition'` (13 clauses: step-contracts-1..6, state-signatures-1, state-flow-1, state-flow-project-1, state-projection-recovery-2, executor-provenance, quality-conditional-1, legacy-execution-graphs). Renders: `n.*` of `id`; `name` = text.

```sql
-- view: py_function
SELECT n.id, nm.child AS name FROM py_node n LEFT JOIN py_field nm ON nm.parent=n.id AND nm.field='name' WHERE n.kind='function_definition'
```

#### `py_class`

`class_definition` nodes; `name`; `outer` = the nearest enclosing class (`owner(parent).cls`). Stage: `classes` (7 clauses: step-contracts-1..4, state-signatures-1, quality-conditional-1, legacy-execution-graphs). Renders: `n.*` of `id`; `name` = text; `outer` feeds `py_context`.

```sql
-- view: py_class
SELECT n.id, nm.child AS name, po.cls AS outer FROM py_node n LEFT JOIN py_field nm ON nm.parent=n.id AND nm.field='name' LEFT JOIN py_owner po ON po.id=n.parent WHERE n.kind='class_definition'
```

#### `py_method`

functions whose parent, or whose `decorated_definition` parent's parent, is a class's `body`. Stage: `methods` (step-contracts-1..3, state-signatures-1). Renders: `f.*` = `functions` row of `fn`; `cls`; `class_name` = text of `py_class.name`.

```sql
-- view: py_method
SELECT f.id AS fn, c.id AS cls FROM py_function f JOIN py_node fn ON fn.id=f.id LEFT JOIN py_node d ON d.id=fn.parent AND d.kind='decorated_definition' JOIN py_field b ON b.field='body' AND b.child=coalesce(d.parent,fn.parent) JOIN py_node c ON c.id=b.parent AND c.kind='class_definition'
```

#### `py_attribute`

`attribute` nodes with their `attribute` and `object` field children. Stage: `attributes` (12 clauses). Renders: `n.*`; `attr` = text; `object` = id; `object_text` = text.

```sql
-- view: py_attribute
SELECT n.id, a.child AS attr, o.child AS object FROM py_node n LEFT JOIN py_field a ON a.parent=n.id AND a.field='attribute' LEFT JOIN py_field o ON o.parent=n.id AND o.field='object' WHERE n.kind='attribute'
```

#### `py_call`

`call` nodes with a `function` field child; `callee` = the function's `attribute` child when it is an attribute that has one, else the function; `receiver` = its `object`. Stage: `calls` (11 clauses). Renders: `n.*`; `callee_id` = id of `function`, `callee_kind`, `full_callee` = its text; `callee` = text; `receiver` = id.

```sql
-- view: py_call
SELECT n.id, f.id AS function, coalesce(a.attr,f.id) AS callee, a.object AS receiver FROM py_node n JOIN py_field fc ON fc.parent=n.id AND fc.field='function' JOIN py_node f ON f.id=fc.child LEFT JOIN py_attribute a ON a.id=f.id WHERE n.kind='call'
```

#### `py_parameter`

per function: the `identifier`/`typed_parameter`/`default_parameter`/`typed_default_parameter` children of its `parameters`, with a name; `position` among them by start. Stage: `parameters` (7 clauses: step-contracts-2,3,5, state-signatures-1, state-flow-1, state-flow-project-1, quality-conditional-1). Renders: `__path`, `fn`, `id`, `kind`, `text` of `id`, `name` = text, `annotation` = id, `position`.

```sql
-- view: py_parameter
SELECT f.id AS fn, p.id, nm.id AS name, t.child AS annotation, row_number() OVER(PARTITION BY f.id ORDER BY p.start)-1 AS position FROM py_function f JOIN py_field ps ON ps.parent=f.id AND ps.field='parameters' JOIN py_node p ON p.parent=ps.child JOIN py_node nm ON (nm.id=p.id AND p.kind='identifier' OR nm.parent=p.id AND nm.ordinal=0 AND nm.kind='identifier') LEFT JOIN py_field t ON t.parent=p.id AND t.field='type' WHERE p.kind IN ('identifier','typed_parameter','default_parameter','typed_default_parameter')
```

#### `py_assignment`

`assignment` nodes with a `left` child in `N`; `value` = `right` child; `fn` = owner. Stage: `assignments` (8 clauses: step-contracts-1,2,4,6, state-signatures-1, state-flow-1, state-projection-recovery-2, state-flow-project-1). Renders: `n.*`; `target`, `target_kind`, `target_text`; `value`, `expr_kind`, `expr_text`; `fn`.

```sql
-- view: py_assignment
SELECT n.id, l.id AS target, r.id AS value, o.fn FROM py_node n JOIN py_field lf ON lf.parent=n.id AND lf.field='left' JOIN py_node l ON l.id=lf.child LEFT JOIN py_field rf ON rf.parent=n.id AND rf.field='right' LEFT JOIN py_node r ON r.id=rf.child LEFT JOIN py_owner o ON o.id=n.id WHERE n.kind='assignment'
```

#### `py_decorator`

per decorated function: each child of each `decorator`; `name` = the child's call callee, else its attribute's `attr`, else the child. Stage: `decorators` (legacy-execution-graphs). Renders: `__path`, `fn`, `id`, `name` = text.

```sql
-- view: py_decorator
SELECT f.id AS fn, d.id, child.id AS child, coalesce(c.callee,a.attr,child.id) AS name FROM py_function f JOIN py_node fn ON fn.id=f.id JOIN py_node w ON w.id=fn.parent AND w.kind='decorated_definition' JOIN py_node d ON d.parent=w.id AND d.kind='decorator' JOIN py_node child ON child.parent=d.id LEFT JOIN py_call c ON c.id=child.id LEFT JOIN py_attribute a ON a.id=child.id
```

#### `py_import_identifier`

identifiers whose start lies in `[start, end]` of an `import_statement`/`import_from_statement` and whose end is at most its end; one row per (import, identifier). Stage: `import_identifiers` (quality-contracts-1, -2; `import_ranges` inlined). Renders: `__path`, `id`.

```sql
-- view: py_import_identifier
SELECT n.id, imp.id AS import FROM py_node imp JOIN py_node n ON n.kind='identifier' AND n.start>=imp.start AND n.start<=imp."end" WHERE imp.kind IN ('import_statement','import_from_statement') AND n."end"<=imp."end"
```

#### `py_context`

state-flow's function contexts: top-level functions (`top_fn = fn`), their dotted class chain and five predicates in SQLite three-valued logic; path-free. Stage: `contexts` (state-flow-1, state-flow-project-1). Renders: `f.*` = `functions` row; `class_name`; `planning`, `preprocess`, `ctx_helper`, `materializer` as is; `provider_helper` = `__path='impl/common/l2070_server_plan/backend_profiles.py' AND provider_helper_name` (the path conjunct is the binder's: facts are path-free).

```sql
-- view: py_context
WITH RECURSIVE cn(id,name) AS (SELECT c.id,i.name FROM py_class c LEFT JOIN identifier i ON i.id=c.name WHERE c.outer IS NULL UNION ALL SELECT c.id,p.name||'.'||i.name FROM cn p JOIN py_class c ON c.outer=p.id LEFT JOIN identifier i ON i.id=c.name) SELECT f.id AS fn, coalesce(q.name,'') AS class_name, rx_matches(fname.name,'_lazy$') OR fname.name IN ('compile','lower','post_op','pre_op','preprocess') OR fname.name='pending_pipeline_effects' OR rx_matches(fname.name,'^apply_.*_effect$') OR EXISTS(SELECT 1 FROM py_node w WHERE w.id=fn.parent AND w.kind='decorated_definition' AND rx_matches(source_text('',w.start,w."end"),'(?m)^\s*@(?:[\w.]*\.)?pure_lazy\b')) AS planning, fname.name='preprocess' AS preprocess, substr(fname.name,-11)='sql_builder' AND fname.name NOT IN ('compile','lower','post_op','pre_op','preprocess') AND EXISTS(SELECT 1 FROM py_parameter p JOIN py_node ann ON ann.id=p.annotation WHERE p.fn=f.id AND rx_matches(source_text('',ann.start,ann."end"),'\bCompilationContext\b')) AS ctx_helper, rx_matches(fname.name,'component|components|score|scores') AS provider_helper_name, EXISTS(SELECT 1 FROM py_parameter p JOIN identifier pn ON pn.id=p.name WHERE p.fn=f.id AND pn.name='runtime_refs') AND (rx_matches(fname.name,'^materialize_.*builder$') OR EXISTS(SELECT 1 FROM py_parameter p JOIN py_node ann ON ann.id=p.annotation WHERE p.fn=f.id AND rx_matches(source_text('',ann.start,ann."end"),'\bRuntimeBuilderSpec\b'))) AS materializer FROM py_function f JOIN py_node fn ON fn.id=f.id JOIN py_owner own ON own.id=f.id AND own.top_fn=f.id LEFT JOIN identifier fname ON fname.id=f.name LEFT JOIN cn q ON q.id=own.cls
```

### 2.3 Binding a stage to a view (for S2)

A view row's **support** is the set of nodes the stage row needs in the clause's `nodes`: the ids in its key and id columns, plus, for `fn`/`cls`/`top_fn`/`outer`/`class_name`, the ancestor chain up to the owner, and for `ordinal`/`position`, the siblings. Over a clause selection `S`, the stage over `nodes(S)` returns exactly the view's rows whose support is ⊆ `S` when `S` is closed under descendants (the `matches X or inside X` gates); otherwise `ordinal`, `position`, `owner` and `class_name` must be recomputed over `S`. Row order is the clause's `nodes` order of the key node (arm order, preorder within an arm), then the view's inner order. S2 owns the binding and re-verifies it with `FACTS_A3_CHECK`.

### 2.4 Not views over the core: `special_attributes`, `return_kinds` (and `direct_events`)

Their shared texts (state-flow-1, state-flow-project-1) read `string_values`, i.e. the clause's `raw_strings_*` **matcher inputs** (decoded strings under the clause's gate), and join rows **by name across the node set** (`runtime(__path,name)`: `r.name=call.callee`; `string_values`: `n.text=a.target_text`). A row then depends on which *other* nodes the clause selected, so no per-row filter of an all-nodes relation reproduces the stage over a gated selection: `Unsupported: name-keyed join across the clause's node selection`. Defining them over all of `N` is possible (the stage chain over these views plus decoded strings) but no clause could bind it; they stay rule-private logic that S2 compiles per clause (cross-clause sharing applies only when two clauses' selections are equal).

## 3. Exported relations (per content; Spike B § 5.1, v1)

Unchanged from Spike B v0 (`spikes/facts-b/src/facts.rs`): `def(name:sym, kind:sym, qualname:sym, line:u32)`, `ref(name:sym)` distinct, `import(module:sym, name:opt_sym, local:sym, top:u8, module_scope:u8)`, `call_edge(caller:sym, callee:sym)` distinct, `def_decorator(qualname:sym, name:sym)`, `dunder_all(literal:u8)`, `dunder_all_name(name:sym)`, `seed(qualname:sym, family:sym)`, `syntax_error()`. Definitions in Spike B § 5.1. Extended by S3's requests (repository plans' leaves), recorded here as they land.

## 4. Exactness (the differential)

`facts-a derived-check` evaluates the views per corpus file and, for every clause that holds one of the stage texts of § 2.2, runs the reference: the clause's own `nodes` (dump inputs with the gate and ancestor predicate forced `true`, or its syntax-spec tables) and its stages up to the target, by the SQL engine. Each target stage's rows are compared with the view's rows rendered into the stage's columns (§ 2.2 "renders"): values, types, and order after the § 2.3 reordering. Results and per-file cost: report § 2.

## 5. Fact files v1

Layout as v0 (`RRFACTS\0`, `u32` version, 32-byte hash, strings, relations, typed columns, 4-byte padding after `u16`/`u8` columns), version **1**. The anonymous-node and compression decisions are recorded in the report with measurements; readers accept versions 0 and 1 (`facts-a inspect`).
