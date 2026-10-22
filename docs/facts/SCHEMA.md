# Fact schema v1

Owner: S4. Status: **v1, interface frozen for S2 and S3** (additions only; any change to a defined relation bumps `SCHEMA_VERSION`). Measured decisions (§ 5) and exactness evidence live in `docs/facts/reports/phase-2-s4-schema.md`.

Conventions (PHASE-1 § Contract, unchanged):

- One fact set per **content** (blake3-256 of the bytes, lowercase hex). Facts are path-independent; no relation reads the path.
- Node ids: `u32` preorder over every tree-sitter node of the native tape (named and anonymous), root = 0; descendants of `id` are exactly `id+1 .. subtree_end` (half-open). Byte offsets half-open `[start, end)`. Lines 1-based.
- Strings: process-interned `u32` symbols (`sym`). Node text is never stored: it is `source[start..end]`. Columns typed `text(node)` below mean "the source slice of that node", recovered on read.
- `opt_X`: `u32::MAX` is none (SQL `NULL`).
- `SCHEMA_VERSION` = **1** (`facts.rs`, `factfile.rs`).

## 1. Syntactic core (per content; Spike A § 5.3, unchanged in meaning)

| relation | columns | notes |
|---|---|---|
| `node` | `id:u32` (row), `kind:sym`, `parent:opt_u32`, `start:u32`, `end:u32`, `start_line:u32`, `end_line:u32`, `subtree_end:u32`, `flags:u8`, `field:u16` | `flags` = named \| extra<<1 \| missing<<2 \| error<<3; `field` = grammar field code under the parent (0 none) |
| `child` | CSR over `parent` (`child_offsets`, `children`) | derived on read |
| kind postings | `kind_keys`, `kind_offsets`, `kind_nodes` | derived on read |
| `line` | the native `LineTable` (terminators `\r\n`, `\r`, `\n`) | |
| `syntax_error` | 0 or 1 row | the tree has an error or missing node |
| decoded string value, f-string pieces | per node, on demand (`pyvalue.rs`, `fstring.rs`) | not persisted |

### 1.1 The node set `N` and the shared node vocabulary

Every derived relation below is defined over **`N`** = the named nodes whose kind is not one of `comment`, `string_start`, `string_content`, `string_end`, `escape_sequence`, `line_continuation`. This is exactly the row set of:

- the `syntax` spec table of `quality-contracts-1/2/3` (`excludeKinds`, rel.rs `SyntaxSpec::tables`), and
- every dump-built `nodes` stage (`syntax_N` / `raw_nodes` arms unioned by `syntax_rows_k`) **with the clause's shared selection (gate) and its ancestor predicate replaced by `true`**: the arms partition the named kinds and exclude the same six.

Within `N`, for a node `x`:

- `parent(x)`: the tree parent (always named). In dump-built `nodes` this is `parent` and, with the gate `true`, also `edge_parent`.
- `field(x, f)`: the first child of `x` hanging under grammar field `f`, if it is named (the dump's field captures and `SyntaxSpec`'s `<f>_*` columns; the fields used below are always named when present).
- `ordinal(x)`: the 0-based position of `x` among the children of `parent(x)` that are in `N`, ordered by `(start, end, kind)` (the `row_number()` of the `nodes` stage; for `N` it equals preorder among `N`-siblings).
- `text(x)`: `source[start(x)..end(x)]`.

Node identity in the policies' SQL is the text `kind:start:end` (dump convention) or the integer id (syntax-spec convention). The derived relations use `u32` ids; the binder renders either form. (The differential, § 4, also checks that no two nodes of `N` in the corpus share `kind:start:end`, so the two identities coincide.)

## 2. Python-derived relations (per content, over `N`)

Each relation is defined to return **exactly** what the named policy stage text returns when its `nodes` (and `ownership`) are built from all of `N` (§ 1.1). The stage texts are byte-identical after whitespace normalisation across the clauses listed; "stage" columns give the SQL column the fact column renders to. `n.*` (the clause's own `nodes` columns) is **not** stored: it is the column view of the key node, which S2 provides. Row order: the order the stage produces over a `nodes` table in preorder (the relation is stored in that order); a clause whose `nodes` is in arm order gets the same rows, reordered as its arm order dictates (§ 2.13).

### 2.1 `owner` (support for `assignment`, `context`; stage `ownership`, 11 clauses, and the syntax spec's `ownership`)

| column | type | stage column | definition |
|---|---|---|---|
| `id` | `u32` | `id` | every node of `N`, preorder |
| `fn` | `opt_u32` | `fn` | nearest ancestor-or-self of kind `function_definition` |
| `cls` | `opt_u32` | `cls` | nearest ancestor-or-self of kind `class_definition` |
| `top_fn` | `opt_u32` | `top_fn` | outermost ancestor-or-self of kind `function_definition` |

Stored as three columns indexed by node id (`u32::MAX` none), so `owner` is O(1) per node and costs no join.

### 2.2 `function` (stage `functions`: `SELECT n.*,n.name_source AS name FROM nodes n WHERE n.kind='function_definition'`; 13 clauses)

| column | type | stage column | definition |
|---|---|---|---|
| `id` | `u32` | `id` (+ `n.*`) | nodes of kind `function_definition`, preorder |
| `name` | `opt_u32` | `name` = `text(name)` | `field(id, name)` |

### 2.3 `class` (stage `classes`: `… WHERE n.kind='class_definition'`; 7 clauses)

| column | type | stage column | definition |
|---|---|---|---|
| `id` | `u32` | `id` (+ `n.*`) | nodes of kind `class_definition`, preorder |
| `name` | `opt_u32` | `name` = `text(name)` | `field(id, name)` |
| `outer` | `opt_u32` | (used by `context`) | `owner(parent(id)).cls`: the nearest enclosing class |

### 2.4 `method` (stage `methods`; 4 clauses)

`SELECT f.*,c.id AS cls,c.name AS class_name FROM functions f JOIN classes c ON c.__path=f.__path WHERE f.parent=<c.body> OR EXISTS(SELECT 1 FROM nodes d WHERE d.id=f.parent AND d.kind='decorated_definition' AND d.parent=<c.body>)`

| column | type | stage column | definition |
|---|---|---|---|
| `fn` | `u32` | `id` (+ `f.*`) | a `function` row |
| `cls` | `u32` | `cls`, `class_name` = `text(class.name)` | the `class` whose `field(cls, body)` is `parent(fn)`, or is `parent(parent(fn))` when `parent(fn)` is a `decorated_definition` |

Order: `fn` preorder (the outer loop); at most one `cls` per `fn`.

### 2.5 `attribute` (stage `attributes`; 12 clauses)

| column | type | stage column | definition |
|---|---|---|---|
| `id` | `u32` | `id` (+ `n.*`) | nodes of kind `attribute`, preorder |
| `attr` | `opt_u32` | `attr` = `text(attr)` | `field(id, attribute)` |
| `object` | `opt_u32` | `object` = its id, `object_text` = `text(object)` | `field(id, object)` |

### 2.6 `call` (stage `calls`; 11 clauses)

`SELECT n.*,f.id AS callee_id,f.kind AS callee_kind,f.text AS full_callee,coalesce(a.attr,f.text) AS callee,a.object AS receiver FROM nodes n JOIN nodes f ON f.id=<n.function> LEFT JOIN attributes a ON a.id=f.id WHERE n.kind='call'`

| column | type | stage column | definition |
|---|---|---|---|
| `id` | `u32` | `id` (+ `n.*`) | nodes of kind `call` with `field(id, function)` present, preorder |
| `function` | `u32` | `callee_id`, `callee_kind`, `full_callee` = `text(function)` | `field(id, function)` |
| `callee` | `u32` | `callee` = `text(callee)` | `attribute(function).attr` when `function` is an `attribute` with an `attribute` field, else `function` |
| `receiver` | `opt_u32` | `receiver` (an id) | `attribute(function).object` when `function` is an `attribute`, else none |

### 2.7 `parameter` (stage `parameters`; 7 clauses)

`SELECT f.__path,f.id AS fn,p.id,p.kind,p.text,n.text AS name,<p.type> AS annotation, row_number() OVER(PARTITION BY f.__path,f.id ORDER BY p.__start)-1 AS position FROM functions f JOIN nodes p ON p.parent=<f.parameters> JOIN nodes n ON (n.id=p.id AND p.kind='identifier' OR n.parent=p.id AND n.ordinal=0 AND n.kind='identifier') WHERE p.kind IN ('identifier','typed_parameter','default_parameter','typed_default_parameter')`

| column | type | stage column | definition |
|---|---|---|---|
| `fn` | `u32` | `fn` | a `function` row |
| `id` | `u32` | `id`, `kind`, `text` | a child `p ∈ N` of `field(fn, parameters)` of kind `identifier`, `typed_parameter`, `default_parameter` or `typed_default_parameter` |
| `name` | `u32` | `name` = `text(name)` | `p` itself when `p` is an `identifier`; else the child of `p` in `N` with `ordinal 0` when it is an `identifier`; rows without a name do not exist |
| `annotation` | `opt_u32` | `annotation` (an id) | `field(p, type)` |
| `position` | `u32` | `position` | 0-based rank of the row among its `fn`'s rows by `start(p)` |

Order: `fn` preorder, then `p` in `nodes` order (preorder over a preorder `nodes`).

### 2.8 `assignment` (stage `assignments`; 8 clauses)

`SELECT n.*,l.id AS target,l.kind AS target_kind,l.text AS target_text,r.id AS value,r.kind AS expr_kind,r.text AS expr_text,o.fn FROM nodes n JOIN nodes l ON l.id=<n.left> LEFT JOIN nodes r ON r.id=<n.right> LEFT JOIN ownership o ON o.id=n.id WHERE n.kind='assignment'`

| column | type | stage column | definition |
|---|---|---|---|
| `id` | `u32` | `id` (+ `n.*`) | nodes of kind `assignment` with `field(id, left)`, preorder |
| `target` | `u32` | `target`, `target_kind`, `target_text` | `field(id, left)` |
| `value` | `opt_u32` | `value`, `expr_kind`, `expr_text` | `field(id, right)` |
| `fn` | `opt_u32` | `fn` | `owner(id).fn` |

### 2.9 `decorator` (stage `decorators`; `legacy-execution-graphs`)

`SELECT f.__path,f.id AS fn,d.id,coalesce(c.callee,a.attr,child.text) AS name FROM functions f JOIN nodes wrapper ON wrapper.id=f.parent AND wrapper.kind='decorated_definition' JOIN nodes d ON d.parent=wrapper.id AND d.kind='decorator' JOIN nodes child ON child.parent=d.id LEFT JOIN calls c ON c.id=child.id LEFT JOIN attributes a ON a.id=child.id`

| column | type | stage column | definition |
|---|---|---|---|
| `fn` | `u32` | `fn` | a `function` row whose parent is a `decorated_definition` |
| `id` | `u32` | `id` | a `decorator` child of that wrapper |
| `child` | `u32` | (row identity) | each child of `id` in `N`, in order |
| `name` | `u32` | `name` = `text(name)` | `call(child).callee` if `child` has a `call` row; else `attribute(child).attr` if present; else `child` |

### 2.10 `import_identifier` (stage `import_identifiers`; `quality-contracts-1`, `-2`)

`SELECT n.__path,n.id FROM import_ranges imp JOIN nodes n ON n.__start>=imp.__start AND n.__start<=imp.__end WHERE n.kind='identifier' AND n.__end<=imp.__end`, with `import_ranges` = nodes of kind `import_statement`/`import_from_statement`.

| column | type | stage column | definition |
|---|---|---|---|
| `id` | `u32` | `id` | `identifier` nodes `n` with `start(imp) ≤ start(n) ≤ end(imp)` and `end(n) ≤ end(imp)` for an import statement `imp`; one row per (imp, n) |

Order: `imp` preorder, then `n` preorder.

### 2.11 `context` (stage `contexts`, `state-flow-1` and `state-flow-project-1`; rule-private constants, path-free part only)

| column | type | stage column | definition |
|---|---|---|---|
| `fn` | `u32` | `id` (+ `f.*`) | a `function` row with `owner(fn).top_fn = fn` |
| `class_name` | `text` (derived) | `class_name` | `coalesce(qual(owner(fn).cls), '')`, `qual(c) = name(c)` if `outer(c)` none, else `qual(outer(c)) || '.' || name(c)` (SQL `NULL` propagation) |
| `planning`, `preprocess`, `ctx_helper`, `materializer` | `tri` (0, 1, `NULL`) | same | the stage's boolean expressions, SQLite three-valued |
| `provider_helper_name` | `tri` | `provider_helper` = `__path='impl/common/l2070_server_plan/backend_profiles.py' AND provider_helper_name` | `rx_matches(name, 'component\|components\|score\|scores')`; the path conjunct is the binder's (facts are path-free) |

### 2.12 Refused as per-content facts: `special_attributes`, `return_kinds` (and `direct_events`)

Their shared texts (`state-flow-1`, `state-flow-project-1`) read `string_values`, which joins the clause's `raw_strings_*` matcher inputs, and `runtime(__path,name)` / `string_values` join rows **by name across the node set** (`r.name=call.callee`, `n.text=a.target_text`). A row's value then depends on which *other* nodes the clause selected, so no per-row selection filter of an all-nodes relation reproduces the stage: `Unsupported: name-keyed join across the clause's node selection`. They stay rule logic (S2's cross-clause sharing).

### 2.13 Binding a stage to a relation (for S2)

A row's **support** is the set of nodes whose presence in the clause's `nodes` the stage row needs: the ids in its key and id columns, and for `owner`/`class_name` the ancestor chain up to the owner. Over a clause selection `S`:

- the stage over `nodes(S)` returns exactly the relation's rows whose support is ⊆ `S`, **provided** `S` is closed under descendants for the nodes of the support (true for the `matches X or inside X` gates) — otherwise `ordinal`, `position`, `owner` and `class_name` are computed over `S`'s siblings and chains and must be recomputed;
- row order is the clause's `nodes` order of the key node (arm order, preorder within an arm), then the relation's inner order.

S2 owns the binding and re-verifies it with `FACTS_A3_CHECK`.

## 3. Exported relations (per content; Spike B § 5.1, v1)

Unchanged from Spike B v0 (`spikes/facts-b/src/facts.rs`): `def(name:sym, kind:sym, qualname:sym, line:u32)`, `ref(name:sym)` distinct, `import(module:sym, name:opt_sym, local:sym, top:u8, module_scope:u8)`, `call_edge(caller:sym, callee:sym)` distinct, `def_decorator(qualname:sym, name:sym)`, `dunder_all(literal:u8)`, `dunder_all_name(name:sym)`, `seed(qualname:sym, family:sym)`, `syntax_error()`. Definitions in Spike B § 5.1. Extended by S3's requests (repository plans' leaves), recorded here as they land.

## 4. Exactness (the differential)

`facts-a derived-check` builds, per corpus file, the reference: every clause that holds one of the stage texts above, with its gate and ancestor predicate forced `true`, its `nodes` (and stages up to the target) run by the SQL engine; and compares each target stage's rows (values, types and, after the documented reordering, order) with the relation rendered into the stage's columns. Results and per-file cost: report § 2.

## 5. Fact files v1

Layout as v0 (`RRFACTS\0`, `u32` version, 32-byte hash, strings, relations, typed columns, 4-byte padding after `u16`/`u8` columns), version **1**. The anonymous-node and compression decisions are recorded in the report with measurements; readers accept versions 0 and 1 (`facts-a inspect`).
