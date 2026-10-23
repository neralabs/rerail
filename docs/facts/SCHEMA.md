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
| `string_value` | `id, value` | parser fact (§ 1.3): for each node of kind `string` or `concatenated_string` whose `pyvalue::decode` is a `str`, its decoded value (text) |
| `string_value_unknown` | `id, named_escape` | parser fact (§ 1.3): each node of kind `string` or `concatenated_string` whose `pyvalue::decode` is unknown; `named_escape` = 1 when decoding met a `\N{...}` escape (no Unicode name table: the leaf executor refuses the clause, I9), else 0 |

All columns are integers or text with no declared affinity. `descendants(x)` is the id range `x < d < subtree_end(x)`, never a relation.

### 1.2.1 Declared structure (INVARIANTS I2)

Rewrites may rely on exactly these; each is checked per content in debug builds (`views.rs` structural checks, to land with the compiler's `Plan` metadata).

| relation | key | functional dependencies | order and intervals | cardinality |
|---|---|---|---|---|
| `node` | `id` (dense `0..n`) | `id → kind, parent, start, end, start_line, end_line, subtree_end, named, flags, field` | preorder: `id` ascending = document order; `start` non-decreasing in `id`; **interval**: `d` is a proper descendant of `x` iff `x < d < subtree_end(x)`; `parent(x) < x`; root `0` has `parent NULL` | `n` rows; `subtree_end ≤ n` |
| `child` | `child` | `child → parent, idx` (= `node.parent`); `(parent, idx) → child` | per parent, `idx` ascending = `child` ascending | `n − 1` rows |
| `field` | `child` | `child → parent, field, rank`; `(parent, field, rank) → child` | per `(parent, field)`, `rank` ascending = `child` ascending | ≤ `n − 1` |
| `identifier` | `id` (⊆ `node.id`, `kind = 'identifier'`, named) | `id → name` | `id` ascending | ≤ `n` |
| `line` | `no` (dense `1..L`) | `no → start, end, indent` | `start` strictly ascending in `no` | `L` = terminators + 1 |
| `string_value` | `id` (⊆ `node.id`, kind `string`/`concatenated_string`) | `id → value` | `id` ascending | ≤ string nodes; disjoint from `string_value_unknown` |
| `string_value_unknown` | `id` (same domain) | `id → named_escape` | `id` ascending | ≤ string nodes |
| `py_node` (`N`) | `id` | as `node`; `(parent, ordinal) → id` | `ordinal` = rank among `N`-siblings in `id` order | ≤ `n` |
| `py_field` | `(parent, field)` | `(parent, field) → child` | | ≤ `n − 1` |
| `py_owner`, `py_function`, `py_class`, `py_attribute`, `py_call`, `py_assignment` | `id` | `id → *` | rows in `id` order except `py_owner` (level order of its fixpoint) | ≤ `|N|` |
| `py_method` | `fn` | `fn → cls` | `fn` order | ≤ functions |
| `py_parameter` | `id` | `id → fn, name, annotation, position`; `(fn, position) → id` | `(fn, id)` order | |
| `py_decorator` | `child` | `child → fn, id, name` | `(fn, id, child)` order | |
| `py_import_identifier` | `(import, id)`; `id → import` (imports never nest) | | `(import, id)` order | |
| `py_context` | `fn` | `fn → *` | `fn` order | ≤ functions |
| `cg_scope` | `id` | `id → kind, name, body, body_end` | `id` order; bodies nest: two rows' `[body, body_end)` intervals are disjoint or nested | ≤ functions + classes |
| `cg_def` | `node` | `node → kind, local_name, parent_def, start, end` | `node` order; `parent_def < node` | = `cg_scope` |
| `cg_call` | `node` | `node → start, end, callee_text, caller` | `node` order; `caller < node` | ≤ `call` nodes |
| `cg_import` | none (a row per imported item; `node` is the statement) | `node → module_text, level, scope` | `(node, item)` order | ≤ import items |

### 1.3 Facts that need a parser (extracted, per content)

| relation | columns | definition | status |
|---|---|---|---|
| `string_value`, `string_value_unknown` (§ 1.2) | `id, value`; `id, named_escape` | Python literal decoding (`pyvalue.rs`, port of native `value/python.rs`) of every `string`/`concatenated_string` node: `Str` → `string_value`; unknown (interpolation, a str/bytes mix, over 64 KiB, an undecodable or `\N{...}` escape, a `t` prefix) → `string_value_unknown`; `Bytes` → neither (a known non-`str` value). These kinds never give a refusal at decode time. A pattern `value: {regex}` on such a node holds iff its `string_value` matches, is unknown iff it is in `string_value_unknown`, and is false otherwise; `DECODED` (`replace` with `value: true`) is its `string_value`, `''` when absent. Value tests on other kinds are not covered (the front end refuses them unless a preceding `kind` conjunct fixes the kind) | computed per content from `node` and the source (`views::core_tables`); not persisted (a fact file holds both inputs) |
| f-string parts | per string node | `fstring.rs` (port of native `literal/python_fstring.rs`) | on demand, not persisted |
| `string_type_definition` | `host` (the literal's `[start, end)`) → the parsed annotation tree: per node `kind:sym`, host `[start, end)` (native `Origin::of_node(..).primary()`), `text`, `parent` (index), `named` | the string literals Ruff 0.16.9 reads as type definitions (`string_type_definitions`, nested ones in their outer annotation's range space), as native's scope table computes them (`scope/python/typedefs.rs`, `ScopeTable::annotations`), extracted by **calling** native's `ScopeTable::build_in` (visibility-only changes in `native/`), default reading (no stub, no typing settings, `python_minor` 0), unbounded budget. **Per content**: the scope table reads one file and never crosses files. Built with no module path: native qualifies a `Relative`/`Definition` import binding through the module path, and such a binding can resolve to `typing`/`_typeshed`/`typing_extensions` only when the file's module path starts with one of those names (none in the corpus; a binder must refuse otherwise). Computed on first use per content (`Facts::typedefs`); consumed by `parse: {grammar: annotation}` (`ast.rs`). | report § 6 |

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

per node of `N`: nearest ancestor-or-self `function_definition` (`fn`), `class_definition` (`cls`), outermost `function_definition` (`top_fn`), following `N`-parents only: a node whose parent is not in `N` (an `escape_interpolation` under `string_content`) starts its own chain, as the stage's roots (`edge_parent` not in `nodes`) do. Stage: `ownership` (dump clauses with an `N` node set: state-signatures-1, state-flow-1, state-flow-project-1, state-projection-recovery-2, executor-provenance, quality-conditional-1, legacy-execution-graphs). The syntax spec's `ownership` table (quality-contracts-1/2/3) chains through every named node, excluded kinds included: it is `py_owner_named` restricted to `N`. Renders: ids rendered. Order: the fixpoint's level order (no stage specifies one).

```sql
-- view: py_owner
WITH RECURSIVE o(id,fn,cls,top_fn) AS (SELECT n.id,CASE WHEN n.kind='function_definition' THEN n.id END,CASE WHEN n.kind='class_definition' THEN n.id END,CASE WHEN n.kind='function_definition' THEN n.id END FROM py_node n LEFT JOIN py_node p ON p.id=n.parent WHERE p.id IS NULL UNION ALL SELECT c.id,CASE WHEN c.kind='function_definition' THEN c.id ELSE o.fn END,CASE WHEN c.kind='class_definition' THEN c.id ELSE o.cls END,CASE WHEN o.top_fn IS NULL AND c.kind='function_definition' THEN c.id ELSE o.top_fn END FROM o JOIN py_node c ON c.parent=o.id) SELECT id,fn,cls,top_fn FROM o
```

#### `py_named`, `py_owner_named`

The all-named node set and its ownership. `step-contracts-1..6` build `nodes` from one `raw_nodes` arm that does not exclude the six kinds: their gate-true `nodes` is every named node, and their `ownership` chains through `string_content`. Every other view is exact for them as well (§ 4), since its stage text filters kinds; `ownership` is `py_owner_named` there.

```sql
-- view: py_named
SELECT id, kind, parent, start, "end", row_number() OVER(PARTITION BY parent ORDER BY start,"end",kind)-1 AS ordinal FROM node WHERE named=1
```

```sql
-- view: py_owner_named
WITH RECURSIVE o(id,fn,cls,top_fn) AS (SELECT n.id,CASE WHEN n.kind='function_definition' THEN n.id END,CASE WHEN n.kind='class_definition' THEN n.id END,CASE WHEN n.kind='function_definition' THEN n.id END FROM py_named n LEFT JOIN py_named p ON p.id=n.parent WHERE p.id IS NULL UNION ALL SELECT c.id,CASE WHEN c.kind='function_definition' THEN c.id ELSE o.fn END,CASE WHEN c.kind='class_definition' THEN c.id ELSE o.cls END,CASE WHEN o.top_fn IS NULL AND c.kind='function_definition' THEN c.id ELSE o.top_fn END FROM o JOIN py_named c ON c.parent=o.id) SELECT id,fn,cls,top_fn FROM o
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

### 2.4 `special_attributes`, `return_kinds` (and `direct_events`): views composed with the clause's matcher inputs

Their shared texts (state-flow-1, state-flow-project-1) are rule logic over the views above plus three rule-specific **matcher inputs**: `string_values` reads `raw_strings_0..2`, each a pattern query (`string`/`concatenated_string` without interpolation, a prefix regex, and a `value: {regex: <rule's name list>}` over the decoded literal, projected with `transform: DECODED`). Over all of `N`, each is a view: the stage chain `arguments`, `keywords`, `contexts` (= `py_context` rendered), `body_nodes`, `callback_parameters`, `parameter_kinds`, `string_values`, `special_attributes`, `return_kinds` as written, over the views of § 2.2 rendered into the dump vocabulary (§ 2.2 "renders") and the three inputs compiled as algebra over `py_node` and the core parser relation `string_value(id, value)` (§ 1.3, decoded by `pyvalue.rs`). The inputs' compilation is S1's pattern front end (INVARIANTS I3: matcher inputs are algebra over `node`/`child`/`field`); this schema declares the leaf they need (`string_value`) and the composition, and adds no hand translation of a matcher input. Binding note for the rewrite (S1/S2): `runtime(__path,name)` joins `r.name=call.callee` and `string_values` joins `n.text=a.target_text`, i.e. **by name across the node set**, so a row depends on other nodes' membership in the clause's selection; selection sharing by semijoin does not apply to these two stages even under subtree closure, and they run over the clause's subset exactly.

`string_value(id, value)` and `string_value_unknown(id, named_escape)` are core relations (§ 1.2, § 1.3), the leaf the three inputs read for `value: {regex}` and `DECODED`.

## 3. Exported relations (per content; Spike B § 5.1, v1)

Schema-level exports from Spike B v0 (`spikes/facts-b/src/facts.rs`), unchanged: `def(name:sym, kind:sym, qualname:sym, line:u32)`, `ref(name:sym)` distinct, `import(module:sym, name:opt_sym, local:sym, top:u8, module_scope:u8)`, `call_edge(caller:sym, callee:sym)` distinct, `def_decorator(qualname:sym, name:sym)`, `dunder_all(literal:u8)`, `dunder_all_name(name:sym)`, `seed(qualname:sym, family:sym)`, `syntax_error()`. Definitions in Spike B § 5.1.

### 3.1 Requested by S3 (repository plans' leaves), per content, schema-level

Superseded by § 3.3 (decider, wave 2). The wave-1 plan was to export native's per-file `Summary` (`calls/summary.rs`) by calling it. Its names are qualified by the module path, which depends on the worktree's `__init__.py` set, so its rows are keyed by (content, module path) and change when membership changes. The exports are therefore **unresolved and content-keyed** (§ 3.3), and module names, qualification and linking are membership-dependent algebra in S3's dataflow.

### 3.2 Plan-level exports (computed by a per-content plan, projected and distinct)

Namespace `x.<rule>.<clause>.<relation>`, stored under the key `(content hash, path, schema version, plan digest)` (INVARIANTS I7; the path is in the key because module names derive from it). S3's list (its report § 5.1) is the interim definition until the compiler emits them from the repository plans' leaves: `quality-project-taxonomy` `cand`; `quality-conditional-1` `class_base`, `canon_sym`, `gated`, `canon_cand`, `ungated`; `required-contracts` `contract_finding`; `state-flow-project-1`/`state-flow-1` `ret_kinds`, `callee`, `project_mode`, `sem_builders`, `helper_sites`; `public-api-triples` `decl_root`, `alias_edge`, `export_cand`, `class_decl`, `base_ref`, `pub_method`, `symbol_traits`, `symbol_family_traits`; `internal-dead-code` `decl`, `base_ref`, `import_sym`, `import_module_prefix`, `export_cand`, `dyn_target`, `annotation_name`, `module_name_use`, `sibling_edge`, `module_init_edge`, `class_member`.

### 3.3 Unresolved call-graph exports (content-keyed; agreed with S3)

These are views over the core, per content, with no module path and no resolution. Ids are this content's node ids, and texts are source slices. S3's dataflow joins them with membership-side relations, so that names resolve against the worktree:

- `module_of(path)` and `package_marker(path)` = the member paths whose file stem is `__init__` (`.py`/`.pyi`), as native's `ModulePath` reads them.
- `package_marker` is a property of the **path**, not of the content: it is computed in the membership layer from the path set and is never a fact (facts are path-free).

| view | columns | definition | differential |
|---|---|---|---|
| `cg_scope` | `id, kind, name, body, body_end` | every `function_definition`/`class_definition` with a `name` field child (rank 0). `body`/`body_end` are its `body` child's id interval (`NULL` without one). Native's walk (`summary.rs::walk`) makes a definition only from such a node and runs only the body in the definition's context: decorators, defaults and bases run in the outer one. | through `cg_def`, `cg_call` |
| `cg_def` | `node, kind, local_name, parent_def, start, end` | per `cg_scope` row:<br>• `kind` is `'class'`, `'async_function'` (first child `async`) or `'function'`;<br>• `parent_def` is the nearest `cg_scope` row whose body interval holds the node;<br>• `local_name` is native's qualified name without the module prefix, built as `summary.rs::definition` builds it: the parent's `local_name`, then `.` under a class or `.<locals>.` under a function, then the name. | native `Summary::extract(.., module None)`: `defs` (`name`, `start`, `end`, `CLASS`/`ASYNC` flags) |
| `cg_call` | `node, start, end, callee_text, caller` | every `call` with a `function` field child:<br>• `callee_text` is the source text of that child;<br>• `caller` is the nearest `cg_scope` **function** whose body interval holds the call. This is native's `owner`: a class body keeps the outer function. `NULL` at module level. | independent walk over the facts with native's context rule |
| `cg_import` | `node, module_text, name, alias, level, scope` | one row per imported item of an `import_statement`, `import_from_statement` or `future_import_statement` (the children under field `name`, plus `wildcard_import`):<br>• `import a.b as c` gives `('a.b', NULL, 'c', 0)`;<br>• `from ..p.q import x as y` gives `('p.q', 'x', 'y', 2)`, and `module_text` is `''` for `from . import x`;<br>• a wildcard has `name = '*'`;<br>• `from __future__ import x` gives `('__future__', 'x', NULL, 0)`;<br>• `level` counts the dots of `import_prefix`;<br>• `scope` is the nearest `cg_scope` (function or class) whose body interval holds the statement, `NULL` at module level. | native `ScopeTable` import bindings (`import`, `import_kind`, `submodule`, `name` range) |

```sql
-- view: cg_scope
SELECT n.id, n.kind, nf.child AS name, ab.child AS body, b.subtree_end AS body_end FROM node n JOIN field nf ON nf.parent=n.id AND nf.field='name' AND nf.rank=0 LEFT JOIN field ab ON ab.parent=n.id AND ab.field='body' AND ab.rank=0 LEFT JOIN node b ON b.id=ab.child WHERE n.kind IN ('function_definition','class_definition')
```

```sql
-- view: cg_def
WITH RECURSIVE s(id,kind,name,parent_def) AS (SELECT c.id,c.kind,c.name,(SELECT max(a.id) FROM cg_scope a WHERE a.body<c.id AND c.id<a.body_end) FROM cg_scope c), q(id,local_name) AS (SELECT s.id,source_text('',m.start,m."end") FROM s JOIN node m ON m.id=s.name WHERE s.parent_def IS NULL UNION ALL SELECT c.id,q.local_name||CASE WHEN p.kind='class_definition' THEN '.' ELSE '.<locals>.' END||source_text('',m.start,m."end") FROM q JOIN s p ON p.id=q.id JOIN s c ON c.parent_def=q.id JOIN node m ON m.id=c.name) SELECT s.id AS node,CASE WHEN s.kind='class_definition' THEN 'class' WHEN k.kind='async' THEN 'async_function' ELSE 'function' END AS kind,q.local_name,s.parent_def,n.start,n."end" FROM s JOIN q ON q.id=s.id JOIN node n ON n.id=s.id LEFT JOIN child c0 ON c0.parent=s.id AND c0.idx=0 LEFT JOIN node k ON k.id=c0.child
```

```sql
-- view: cg_call
SELECT n.id AS node,n.start,n."end",source_text('',f.start,f."end") AS callee_text,(SELECT max(a.id) FROM cg_scope a WHERE a.kind='function_definition' AND a.body<n.id AND n.id<a.body_end) AS caller FROM node n JOIN field fc ON fc.parent=n.id AND fc.field='function' AND fc.rank=0 JOIN node f ON f.id=fc.child WHERE n.kind='call'
```

```sql
-- view: cg_import
SELECT s.id AS node,CASE WHEN s.kind='import_statement' THEN source_text('',nm.start,nm."end") WHEN s.kind='future_import_statement' THEN '__future__' WHEN mm.kind='relative_import' THEN coalesce(source_text('',rd.start,rd."end"),'') ELSE source_text('',mm.start,mm."end") END AS module_text,CASE WHEN s.kind='import_statement' THEN NULL WHEN it.kind='wildcard_import' THEN '*' ELSE source_text('',nm.start,nm."end") END AS name,source_text('',al.start,al."end") AS alias,CASE WHEN mm.kind='relative_import' THEN length(source_text('',px.start,px."end"))-length(replace(source_text('',px.start,px."end"),'.','')) ELSE 0 END AS level,(SELECT max(a.id) FROM cg_scope a WHERE a.body<s.id AND s.id<a.body_end) AS scope FROM node s JOIN node it ON it.parent=s.id AND (it.field='name' OR it.kind='wildcard_import') LEFT JOIN field itn ON itn.parent=it.id AND itn.field='name' AND itn.rank=0 AND it.kind='aliased_import' LEFT JOIN node nm ON nm.id=coalesce(itn.child,it.id) LEFT JOIN field ita ON ita.parent=it.id AND ita.field='alias' AND ita.rank=0 AND it.kind='aliased_import' LEFT JOIN node al ON al.id=ita.child LEFT JOIN field mf ON mf.parent=s.id AND mf.field='module_name' AND mf.rank=0 LEFT JOIN node mm ON mm.id=mf.child LEFT JOIN node px ON px.parent=mm.id AND px.kind='import_prefix' AND mm.kind='relative_import' LEFT JOIN node rd ON rd.parent=mm.id AND rd.kind='dotted_name' AND mm.kind='relative_import' WHERE s.kind IN ('import_statement','import_from_statement','future_import_statement')
```

Texts are source slices. Native normalises names: `python_identifier` applies NFKC, and `scope/python.rs::dotted` joins a dotted name's identifiers with `.`.

- The views equal native's names exactly when a content has no non-ASCII identifier and no dotted name with whitespace or a comment inside.
- `cg_refusal` lists the nodes that break that condition. A consumer of § 3.3 **refuses the content** when it has a row, naming `construct` (I9); it never uses raw text in their place.
- On the corpus it has 0 rows: there are no non-ASCII identifiers (report § W2).

```sql
-- view: cg_refusal
SELECT i.id AS node,'non-ASCII identifier (native NFKC-normalises names)' AS construct FROM identifier i WHERE rx_matches(i.name,'[^\x00-\x7F]') UNION ALL SELECT d.id,'dotted name with whitespace or a comment (native joins its identifiers)' FROM node d WHERE d.kind='dotted_name' AND d."end"-d.start<>(SELECT sum(x."end"-x.start)+count(*)-1 FROM child c JOIN node x ON x.id=c.child WHERE c.parent=d.id AND x.kind='identifier')
```

The views resolve no binding: `self.m()`, aliases, star imports and bases are left to S3's linking rules. S3 will list any further leaf it needs (name references with native's name forms, bases, decorators), and each will be added here as a view with its differential.

## 4. Exactness (the differential)

`facts-a derived-check` evaluates the views per corpus file and, for every clause that holds one of the stage texts of § 2.2, runs the reference: the clause's own `nodes` (dump inputs with the gate and ancestor predicate forced `true`, or its syntax-spec tables) and its stages up to the target, by the SQL engine. Each target stage's rows are compared with the view's rows rendered into the stage's columns (§ 2.2 "renders"): values, types, and order after the § 2.3 reordering. Results and per-file cost: report § 2.

## 5. Fact files v1

`<store>/<hash[0..2]>/<hash>.facts`, little-endian (`factfile.rs`):

- Header: magic `RRFACTS\0`, `u32` schema version (**1**), 32-byte blake3 content hash, `u32` flags (bit 0: zstd body; other bits must be 0). With bit 0: `u32` raw body length, then one zstd frame (level 3). Version 0 files (no flags word, raw body) stay readable.
- Body (as v0): `u32` string count, strings (`u32` length, UTF-8), `u32` relation count; per relation `u32` name index, `u32` rows, `u32` columns, per column (`u32` name index, `u8` type: 0 `u32`, 1 `u16`, 2 `u8`, 3 `sym`, 4 `opt_u32`, 5 `opt_sym`), then each column at native width; after a `u16`/`u8` column, zero bytes to a 4-byte offset from the body start (a 4-byte file offset in both versions' raw images).
- Relations: `node` (row = node id; **every node, named and anonymous**): `kind:u16` (string index), `start:u32`, `end:u32`, `subtree_end:u32`, `flags:u8`, `field:u8` (string index + 1, 0 none; field names are the first strings); `identifier`: `id:u32`, `name:sym`; `syntax_error` (0 or 1 row). Derived on read: `parent` (innermost open subtree in preorder), `start_line`/`end_line` and `line` (from the source's `LineTable`), the children CSR and kind postings.
- Decisions, measured (report § 5.2): anonymous nodes are **stored** (dropping them saves 5.3% after compression and makes every read a re-parse, p50 0.70 ms instead of 0.08 ms; `field` ranks need them); bodies are **zstd level 3** (186.4 MB for the corpus, 1.35× source, against v0's 1,048 MB).
- Readers: `facts-a inspect` reads v0 and v1 `.facts` and Spike B's v0 `.xfacts`; `.xfacts` stay v0 until S3 moves the exported relations to v1.
