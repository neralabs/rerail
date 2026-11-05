# Group 4: what .rr lacks for 23 data-layer checks

## Shared facts for the checks below

These hold for every plugin-based check in this part, and are cited once here.

- **File scope.** `data_layer_production_dirs` (`lint_lib/discovery.py:32-41`) is `impl`, `data_layer`, `_internal` and
  every top-level directory matching `^l\d{4}_.+`. `include_tests=False` drops any path with a part `tests`, `test` or
  `conform` (`discovery.py:112-127`). This is expressible as a path regex on `doc` (`EXISTS doc, match`), e.g.
  `match("(impl|data_layer|_internal|l[0-9]{4}_[^/]+)/.*", p)`.
- **Unparseable files.** An `AstCollectorSpec` runs only on files that CPython parses (`fact_engine.py:1104-1108`,
  `_extract_ast_collector_facts` takes `parsed_ast`). Its `start_file` text scans are skipped as well, so a file
  with a syntax error yields no findings. The `SyntaxError` is not reported (`fact_engine.py:1349`).
  `parser_mode="source"` extractors (backend_type_strings, no_source_inspection, plan_composition) do run on
  unparseable files.
- **Extraction failures.** Any other extractor exception becomes a `FACT-EXTRACTION-FAILED` finding at line 1
  (`fact_engine.py:1339-1350`).
- **Source triggers.** `source_triggers` is a whole-file substring prefilter (`fact_engine.py:1178-1200`). It usually
  changes nothing, but where it does, it is noted under the check.
- **Line numbering.** Text scans number lines with Python's `str.splitlines()`, which also breaks on `\v \f \x1c-\x1e
  \x85    `. AST line numbers count only `\n`, `\r\n` and `\r`. The two numberings differ in any file that
  contains one of the extra terminators. This is the "line terminators" item of the earlier port's ledger (NER-1969).

---

### no_blanket_grammar_bypass (hard-fail, 366 lines)
- **Computes:** NGB001 for every source line, in production `.py` files, that is not a `#` comment line and
  contains `_ALLOW_DIRECT_EXECUTION` anywhere, including after a trailing `#`. Also for `allow_direct_execution`
  before the first `#` of the line. Also for every `def` or `async def` (at any depth) whose name is in a fixed
  set of 34 `execute_grammar*` names, in the single file `impl/common/l2200_sql_dialect/sql_connection.py`.
- **Inputs:** Python source text, line by line, and the Python AST. Paths: production dirs, plus the fixed file
  `impl/common/l2200_sql_dialect/sql_connection.py`.
- **Needs:**
  - `NATIVE src_line(d: number, l: number, s: span)`: one row per line of document `d`, numbered and split as
    Python's `str.splitlines()`. `s` is the line's text without its terminator, so it has `text` and can be the
    finding's `at`. Native, because core's `line` counts `\n` lines and a span is only made by a capture.
    Renumbering with the extra terminators would mean counting terminators before an offset, and spans expose
    no offset. (`lint_no_blanket_grammar_bypass.py:203-211`, `:231-239`)
  - `EXISTS match, contains` on `s`: `!match("[ \t]*#.*", s)`, `contains("_ALLOW_DIRECT_EXECUTION", s)`, and
    `match("[^#]*allow_direct_execution.*", s)`. The `[^#]*` prefix reproduces
    `stripped.split("#")[0]`; leading whitespace does not change the result, so no strip is needed.
  - `EXISTS def_name, docof, doc`: the method names, as facts, with the path equal to `SQL_CONNECTION_FILE`. The
    oracle checks every `FunctionDef` in the file, at any depth and on any class, not only `SyncSqlConnection`
    methods (`:213-220`).
  - `POLICY` (CLI only): the CLI reports `file not found` when `sql_connection.py` is missing (`:182-183`). The
    plugin does not. If kept, this needs a finding with no node: `!file("impl/…/sql_connection.py")`. That is a
    document-less finding, which `.violation(at: node, …)` has no form for.
  - `POLICY`: hard-fail, no baseline (`:300`, `:307`).
- **Hard parts:** One finding per line per term, even when the term occurs several times on the line. The
  `_ALLOW_DIRECT_EXECUTION` test also matches inside a trailing comment and inside strings and docstrings. The
  `allow_direct_execution` test treats a `#` inside a string literal as a comment start (a text heuristic, not
  tokenisation). `ALLOWED_FILES` are all under `scripts/`, so they are outside the plugin's scope and the
  exclusion is dead in the plugin. Files that CPython cannot parse produce nothing, because the text scan runs
  in an AST collector's `start_file`.

### classvar_check (hard-fail, 377 lines)
- **Computes:** CLASSVAR001 for each comparison whose *first* operand is a call whose callee is the name
  `get_origin` or any `x.get_origin`, with some operator `is`, `==`, `is not` or `!=` whose right-hand
  comparator is the name `ClassVar` or any `x.ClassVar`. Comparisons inside a sync `def is_classvar_annotation`
  (at any depth of nesting) are skipped. Production dirs.
- **Inputs:** Python AST.
- **Needs:**
  - `EXISTS kind, text, attr_of, child`: syntactic callee and comparator tests, with no name resolution, as in the
    oracle (`:139-150`).
  - `EXISTS within, def_name, is_async`: the exemption is any enclosing sync `FunctionDef` named
    `is_classvar_annotation`. `on_FunctionDef` only, so an async def of that name does not exempt
    (`lint_classvar_check.py:115-121`).
  - `NATIVE op_at(n: node, i: number, s: symbol)`: the i-th anonymous operator token of `n`, with two-token
    operators joined (`is not`, `not in`). core's `op(n, s)` gives the set of tokens but not their order, and
    tokens have no node or offset to sort by. `cmp` (names.rr) covers only unchained comparisons. The oracle
    zips `ops` with `comparators` over a whole chain: `get_origin(x) is A is ClassVar` is reported
    (`lint_classvar_check.py:146-151`).
  - `LIB cmp_link(c: node, i: number, op: symbol, r: node)`: operator `i` of comparison `c` and its right
    operand. Body: `kind(c, "comparison_operator"), op_at(c, i, op), child(c, r, i + 1)`.
  - `POLICY`: hard-fail; the message carries `is/==` or `is not/!=`, so both forms on one line are two findings
    (`:152-163`).
- **Hard parts:** The left operand is always `node.left`, the first operand. A `get_origin` call in a later chain
  position does not count. `ast` drops parentheses but tree-sitter keeps `parenthesized_expression`, so
  `(get_origin(x)) is ClassVar` needs unwrapping. The dead `PARSE` code in `lint_file` (`:173-184`) never fires,
  because `read_text` does not raise `SyntaxError`.

### backend_type_strings (hard-fail, 367 lines)
- **Computes:** On each source line that is not a `#` comment line, every match (`finditer`) of each of 8
  regexes: `dialect.name == "<backend>"`, `dialect.name in (`, `backend_type == …`, `backend == …`,
  `backend in (… "<backend>"`, `"<backend>" in …class_name`, `"<backend>" in type(`, `__class__.__name__.lower()`.
  The code (BT000–BT005) is chosen from the matched text. Files whose path string contains any of 13 skip
  substrings are skipped. Production dirs, `.py` files, with no AST (`parser_mode="source"`).
- **Inputs:** Python source text, per line. Paths: production dirs, minus the substring skip list (`conform/`,
  `/tests/`, `test_`, `l1400_types/__init__.py`, `impl/common/l2050_sql_types/backend_type.py`, `security-`, …).
- **Needs:**
  - `NATIVE src_line` (see no_blanket_grammar_bypass) (`lint_backend_type_strings.py:180-186`).
  - `EXISTS match_all` on the line span: one row per non-overlapping match, as `finditer` gives
    (`:191-192`). The backend alternation is a fact list, or literal text in the regex.
  - `EXISTS contains` on the match text, for the code choice (`:196-214`). For example, `backend  ==` with two
    spaces does not contain `"backend =="`, so it falls to BT000. This needs the whole match bound as a span
    capture `(?<m>…)`. That is already possible.
  - `EXISTS doc, contains`: a substring skip on the path (`:153-159`).
  - `POLICY`: one finding per match. Two matches of one pattern on a line give the same (path, line, code) key,
    and under the loose findings contract they collapse. The oracle keeps a `column` (`:220`). If multiplicity
    matters, the key needs the column, and there is no `col` relation: `NATIVE col(n: loc, c: number)`, 1-based
    (`:220`).
- **Hard parts:** `BACKEND_STRINGS` is a `frozenset`, so the alternation order varies with `PYTHONHASHSEED`. The
  surrounding quotes make the match set order-independent. `should_skip_file` tests the *absolute* path string
  (`:155`), so a checkout path containing `test_` or `security-` skips everything. A rule tests the
  repository-relative path, which is the intended meaning. Comment skipping is "line starts with `#`" only: a
  pattern in a trailing comment or a docstring is reported. The scan runs on unparseable files.

### type_safety_patterns (hard-fail, 735 lines)
- **Computes:** One code, TSP001, in nine categories. (a) Five line regexes run on every line, comments included:
  `str | Expression`, `str | Field`, `Expression[Any, Any]`, `_lt|_gt|_le|_ge|_eq|_ne` followed by `=` or `:`,
  and `.filter_(gt|…)(`. Each regex is skipped in a per-category allowlist of files. (b) For every `def` or
  `async def` named `order_by`, `where`, `select` or `group_by`, at any depth, a regex over the def's
  *source lines* (str-typed column/expression parameters). Findings are deduplicated by (file, line,
  category). Production dirs.
- **Inputs:** Python source text (lines), the Python AST (def names and line ranges), and the allowlist tables
  embedded in the script (`:144-252`).
- **Needs:**
  - `NATIVE src_line` (`lint_type_safety_patterns.py:317-321`).
  - `EXISTS match_all` (search semantics) on the line span, for the five line regexes (`:255-262`, `:321-387`).
  - `EXISTS` facts and `doc`: the per-category path allowlists (`:273-276`).
  - `NATIVE end_line(n: loc, l: number)`: the last line of a node, as `ast`'s `end_lineno`. core's `line` gives the
    start line only, and the line of a node's last descendant is wrong when that descendant spans lines
    (a multi-line string, for instance) (`:505`).
  - `NATIVE line_range(d: number, first: number, last: number, s: span)`: one span covering whole lines `first`
    to `last` of `d`, as `"\n".join(lines[first-1:last])`. The `where`, `select` and `group_by` regexes run on
    `lines[lineno-1:end_lineno]` (`:505-506`). That is not `text(f)`: it includes the indentation before `def`,
    and anything after the body on its last line. The `order_by` window is up to 11 lines from `def`, and need
    not end inside the function (`:496-503`). Datalog cannot build a span, and joining line texts in order needs
    an ordered string aggregate, which the language lacks. The alternative to this native is a `LANG` ordered
    `cat` aggregate: `w = cat : { src_line(d, k, s), L <= k, k <= E } order by k sep "\n"`.
  - `EXISTS min, count, contains`: the `order_by` window end is the first line in `[L, L+10]` containing `):`
    or `) ->`, else `min(L+10, nlines)`:
    `E = min k : { src_line(d, k, s), k >= L, k <= L + 10, (contains("):", s) ; contains(") ->", s)) }` (`:497-502`).
  - `EXISTS match_all` on the `line_range` span. A search across newlines needs `\s` and `[^,)]` to match `\n`,
    which they do in every regex dialect, so no dot-all flag is needed.
  - `POLICY`: dedupe key (file, line, category) (`:524-532`). The message is `"<category>: <suggestion>"`. Set
    semantics do this with `category` as a finding column.
- **Hard parts:** The whole-file prefilter (`:314`) lists `_lt`, `_gt`, `_le`, `_ge`, but not `_eq` or `_ne`. A file with
  `_eq=` but none of `str`, `Expression`, `Field`, `Any`, `filter_`, … is silently skipped. In practice `str` is in
  almost every file, but it is a document-level gate (`EXISTS contains` on the module text). Allowlists apply
  per category: `DEPRECATED_COMPARISON_OP` is never allowlisted, and `STR_IN_WHERE` has no allowlist. The text
  regexes match in comments and strings. `lineno` is the `def` line, not the decorator line, which matches where
  a tree-sitter `function_definition` starts. Unparseable files give nothing, since this is an AST collector.

### expression_owner_propagation (hard-fail, 410 lines)
- **Computes:** It finds calls to an `Expression` constructor that pass no non-None `owner=` keyword. The
  constructor is the name `Expression`, an alias imported `from … import Expression as E`, or a name assigned from
  an alias; the callee is stripped of subscripts, and both `Name` and `.attr` count. Each such call gets one code,
  tested in order:
  - EOP001: the field argument (`field=`, else the first positional) is `x.name`, or a function-local name
    aliased to such a value.
  - EOP002: the field argument is `x.field` (not on a call), `getattr(_, "field")`, or contains one of these. Or a
    semantic keyword (`left`, `right`, `args`, …) reads `p.<semantic attr>` for a parameter `p` annotated with
    `Expression`.
  - EOP003: the `args=` argument (else the 8th positional) references a "carrier" name. A carrier is a parameter
    annotated with `Column`, `Expression`, `Field` or `WhenClause`, closed under `x = …`, `x.append(…)` and
    `for x in …` within the function.

  Files under `impl/{doltdb,mongodb,neo4j}` are skipped. Production dirs.
- **Inputs:** Python AST.
- **Needs:**
  - `EXISTS doc, match`: the out-of-tree backend path skip (`lint_expression_owner_propagation.py:39-43`, `:188-190`).
  - `LIB call_name(c: node, s: symbol)`: callee text after stripping subscripts; a `Name` gives its id, an
    `Attribute` its attr. Body:
    `call_name(c,s) :- field(c,"function",f), head_name(f,s).` with `head_name` recursive through
    `subscript`'s `value` (`:75-83`).
  - `EXISTS import_alias, rasg, node offsets`: the constructor aliases. `from <any module> import Expression as E`
    adds `E`, and `Y = X` with `X` an alias adds `Y`, in any scope. Only those seen *earlier* in traversal count
    (`:203-208`, `:262-264`). This is monotone recursion with a position comparison (`start` from `node`), so
    plain Datalog handles it.
  - `EXISTS call_arg, value/kind none`: `_has_runtime_owner` (`:90-92`).
  - `LIB ast_expr_child(p: node, c: node)`: the children that Python's `ast.iter_child_nodes` yields *and* that are
    `ast.expr`, mapped onto tree-sitter. This skips the `argument_list` wrapper and excludes keyword-argument
    values (`ast.keyword` is not an `expr`) and comprehension clauses. Body: a rule per tree-sitter kind. The
    oracle's two recursive walks, `_source_code` (`:296-301`) and `_contains_carrier_reference` (`:310-314`),
    both use this child set, so `f(x=a.field)` nested in the value is *not* searched.
  - `LIB src_code(v: node, f: node, code: symbol)`: the recursive classification of `_source_code`. It gives
    EOP001 for `.name`, EOP002 for `.field` on a non-call, `getattr(_, "field")`, or any `ast_expr_child` with
    EOP002, and alias lookup for a bare name. EOP001 is not inherited from children (`:274-301`).
  - `LANG` recursion through an aggregate over a well-founded order: "latest alias before this point". A name's
    code is the code of the *latest* earlier assignment to it, in the same nearest `def` (or module), whose
    value has a non-None code. Rebinding to a no-code value does not clear it (`:265-269`, `:281-282`). The
    code of that earlier value can itself come from an alias lookup, so `src_code` recurses through `max`
    over source position. Soufflé rejects that as unstratifiable, though it is well-founded, since positions
    strictly decrease. Rules plus aggregates cannot say it unless the recursion is stratified by position.
    Example of a construct that would allow it, with positions as the stratifying order:
    `.decl alias(f: node, x: symbol, at: number, code: symbol) ordered by at`
    `last(f, x, p, c) :- p0 = max a : { alias(f, x, a, _), a < p }, alias(f, x, p0, c).`
  - `LIB carrier(f: node, x: symbol)`: a recursive fixpoint. Seeds: parameters of `f` whose annotation contains a
    `Name` or `.attr` in {Column, Expression, Field, WhenClause}. Steps: `x.append(src)` with exactly one
    positional argument, `x = src` with a single name target, and `for x in src`, where `src` contains a
    carrier name at any depth (`ast.walk`, which also descends into keywords and lambdas). The search is
    anywhere in `f`, nested defs included (`:119-166`). Positional-only, keyword-only, `*args` and `**kw`
    parameters all count (`:113-116`). `EXISTS within, nameref, param` for the walk.
  - `LIB carrier_ref(v: node, f: node)`: recursive over `ast_expr_child`. It stops at an `Expression` constructor
    call whose keywords are all `value` (positional arguments are ignored), and is true at a carrier name
    (`:303-314`).
  - `LIB expr_input(f: node, x: symbol)`: parameters annotated with `Expression` or `Expression[…]`, also as `.attr`,
    anywhere in the annotation (`:169-185`). `EXISTS`-composable from `param`, `within`, `attr_of`.
  - `EXISTS call_arg, within, attr_of`: `_uses_expression_structure`, a walk of the semantic keyword values
    (`:316-336`).
  - `EXISTS !`: the codes are exclusive, EOP001 then EOP002 then EOP003 (`:227-240`). Write the later ones with
    `!eop001(c)` and `!eop002(c)`.
  - `POLICY`: dedupe on (code, line, message) (`:338-341`). Set semantics do this.
- **Hard parts:** Order is the AST's pre-order traversal (`fact_engine.py:817-842`), not text order. Within a
  statement, Python's field order differs from source order in places: a decorator list is visited *after* the
  body, keyword arguments after positionals. An alias assigned in a decorator is therefore "later" than calls in
  the body. Start offsets approximate this, and differ only in such corners.

  The enclosing function is the nearest `def` or `async def` (lambdas and classes are transparent), and aliases
  are keyed by that function, with no scope chain. A name in a nested def does not see the outer def's
  aliases. Constructor aliases are module-wide and only accumulate.

  The 8th positional argument stands in for `args` (`:235-236`).

  The prefilter is `"Expression"` in the file. It is harmless, since every alias originates from that text.

### broad_type_surfaces (metric, 1473 lines)
- **Computes:** an inventory of "broad" type surfaces (codes BTS001–BTS016) in production files whose path has a part
  containing `connection`, `schema`, `executor` or `optimizer`. It checks annotations of module- and class-level
  `x: T`, of every function parameter (except ones named `self`/`cls`), `*args`/`**kw`, and non-`None` returns. Also
  Protocol classes, PEP 695 type parameters, brand constructors, `tuple([...])`, and nominal-text producers. Findings
  are filtered by a signed accepted-surface allowlist. Counts per code are ratcheted against a baseline.
- **Inputs:** Python AST of `.py` files under `data_layer`, `impl`, `l1000_schema`, `l2000_connection`, `l2070_batching`,
  `l3200_metadata`, `l7000_versioned_schema` (`--broad`, which lint_all passes). Excluded: any path part in
  {`__pycache__`, `tests`, `test`, `conform`, `examples`}, and `impl/*` outside `impl/{common,sqlite,postgres,mariadb}/`.
  Also: imported modules' ASTs, resolved by path (`_module_path`, `lint_broad_type_surfaces.py:975`), which may be
  outside that scope. Signed JSONL `scripts/allowlists/lint_broad_type_surfaces_allowlist.jsonl`, with its signature
  held as a constant in the script (`:30`) and checked by `l2070_batching/allowlist_signing.verify_allowlist`. Ratchet
  baseline `.ratchet-baselines/lint_broad_type_surfaces.json` (per-code counts plus `total`, Ed25519 `_metadata.signature`).
- **Needs:**
  - `EXISTS path_part, doc, match, contains`: the file filter. The "a part contains a keyword" test is
    `path_part(d, p), match(".*(connection|schema|executor|optimizer).*", lower(p))`. Lowercasing is via
    `@convert(p, "lowerCase", "")`, or a case-insensitive regex `(?i)`.
  - `EXISTS match_all, text, str`: the annotation's name set is `re.findall(r"[A-Za-z_]\w*", text)` (`:892`).
    That is `match_all("(?<x>[A-Za-z_]\\w*)", t)`, with t the annotation's text, or the string's contents for a
    string annotation (`_annotation_text`, `:885`). BTS001/002/005/006/013 test that set. BTS007 is text = "None",
    and BTS003 is `Optional\s*[`, or `|` together with `None` (`:899`). All are regex tests on the same text.
  - `EXISTS kind, field, child, def_name, decorator, nearest, call_arg, str`: BTS006 (a class with a base
    whose root name is `Protocol` and no type params), BTS010 (`Unset` in the return names, and none of
    LazyResult/ScalarServerRef/ServerRef), BTS011 (callee root name in a fixed brand set, first argument `""`),
    BTS012 (`tuple([..])` or `tuple([.. for ..])`), BTS014 (a public function with a `tuple[X, ...]` parameter).
  - `EXISTS count`: BTS008/BTS009 count how often a type parameter's name appears in the signature or in class-body
    annotations (`_name_occurrences`, `:617`). `count : { … }` over identifier nodes `within` those annotation roots.
  - `NATIVE grammar: PEP 696 type-parameter defaults` (`class C[T: X = X]`). BTS009 reads `type_parameter.default_value`
    (`:701`), and the repository uses the syntax (e.g. `impl/common/l2066_plan_optimizer/ir.py:49`). If the bundled
    tree-sitter-python does not parse `= default` in a type parameter list, the class header becomes an ERROR node, and
    BTS009 plus every finding in that class is lost. Only the parser can supply it. Verify against the grammar version.
  - `LIB type_alias(d: number, name: symbol, alias: node, value: node)` and `alias_param(alias: node, i: number, x: symbol)`.
    Body: `type_alias_statement` at module level, and its `type_parameter` names. Then
    `alias_ref(n, a)`: an identifier in annotation n resolves to alias a, locally or through a module-level
    `from m import X [as Y]` (`ifrom`/`rifrom` + `module_path`, followed transitively as the oracle's per-module
    alias table is, `:1082`). `expanded_name(n, x)` (recursive): x is a name of n's text, or of an alias value reached
    from n. Type-parameter names in an alias value are replaced by the argument's names. Then
    `broad_expanded(n) :- expanded_name(n, x), broad_name(x)`, plus the None/Optional tests over reached values.
    BTS015 (`:493-505`), including the "identity alias" case (`type X[T] = T`).
  - `LIB class_field(d: number, cls: symbol, field: symbol, ann: node)` and `ctor_param(d, cls, param, ann)`.
    class_field: AnnAssign with a Name target in a top-level class body. ctor_param: a top-level class's `__init__`
    parameters other than self/cls, with annotations. Both are imported through module-level `from` imports under
    the local alias (`:1002`, `:1034`). Used by BTS016's keyword-argument and attribute-assignment producers. The
    constructor table wins over fields when the class has `__init__` (`:362`).
  - `LIB nominal_text(ann: node)`, `explicit_contract(v: node, ann: node)`, `unbranded_text(v: node, ann: node)`:
    BTS016's three predicates (`:802`, `:810`, `:867`). A union/Optional member's root name ends in one of 12
    suffixes, and none is `str`. The value is an Attribute, None, a call of a contract name, or a Name last bound to
    such a call. It is a str literal, an f-string, `str(...)`, or a Name/subscript/`.get` of a module-annotated `str`
    mapping. Plain Datalog over kinds and name maps.
  - `LIB for_elem_type(loop: node, var: symbol, cls: symbol)`: inside `for v in xs:`, v has type cls, if `xs: list[C]`
    (or Iterable/Iterator/Sequence/tuple) or `xs.append(C(...))` appears anywhere in the module (`:721`, `:332`).
    The type applies only within the loop body (`:346`). It gives the class for BTS016's `v.attr = value` (`:462`).
  - `LIB class_func_path(n: node, s: symbol)`: the `scope` column (`:559`). It joins every enclosing class name
    and then every enclosing function name, each list outermost first, with "." between. This is not the true nesting
    order: in `def f(): class C: def g` it gives `C.f.g`. Two recursive chains (classes, functions), each built with
    `cat`, then concatenated. The scope column is part of the allowlist key.
  - `NATIVE unparse(n: node, s: symbol)`: CPython `ast.unparse` text of a node. The allowlist key holds `declaration` =
    `@dec def name[T,…](unparsed args)->unparsed returns` (`:582-589`). It also holds `contract` = the unparsed
    finding node (`:885-888`; for BTS006 that is the whole class). Unparse normalises quotes, spacing, parentheses,
    comments and number literals. Rebuilding it in Datalog would mean a pretty-printer with operator precedence and
    string re-quoting for every node kind. Without it, signed allowlist entries cannot be matched exactly.
  - `LANG ordered string aggregate`. The BTS004 detail lists the sorted broad collection names joined with ", "
    (`:529-533`). The BTS015 detail lists the sorted alias names the same way (`:498`). The detail is part of the
    ratchet identity in broad_type_surfaces_ratchet, so it must be exact. `count`/`min`/`max` cannot build a string,
    and enumerating every subset is not feasible (alias names are unbounded).
    ```
    names(a, s) :- annotation_root(a), s = join(", ") : { ann_name(a, x), broad_collection(x) } order by x.
    BTS004(a, cat("broad/mutable collection annotation ", s, "; use tuple, …")) :- names(a, s), s != "".
    ```
  - `NATIVE signed_jsonl(path: symbol, line: number, key: symbol, value: symbol)`, or `signed_list` extended to
    parse JSON lines. The allowlist is canonical JSONL with six string fields (`:1203-1238`). `signed_list` gives
    whole lines. Matching them against JSON-escaped finding fields with `match` captures would need JSON
    unescaping. If the signature does not verify, the oracle silently applies no allowlist (`:1264`).
  - `POLICY`: an allowlist entry counts only when the finding's own source line contains `# broad-type: accepted`
    (`:1276`). That is expressible as `EXISTS kind(c,"comment"), line(c, l), contains(text, marker)`, given the
    same-document join. The oracle tests the raw line, so a marker inside a string also counts.
  - `POLICY .budget * by code` with the counts held in an external baseline file: `.ratchet-baselines/lint_broad_type_surfaces.json`
    (`:1393`, `:1461`), with `require_signature=False`. The signature is checked if present
    (`lint_lib/ratchet.py:169-190`), and a missing baseline passes. A code absent from the baseline has ceiling 0.
    `total` is also ratcheted. Today's `.budget` is inline numbers. This needs `.budget * by code from "<file>"
    signed optional`.
  - `POLICY metric enforcement`. In lint_all (`lint_all.py:2033-2051`) the check runs with `--broad --json` and no
    `--check`. Every finding becomes a non-blocking severity-"info" finding (`lint_all.py:573-625`), and output that is
    not JSON becomes a single `INVENTORY_FAILED` finding. The LinterConfig (`lint_all.py:1156`) adds `--check`, the
    per-code ratchet. There are also codemod-opportunity counts (`:1317`), which are metrics only (`.output`).
  - `POLICY` fact scope wider than report scope. Class fields, constructors and aliases come from any imported repository module (`:975-993`),
    including modules outside the keyword-filtered scan set. If `.scope` limits which files are parsed, these
    rules need "parse for facts, do not report".
- **Hard parts:**
  - "on non-generic surface" in the messages is not enforced: `_generic_depth` is tracked (`:221`, `:275`, `:412`) but
    never read, so annotations inside generic classes and functions are reported too. Only BTS004 is relaxed: a
    `list[T]` where every list argument is a bound type parameter of an enclosing def or class (`:196`, `:525`).
  - The name set comes from a regex over unparsed text, so `typing.Any` yields both `typing` and `Any`, and names inside
    nested string literals count. With tree-sitter text, a comment inside a multi-line annotation would add names
    that unparse drops.
  - BTS016's maps are flat per module, and the last binding wins: the dict is overwritten in `ast.walk` order, which
    is breadth-first, not source order (`:750`, `:764`). Datalog will union every binding (may-semantics). That is a
    superset unless "last in BFS order" is modelled, and it should not be.
  - An annotated parameter is reported once per code that applies, so one annotation can carry BTS001, BTS003, BTS004
    and BTS015 together.
  - Imported aliases, fields and constructors follow only module-level `from` imports (`tree.body`), not `import m; m.X`,
    and resolve by file path from ROOT, not by `sys.path`.

### broad_type_surfaces_ratchet (hard-fail, 95 lines)
- **Computes:** runs the same scanner over the broad roots and the narrow default targets. The defaults add
  files such as `l3200_metadata/bootstrap_state.py` whose names have no keyword. It then counts findings per stable
  identity `[path, owner, code, surface, detail]`. It fails on any identity whose count rises above its signed baseline
  (deletions never offset), and on any "anti-dodge" finding (BTS008–BTS014, BTS016), which no baseline can admit.
- **Inputs:** as broad_type_surfaces (Python AST, signed accepted-surface allowlist), plus
  `.ratchet-baselines/lint_broad_type_findings.json`. Its keys are JSON-array strings, its counts must be positive
  ints, and `_metadata.signature` is required. The helper is `scripts/broad_type_finding_inventory.py`.
- **Needs:**
  - `EXISTS`: every per-finding need of broad_type_surfaces; the ratchet reuses `lint_paths` (`lint_broad_type_surfaces_ratchet.py:31-32`).
  - `LIB owner_path(n: node, s: symbol)`: the identity's owner (`broad_type_finding_inventory.py:22-37`). It is the
    innermost def or class whose line range `[lineno, end_lineno]` contains the finding's line, named by its real
    nesting path joined with ".", or `<module>`. Recursive `cat` along `nearest(…, scope)` over def and class nodes,
    with the node itself included when it is a def or class (BTS006 owns its own class). This differs from the
    scope column above, which is ordered classes-then-functions.
  - `LANG ordered string aggregate`: as above. `detail` is part of the identity, and it is built with sorted joins
    for BTS004 and BTS015.
  - `POLICY per-identity budget from a signed file`: `.budget * by (path, owner, code, surface, detail) from
    ".ratchet-baselines/lint_broad_type_findings.json" signed required` (`:33-36`, `:65-90`). The ceiling applies
    to each key separately, and a missing key means 0 (`broad_type_finding_inventory.py:44-50`). These failures
    are themselves findings: a missing or unreadable baseline, a baseline that is not an object, a missing
    signature, a count that is not a positive int (`:66-81`), and a signature that fails Ed25519 over (name,
    counts) (`lint_lib/ratchet.py:169-190`). The count is per identity, and several findings on different lines
    share one key.
  - `POLICY unbudgetable codes`: findings with codes BTS008–BTS014 or BTS016 always fail, even if the budget would admit
    them, and `--update-baseline` refuses them (`:33`, `:50-52`; `broad_type_finding_inventory.py:12`). This could be
    written as `.budget BTS008…BTS016 { "total": 0 } locked`, or by excluding those codes from the `by` budget.
- **Hard parts:**
  - The owner is chosen by line range, not by tree position: the range starts at the `def`/`class` line (not the
    decorator), and when several ranges contain the line the last in preorder wins (`owners[-1]`). That is the
    structural nearest ancestor except when two defs share a line.
  - The scope is the union of two file sets. Explicit default-target files skip the keyword filter
    (`lint_broad_type_surfaces.py:1194`).

### no_source_inspection (hard-fail, 252 lines)
- **Computes:** NSI001 on each line, not a `#` comment line, matching any of 5 regexes: `inspect.getsource(`,
  `inspect.getsourcelines(`, `inspect.getsourcefile(`, and `from inspect import … getsource` or `getsourcelines`.
  One finding per (line, pattern). Scope is the top-level `tests/` and `conform/` trees, tests included.
- **Inputs:** Python source text (lines), with no AST. Paths: `tests/**`, `conform/**`, minus `__pycache__` and
  `.pytest_cache` parts.
- **Needs:**
  - `NATIVE src_line` (`lint_no_source_inspection.py:137-142`).
  - `EXISTS match_all`, `match("[ \t]*#.*", s)` on the line span. The pattern name is a column, as a fact per
    regex (`:63-69`).
  - `EXISTS .scope dirs "tests", "conform"`, `.scope tests true` (`:183-190`).
  - `EXISTS contains` on the module text: the file prefilter (see Hard parts) (`:75-80`).
  - `POLICY`: hard-fail, no baseline.
- **Hard parts:** The source trigger changes the result. The prefilter needs the literal `from inspect import`
  with single spaces, while the regexes allow `\s+`. So `from  inspect import getsource` (two spaces) in a file
  with no other trigger is not reported. `from inspect import getsource, getsourcelines` gives two findings,
  because the patterns' messages differ. Since the scan uses `parser_mode="source"`, unparseable test files are
  scanned too.

### bootstrap_cache_health (hard-fail, 370 lines)
- **Computes:** in `conform/**/test_*.py`, calls of the bootstrap test helpers or `create_table(s)`:
  - BCH001: a helper is called (bare or `x.helper`) inside a test or fixture.
  - BCH002: any helper call has a `manifest_key=` keyword, wherever it is.
  - BCH003: a bare-name `create_table(s)()` inside a test or fixture.
  "Test or fixture" means the innermost enclosing def is named `test_*`, or carries a decorator whose dotted name
  ends with `pytest.fixture` or equals `fixture`. A `bootstrap-cache-health: allow` marker on the call's line or
  the line before suppresses it.
- **Inputs:** Python AST of `conform/**/test_*.py` (excluding `__pycache__`, `.pytest_cache`). The plugin path uses
  `FileScopeSpec(directories=("conform",), include_tests=True)`, i.e. all `.py` files under conform
  (`lint_bootstrap_cache_health.py:337`). An in-script allowlist that must stay empty, with a signature borrowed
  from another linter's label (`:98-105`).
- **Needs:**
  - `EXISTS kind, field, text, call_arg, nearest(c, f, fdef), def_name, decorator, dotted, match`. The callee root
    name is the Name id or the Attribute's attr (`:200`). The decorator name is the dotted text of the decorator, or of
    its callee when it is a call (`:208`). The test-or-fixture test is `nearest(c, f, fdef)`, then def_name or
    decorator (`:167-176`).
  - `EXISTS kind(c, "comment"), line, contains`: the allow marker on line L or L-1 of the call's start line (`:178-183`). The
    oracle tests the raw line text, so a marker inside a string literal also suppresses. With comment nodes it does not.
  - `EXISTS .scope dirs "conform"`, `file_name` + `match("test_.*\\.py", …)`, `.scope tests true`.
  - `EXISTS signed_list`: the allowlist is empty by design (`:98`). In `.rr` it is dropped, or kept as an empty signed
    list keyed `code:path:line`.
  - `POLICY extraction-failure findings`: a file whose fact extraction fails yields `BCH000` at line 1 with the
    error text (`:253-255`). A file that cannot be read or decoded is skipped silently (`:282-285`).
  - `POLICY hard-fail on bad allowlist signature`: if the (empty) allowlist's signature fails, the whole check fails
    before scanning (`:351-353`). This matches `signed_list`'s "refused", if refused means the policy fails.
- **Hard parts:**
  - "Innermost function" is the innermost `def` or `async def`, not a lambda, and a class in between does not stop
    it (`function_stack[-1]`, `:170`). This is `nearest(…, fdef)` with fdef = function_definition only.
  - BCH001 and BCH002 can both fire on one call.
  - The line is `Call.lineno`, the start of the callee expression. For `obj.helper(...)` split over lines, that is
    the line of `obj`.
  - `_is_scope_execute_call` (`:219`) is dead code.

### plan_optimizer_v2 (hard-fail, 1032 lines)
- **Computes:** 20 invariant rules (PV2-001..060) over the v2 optimizer packages: import purity and layer DAG,
  single-scope discipline, magic strings, contract fallbacks, raise messages without repair hints, legality-oracle
  placement, pass structure in `passes.py`, a 600-line file cap, and a three-way sync between the registered
  `OptimizerInvariant(code=…)` calls in `verifier.py`, their `(code)` citations, and a markdown design doc.
  Findings are deduplicated per (code, line) within a file.
- **Inputs:** Python AST of `impl/common/l2065_plan_core/**/*.py` and `impl/common/l2066_plan_optimizer/**/*.py`
  (excluding `__pycache__`, `.pytest_cache`, `tests`). Markdown text of `docs/design/PLAN_OPTIMIZER_V2_INVARIANTS.md`.
  The raw source text of `impl/common/l2065_plan_core/verifier.py`.
- **Needs:**
  - `EXISTS import_alias, module_text, rel_base, path_part, file_name, match`: PV2-001/002/003 (`lint_plan_optimizer_v2.py:254-318`).
    Every `import` alias and every absolute `from m import` (level 0; relative imports skipped) anywhere in the file
    is tested by prefix (`m == p` or `m` starts with `p.`). In core files (`l2065_plan_core` among the path parts)
    the legacy and l2066 prefixes are forbidden. Elsewhere legacy prefixes are forbidden except in `adapter.py`.
    An I/O import reports only PV2-001 (`continue`).
  - `EXISTS attr_of, call_arg, value/str, kind, field, text`: PV2-010 (attr `_bindings`/`_exports`, except in
    scope.py), PV2-011 (`.partition/.split/.rpartition/.rsplit(".")`), PV2-022 (`&` with a bare `set(...)` or
    `frozenset(...)` call on either side, or any `.intersection(...)`), PV2-023 (`ColumnSchema("…")` with a bare-name
    callee), PV2-060 (a non-async def named in a set, an attr in a set, a `.can_share_statement_after()` call; off in
    effects.py and adapter.py).
  - `EXISTS kind, child, str`: PV2-020 (`:396-409`). Any operand of a comparison, chained ones included, is a
    constant in {count_only, count, exists}; adapter.py is exempt. `cmp` is not usable, because it drops chained
    comparisons. Use `child(c, o, _)` of the `comparison_operator`.
  - `EXISTS kind, str, match`: PV2-021 (`:410-433`). Every str constant ending in `_count`, docstrings
    included, and every literal segment of an f-string ending in `_count`. In tree-sitter the f-string segments are the
    `string_content` children between `interpolation` nodes. Byte strings must be excluded (prefix test on the string
    node).
  - `LIB ast_name(n: node, x: symbol)`: an identifier that CPython's ast would make an `ast.Name` (Load, Store or Del).
    That is `nameref` minus identifiers inside `dotted_name`/`aliased_import` of import statements, parameter
    names, `global`/`nonlocal` names, and keyword-pattern names. PV2-030 (`:478-494`) reports an `ast.Name` or
    `ast.Attribute` whose name is in {model_fields, model_sql_column_type_hints, model_json_schema, get_type_hints}.
    `nameref` would wrongly include `from typing import get_type_hints`. PV2-043 (`:707`) also needs it: the
    `PlanVerifier` Name test.
  - `EXISTS within, kind, field, str, match`: PV2-031 (`:497-528`). For every def (sync or async, nested ones
    too), take the raises inside it (nested defs included) whose exception is neither absent nor a bare Name. If no
    str constant anywhere inside the def matches `\b(Fix|Use|…|Keep)\b` (decorators, annotations, docstrings and nested
    defs included), each such raise is reported. Dedup per line keeps the first, outermost function, so: report a
    raise if some enclosing def has no hint string.
  - `EXISTS within, def_name`: PV2-042 (`:636-649`). An assignment whose first target is an attribute, with no lexically
    enclosing def (at any depth) named `__init__`/`__post_init__`. Module-level ones count. AugAssign/AnnAssign do not.
  - `EXISTS child, precedes, kind, field, text, attr_of`: PV2-040/041/043 in `passes.py` only (`:650-718`). PV2-040: a
    top-level `*Pass` class with no Protocol base lacks `phase: … = X.Y`, a `name:` AnnAssign, or a sync `def run`.
    PV2-041: in a top-level `DEFAULT_V2_PASSES = (…)` tuple, the phase ranks of `Cls()` elements must not decrease.
    Only classes defined *before* the assignment count (phase_by_class is filled in statement order). "ranks !=
    sorted(ranks)" is an inversion pair: elements i before j with rank(i) > rank(j). This is expressible with
    child indexes.
  - `NATIVE doc_lines(d: number, k: number)`: the file's line count, `source.count("\n") + 1` (`:626-627`), for PV2-044. Core has
    only start lines. The module node's byte range need not cover leading or trailing trivia, and counting
    newline captures over its text depends on that. It is a property of the file bytes, which the reader has.
  - `NATIVE repo_text(path: symbol, s: symbol)`: the raw text of a repository file that is not source code, here the
    markdown doc (`:769`). `match_all("`(?<c>(?:scope|schema|effects|cardinality|rewrite)\\.[a-z_0-9]+)`", s)` then
    gives the doc's codes (`:738`, `:780`), which is EXISTS once the text is available. `.input … IO=file` reads policy
    files, not repository files. `file(path)` gives existence only.
  - `EXISTS text(module), contains, cat`: PV2-052's "cited" (`:754`) is a substring `(code)` anywhere in verifier.py's
    source, comments and the registration call included.
  - `LANG findings anchored at (path, line)`, not at a node. PV2-050 on a missing doc and PV2-051 report at
    `docs/design/PLAN_OPTIMIZER_V2_INVARIANTS.md:1` (`:771-778`, `:812-822`). That file is not a parsed document, so no
    `loc` exists for it. `.violation`'s `at` must accept a path and a line.
    ```
    .violation PV2_051(file: symbol, line: number, code: symbol) "Doc lists invariant code {code} …"
    PV2_051(p, 1, c) :- doc_code(p, c), !registered(c).
    ```
  - `EXISTS min`: dedup per (code, line, file) (`:836-839`) keeps the first finding, which picks its message. For keys
    of (code, path, line), report one finding per line, taking `min` over the candidate node ids, or omit the
    message-varying columns.
  - `POLICY`: zero baseline, hard-fail. A file that cannot be read or decoded is skipped silently (`:890-893`). The
    plugin path computes doc-sync findings only when verifier.py's facts were seen (`:957-967`). The CLI path always
    computes them.
- **Hard parts:**
  - PV2-031's descendant set includes the def's decorators, defaults and annotations, and all nested defs. One hint
    string anywhere in an outer def silences that def's raises, but not a nested def's, if the nested def itself has
    no hint.
  - PV2-021 also fires on docstrings and on any identifier-like literal such as `"row_count"`, so the reach is large.
    Tree-sitter splits strings at escape sequences, so "ends with `_count`" must use the unescaped segment value.
  - PV2-012's names are direct Name targets of `=` (each target of a chain), AnnAssign Name targets, and def parameters
    (posonly/args/kwonly, not `*a`/`**k`), each at the target's own line. They are not tuple-unpacking targets or loop
    targets, so `binding(…, "assign")` is too wide.
  - PV2-030's line is the Attribute node's start, the line where the object expression begins.
  - Import findings use the statement's line. `from m import a, b` gives one finding per statement. That matches the
    per-line dedup, but not a per-alias rule without dedup.

### plan_composition_architecture (hard-fail, 354 lines; plugin marker 8 lines)
- **Computes:** It scans one file, `l2070_batching/semantic_plan.py`, and reports five codes:
  - SPC-001: an `import`/`from` anywhere in the file whose module text contains `cache`, `connection`, `executor`,
    `plan_flush`, `runtime_services` or `transaction`, or ends in `.lazy_result` or `.lazy_seq`.
  - SPC-002: an annotated assignment anywhere whose unparsed annotation contains `Connection`, `Cursor`,
    `Executor`, `LazyResult`, `PlanFlushContext` or `Transaction`.
  - SPC-004: in a top-level `class SemanticPlanBuilder`, a public sync method whose return annotation root is
    `dict`, `list` or `set`.
  - SPC-003: in the same class, a call `.commit`, `.execute`, `.resolve`, … anywhere inside a sync method.
  - SPC-005: the class's `add_fragment` does not mention both of the names `RefRemap` and
    `rewrite_exact_plan_refs_payload`.

  Two further findings, SPC-012 and SPC-005, come from *running* the project's `SemanticPlanBuilder`: a
  fingerprint determinism probe and a fragment ref-collision probe.
- **Inputs:** The Python AST of `l2070_batching/semantic_plan.py`, plus runtime: it imports and executes
  `l2070_batching.semantic_plan` and `impl.common.l2070_server_plan.steps.collect_results`.
- **Needs:**
  - `EXISTS doc` path test (`lint_plan_composition_architecture.py:229-230`, `:294`).
  - `EXISTS import_stmt, import_alias, module_text, contains, match`: SPC-001. `_import_name` is the
    `from` module without its leading dots (`node.module`), or the comma-joined names of an `import`
    (`:74-77`, `:99-113`). module_text resolves relative imports to absolute names, which can add a matching
    part such as `l2070_batching`. The exact meaning needs the written module text: the dotted name after the
    dots, which is `text` of the `module_name` field's dotted child (`EXISTS field, text`).
  - `EXISTS kind, field, text, contains`: SPC-002 on the `type` field of an annotated `assignment`, using its
    source text in place of `ast.unparse` (`:114-124`).
  - `EXISTS def_name, child(modnode…), method_of, is_async, within, attr_of, nameref`: SPC-003, 004 and 005 (`:126-164`).
    The oracle uses direct `class_node.body` `FunctionDef`s only, which `method_of` approximates.
  - `POLICY` runtime behaviour probes (`:168-225`, `:273`): the check imports project modules, builds plans, and
    compares fingerprints and ref maps. No static rule can express this, nor should one. Either it stays a
    project test, or the policy accepts an external result as findings: a native reader for a probe-result file
    produced by the test suite (`.input probe(IO=file, …)`). Then the rule only maps rows to SPC-012 and SPC-005.
- **Hard parts:** SPC-002 tests `ast.unparse`. The test is substring-based, so source text agrees except for
  comments inside a multi-line annotation, and string annotations (`"Connection"`), which match either way.
  `nameref` includes parameter names, while `ast.Name` does not; this matters for SPC-005 only if a parameter is
  named `RefRemap`. Methods inside an `if` in the class body count for `method_of` but not for the oracle. Calls
  are found by `ast.walk(method)`, so nested defs and lambdas inside the method count (`within`, not
  `within_stop`). The probes' findings are always at line 1 of `l2070_batching/semantic_plan.py`.

### dynamic_module_loading (hard-fail, 491 lines)
- **Computes:** a line-by-line regex scan of production Python files. It reports module-level `def __getattr__(`,
  `globals()[…] =`, `__import__(` and `import_module(` / `importlib.import_module(`. It skips docstring lines,
  comment lines and lines it guesses are under `if TYPE_CHECKING:`. A failed allowlist signature gives one DML000
  and no other findings.
- **Inputs:** Python source text, read as lines (not the AST). Scope: `impl/`, `data_layer/`, `_internal/` and every
  top-level `l\d{4}_.+` directory (`lint_lib/discovery.py:28-41`). Directories `tests`, `test` and `conform` are
  excluded, as are the SKIP_DIRS (`discovery.py:15-25,112-127`). Also a signed allowlist that lives inside the
  script itself: `ALLOWLISTED_FILES` and `ALLOWLIST_SIGNATURE` (`lint_dynamic_module_loading.py:146-150`).
- **Needs:**
  - `EXISTS doc, match`: the scope, as a regex on the document path (`l[0-9]{4}_.*/.*`). The candidate JSON lists
    all 90-odd layer directories by hand. `.scope dirs` with a glob or regex would avoid that.
  - `EXISTS contains, text(modnode)`: the whole-file pre-filter (`:253`). The file must contain one of
    `__getattr__(`, `globals()[`, `__import__(` or `import_module(`.
  - `LIB source_line(d: number, l: number, s: symbol)`: each physical line of document d. Body:
    `modnode(d,m), match_all("(?<L>[^\n\r\x0b\x0c\x1c-\x1e\x85  ]*)…", m), line(L,l), text(L,s)`. It must
    split where Python's `str.splitlines` splits, which `(?m)^…$` does not (`:256`).
  - `LIB in_docstring(d: number, l: number)`: a per-line state machine, written as recursion over l
    (`state(d,l+1) :- state(d,l), …`). A line in "code" state with exactly one `"""` opens a docstring. With two
    or more it is skipped but stays code. `'''` is the same, and `"""` is tested first. In docstring state, any
    line containing the opening delimiter closes it, and is skipped too (`:257-285`).
    `count : { match_all("\"\"\"", s) }` gives the delimiter count.
  - `LIB tc_guard_line(d: number, l: number)`: the TYPE_CHECKING guess (`:209-221`). Let z be the nearest line
    above l with indent 0, ignoring blank and comment lines. Some non-blank, non-comment line in [z, l) has
    indent < indent(l) and contains `TYPE_CHECKING`. Indent comes from `match("(?<W>\\s*).*", s)` and
    `strlen(W)`, and z from `max l0 : {…}`. This is not `type_checking_block`, which finds the real `if`.
  - `EXISTS match`: the five line regexes. If any of the three "lazy" patterns matches, the two `import_module`
    patterns are suppressed (`:295-306`): a negation. `importlib.import_module(` matches both import_module
    regexes, so it gives two findings on one line.
  - `NATIVE signed_list` format: entries are checked with Ed25519 over the canonical JSON
    `{"allowlist": sorted, "linter": name, "version": 1}`, with the repository's `PUBLIC_KEY_HEX`
    (`lint_lib/signing.py:33,65-83,107-157`). The `NEEDS_SIGNING`, `UNSIGNED` and `""` sentinels count as failure.
    Core `signed_list` reads a separate file and does not name a signature scheme. Here the list is a constant
    inside the script.
  - `POLICY signature-failure finding`: a failed check is a finding (DML000 at
    `scripts/lint_dynamic_module_loading.py:1`) that replaces every other finding (`:352-362`). Core's
    `signed_list` is "refused", which defines no finding.
- **Hard parts:** this is text, not syntax. `x = """…""" + __import__("m")` on one line is skipped, because the
  line has two `"""`. A `__import__(` inside a one-line string (not a triple-quoted one) is reported. The
  TYPE_CHECKING test is about indentation, not structure: an `else:` at column 0 ends it, and so would
  `x = TYPE_CHECKING` on a line with less indent. Line numbers follow `splitlines`, which is not LF counting. The
  file pre-filter changes results: `globals() [k] = v` (with a space) in a file with no other trigger is not
  reported. An AST port would be cleaner, but it would not mean the same thing.

### unforgeable_grants (hard-fail, 473 lines)
- **Computes:** four syntactic findings in internal code. UG002: any call whose callee name ends with
  `register_application_key`, anywhere in internal production code. The other three apply only to `*_grants.py`
  files. UG001: `from … import register_application_key`. UG003: `CallsiteToken.from_base64(…, key_id=<not
  None>)`. UG004: a bare-name target of `=` or `x: T = …` that matches `(^|_)(PUBLIC_KEY_HEX|PUBLIC_KEY_CHECKSUM|KEY_ID)$`.
  UG000 is the signature failure.
- **Inputs:** the Python AST of files under `api/`, `impl/` and top-level `l\d{4}_.+/`, excluding tests
  (`lint_unforgeable_grants.py:127-128,158-165,385-388`). The signed in-script allowlist (`:121-125`) is empty.
- **Needs:**
  - `EXISTS doc, path_part, file_name, match`: the internal-production and grant-file tests on the path.
  - `EXISTS import_alias`: UG001 matches the imported name x, not the local name (`:193-194`). It covers
    `from`-imports only, and any nesting.
  - `LIB call_name_tail(c: node, s: symbol)`: the last identifier of the callee, which is the Name's text or the
    attribute's name, at any depth. `_call_name` falls back to the attribute name when the base is not a
    Name/Attribute chain (`:168-174`). So "ends with register_application_key" (`:209`) is
    `call_name_tail(c,s), match(".*register_application_key", s)`. Body:
    `field(c,"function",f), (kind(f,"identifier"), text(f,s) ; attr_of(f,_,s))`.
  - `LIB call_dotted(c: node, s: symbol)`: the callee's dotted text as `_call_name` builds it (`:168-174`). A
    prefix that is not a Name is dropped, so `g().CallsiteToken.from_base64` gives `CallsiteToken.from_base64`.
    The body is recursive over `attr_of`, with the fallback above. UG003 tests
    `s = "CallsiteToken.from_base64"` (`:219`). It is purely textual and not resolved: `sym_of` would change the
    meaning.
  - `EXISTS call_arg`: `key_id=` whose value is not the `None` literal (`:220-221`). `kind(v,"none")`.
  - `EXISTS kind, field, match`: UG004 targets. Each `=` target that is an identifier, including every target of
    `a = b = …`, and a bare-name target of `x: T = …` (`:232-251`). Tuple targets, augmented assignments, walrus
    and `for` targets are excluded. This holds at any scope.
  - `NATIVE signed_list format` and `POLICY signature-failure finding`, as for dynamic_module_loading
    (`:347-357`).
- **Hard parts:** the patterns are names, not resolved symbols. `foo.my_register_application_key()` is reported.
  `CallsiteToken` imported under an alias is not. The source triggers (`:132-139`) never change the result: each
  finding needs one of the trigger strings.

### permission_context_system_flag (hard-fail, 891 lines)
- **Computes:**
  - PSF001: in permission "target" files, any call passing `is_system_session=`, unless the callee is exactly the
    name `ResolvedSessionContext`.
  - PSF004: in target files and generic-lowering files, calls of `may_filter_query_rows` /
    `system_session_applies_sql_row_filter`, matched by terminal name. Also defs with those names, in target files
    only.
  - PSF003: in generic-lowering files, `isinstance(_, <…PermissionScheme | Scheme imported from l4000_permissions>)`
    inside a function body.
  - PSF002: in generic-lowering files, calls of `is_system_session` / `is_schema_session` /
    `is_system_or_schema_session` in a function. The function must be connected to a "permission policy sink",
    or the call must sit in an `if` test whose body returns `None`.
  - PSF000 is the signature failure. PSF999 (syntax error) is reported on the CLI path only.
- **Inputs:** the Python AST. Scan dirs are `api`, `impl`, `l2050_transaction`, `l2070_batching` and
  `l4000_permissions`, excluding tests (`:823-832`). Target prefixes are matched after a path is canonicalised
  with an `api/` prefix (`:132-140,199-203`). Generic prefixes are at `:142-152,206-208`. The signed in-script
  allowlist (`:116-120`) is empty.
- **Needs:**
  - `EXISTS doc, match`: the target and generic path classes. The `api/` canonicalisation is one alternative of
    the regex: `(api/)?l4000_permissions/.*`.
  - `EXISTS call_arg`: the `is_system_session=` keyword. The callee exemption compares the full dotted text
    (`:516-518`), so `x.ResolvedSessionContext(...)` is still reported. This needs `LIB call_dotted` (above).
  - `LIB call_name_tail` (above), as `_terminal_name` (`:220-222`). It is used by PSF002, PSF003 (`isinstance`)
    and PSF004 (`:499-503`), and by the call graph.
  - `LIB own_body_node(f: node, n: node)`: n is in f's body and does not cross a nested def (`:225-248`). That
    covers the body statements only, not f's decorators, parameters or return annotation. A nested def and its
    decorators are excluded. Lambdas and class bodies are included, but methods are not. Body:
    `field(f,"body",b), (n = b ; within_stop(n, b, nested_def))`, where `nested_def` holds `function_definition`
    and `decorated_definition` nodes. A tree-sitter decorator sits outside the function_definition node, so a
    stop set of the kind `function_definition` alone would leak nested decorators.
  - `LIB expr_name(n: node, x: symbol)`: an identifier that Python's ast would report as `ast.Name` (load, store or
    del). This is `nameref` minus parameter identifiers, import-alias identifiers and global/nonlocal names
    (`:328-333`). `nameref` keeps parameters and import names, which `ast.Name` is not.
  - `LIB policy_sink(f: node)`: the disjunction at `:284-336`, over `own_body_node(f, n)`. It holds when any of
    these appears in f's own body:
    - `attr_of(n,_,"BYPASS")`;
    - a keyword `permission_grammar=None`;
    - an `=` / `x: T = v` target named `permission_grammar` (Name, or the Attribute's name) with value `None`;
    - `del …permission_grammar`;
    - `setattr(_, "permission_grammar", None)`;
    - an `expr_name` or attribute name in {`SessionUserIdStep`, `filter_rows`, `permission_grammar`,
      `permission_state`, `requires_user_id`, `user_id_ref`, `user_resolution`};
    - a string constant equal to one of those names.
  - `LIB policy_connected(f: node)`: the fixpoint at `:377-392`. It is a connected component of an undirected,
    same-file, name-based call graph that contains a sink. Body:
    `policy_connected(f) :- policy_sink(f).`
    `policy_connected(g) :- policy_connected(f), name_edge(f,g).`
    `name_edge` is symmetric. `name_edge(f,g)` holds when f's own body has a call c with `call_name_tail(c,t)`
    and `def_name(g,t)` in the same document. This is unresolved, and every def with that name counts (methods,
    nested functions). `calls`/`call_target` are resolved and directed, so they do not mean this.
  - `LIB if_arm(i: node, test: node, body: node)`: `if_statement` and each `elif_clause` with its own condition and
    consequence. Python's ast represents `elif` as a nested `If` (`:343-360`).
  - `LIB no_policy_return(r: node)`: `return None`, or `return` of a non-empty tuple that is all `None`. Bare
    `return` does not count (`:276-281`). The tuple may be `tuple`, `expression_list` or a parenthesised `none` in
    tree-sitter.
  - `EXISTS within`: the shortcut. An identity call within `test`, and a `no_policy_return` anywhere within
    `body`, including nested defs (the oracle uses `ast.walk`, `:353-357`).
  - `EXISTS import_alias, text, match`: Scheme names imported from a module whose raw `from` text contains
    `l4000_permissions`, whose original name ends with `Scheme`, bound to the local name (`:489-496,422-428`). The
    type argument is a Name, an Attribute's name, or a recursive `tuple`, from positional argument 1, with at
    least 2 positional arguments. `GraphInheritedPermissionScheme` is excluded (`:251-273`).
  - `NATIVE signed_list format` and `POLICY signature-failure finding` (`:780-790`). The plugin passes the list
    unsorted; the list is empty.
  - `POLICY extraction-failure findings`: a syntax error gives PSF999 on the CLI path (`:610-612`). In plugin mode,
    SyntaxError is silent and other extraction errors are FACT-EXTRACTION-FAILED (`lint_lib/fact_engine.py:1337-1350`).
    Tree-sitter recovers from errors, so the port needs a `parse_failed(d)` fact to skip such files (approximable
    as `kind(_, "ERROR")`).
- **Hard parts:** PSF002 is gated by a connected component, not by reachability. Once a function is in, every
  function it calls by name is in too, and so are their callers. Matching is by bare name across all defs in the
  file. "Own body" differs between the two ASTs: decorators, `elif`, parenthesised `None`, and parameters vs
  `ast.Name`. The file source triggers (`:176-186`) can hide PSF003. A file whose only Scheme check is
  `isinstance(x, RbacScheme)`, with RbacScheme imported from l4000_permissions, contains no trigger string and is
  not scanned. Findings are deduplicated on (line, code, snippet) (`:616-629`). The PSF003 line is the
  `isinstance` call's line.

### permission_test_classification (hard-fail, 756 lines)
- **Computes:**
  - Classifies every directly collectable `test_*` function under `conform/` that touches the database into one of
    six categories, through an ordered if/elif chain.
  - PTC001 compares the result with the committed JSONL inventory: the records must be equal, the header counts
    must be equal, and the matrix count must be at least the stored minimum. It also checks a "principal
    conversion surface" JSON.
  - PTC002 validates `scripts/permission_surface_manifest.json` against the public operations it finds by
    statically resolving `data_layer/*.py` exports and class methods.
- **Inputs:**
  - Python AST of `conform/**` (tests included, `:713-717`) and of `data_layer/*.py` plus every module they import
    or inherit from (`permission_surface_manifest.py:71-80`).
  - JSONL `data_history/permission_matrix/conformance_test_classification.jsonl`, with a header line then one
    record per test.
  - JSON `data_history/permission_matrix/principal_conversion_surface_2026-09-03.json` (`.tests[].path/function`).
  - JSON `scripts/permission_surface_manifest.json`.
  - File and directory existence for paths in the manifest. `:81-82`; `permission_surface_manifest.py:228-234`.
- **Needs:**
  - `EXISTS kind, nearest, def_name, match, decorator`: a test is "directly collectable" (`:398-406`) when:
    - its parent is the module, or a class whose name starts with `Test` (at any depth);
    - its name matches `test_.*`;
    - it has no `@pytest.fixture` / `@pytest.fixture(…)`, matched as text, not resolved (`:200-210`).
  - `LIB named_param(f: node, x: symbol)`: positional-only, positional and keyword-only parameter names. It
    excludes `*args`/`**kwargs` (`:183-185`). `param`/`params` include splat parameters through their first
    identifier child.
  - `LIB receiver_root(e: node, x: symbol)`: the base Name reached by stepping through Attribute.value,
    Call.func and Subscript.value (`:188-197`). Recursive.
  - `EXISTS within, nearest, attr_of, match`: the call facts per test (`:435-452`). A call belongs to its
    nearest-first enclosing scanned test (ancestors are nearest-first, `lint_lib/ast_collectors.py:42-50`), so
    calls in nested helpers count. The facts are:
    - `called_methods` (attribute calls only);
    - `has_data_operation`: a boundary method, or a data method whose receiver root contains txn, transaction, conn
      or session, case-insensitively (`(?i)`);
    - `has_system_call`: `SessionId.system(...)`.
  - `EXISTS rules with !, count`: the category chain (`:325-385`). Each branch negates the ones before it.
    "lifecycle only" is `count : { called(t,m), !lifecycle(m) } = 0` with a call present. The path tables
    (`:224-313`) are lowercase substring tests on `path` and `path::function`: `match("(?i).*…", …)`.
  - `EXISTS kind, field, within`: a module-level `pytestmark = …` containing an attribute `pytest.mark.system_session`
    or `pytest.mark.permission_specific`. Matched as text (`:414-433`).
  - `NATIVE json(path: symbol, ptr: symbol, kind: symbol, value: symbol)`: a generic JSON / JSON-Lines reader
    (one row per scalar, keyed by JSON pointer, a line index first for JSONL). Inventory, conversion surface and
    manifest all need it (`:552-560,569-597`; `permission_surface_manifest.py:240`). `.input r(IO=file, …)` does
    not say how nested objects and arrays map to columns, which the conversion surface's `tests[]` list and the
    manifest's `groups[].operations[]` require.
  - `EXISTS count, !` over `json` rows: inventory freshness (`:616-639`). It is a set difference both ways between
    the derived records (path, function, line, category, reason, cases) and the stored ones. The header's
    `database_tests` and `category_counts` are compared with `count : {…}` per category. `cases` is a list,
    compared by index.
  - `POLICY ratchet floor from a committed file`: `current matrix_count >= stored minimum_matrix_tests`
    (`:624-630`). The manifest's `completed_contract_floor` is the same (`permission_surface_manifest.py:282-286`).
    A plain `.budget` is a ceiling on findings, not a floor on a derived count.
  - `POLICY summary findings`: one finding stands for a whole set. Examples: "inventory differs" (`:633-636`), and
    "conversion surface is incomplete: N test(s) … (first five)" (`:604-612`). Each is located at a data file, not
    a node, with line = the finding's ordinal in the list (`:674-683`). `.violation` takes `at: node`, so a
    finding located at a non-Python file path is needed.
  - `LANG ordered preview`: the message lists the first five keys in file order and the total count. Rules plus
    `count` give N, but not "the first five in input order, joined". Example:
    `.violation incomplete(at: path, n: number, preview: symbol)`;
    `incomplete(p, n, s) :- n = count : { gap(_, _) }, s = join(", ", 5, k : gap(i, k) order by i)`. Under the
    loose findings contract this only affects the message; the finding key can drop it.
  - `EXISTS json rows + rules`: the conversion-surface special cases (`:581-587`): one renamed key and two retired
    keys, written as facts.
  - `LIB catalog_name(m: symbol, x: symbol, b: node)`: the manifest's own name table (`permission_surface_manifest.py:92-134`).
    It covers top-level statements, recursing only into `if` body/orelse and `try` body/handlers. It holds
    def/class, from-import (relative, with its own package rule), `import`, `=` and `x: T` bindings. The last
    binding in statement order wins (`max` over start offsets). It differs from `exports`/`public_name`: those
    skip assignments and require `import_time`, but this table keeps TYPE_CHECKING imports.
  - `LIB catalog_export(m: symbol, x: symbol)`: from a literal `__all__` (string elements), else names without
    a leading `_` that are not bound by a plain `import` (`:136-149`). `dunder_all` covers the first half only.
  - `LIB catalog_resolve(m: symbol, x: symbol, def: node)`: follows import tuples and `X = Y` Name aliases, with
    cycle protection (`:151-165`). Recursive. Unreachable cycles give no row.
  - `LIB catalog_methods(c: node, x: symbol)`: public (no `_`) methods of a class body, plus those of bases. A base
    resolves through `catalog_resolve`: `Base[T]` uses its value, and `a.b.C` goes through a plain-`import`
    binding (`:167-201`). `mro_method`/`class_base` use `sym_of`, which does not handle subscripted bases and
    takes all ancestors.
  - `EXISTS rules`: `discover_operations` (`:204-225`) is `data_layer.<stem>.<x>` for each `data_layer/*.py` not
    starting with `_`, plus `.<method>` for classes.
  - `EXISTS json rows + rules`: the manifest checks (`:244-289`): duplicate ids and ownership, disposition and
    scenario rules, canonical scenario order (by index), coverage-status rules, owner prefix, unowned and stale
    operations.
  - `LIB path_exists(p: symbol)`: a file, or a directory (some file whose path starts with `p/`), via `file`,
    `substr` and `cat`. Plus a `..` / absolute-path refusal (`:228-234`). Core `file` holds files only.
  - `LIB manifest_invalid(msg)` plus negation: schema validation, i.e. pydantic `extra="forbid"`, required
    fields, regex patterns, Literal enums, `min_length` (`:36-59`). Discovery errors count too: mutated `__all__`
    (`+=` or an `__all__.x(…)` call at those statement levels) and a non-literal `__all__`. Any of these gives a
    single finding, and every other manifest finding is suppressed (`:239-243`). That is expressible as
    `!manifest_invalid(_)` guards. The oracle's message is pydantic's text, which cannot match.
  - `POLICY fact scope wider than finding scope`: findings sit on two data files, but facts come from `conform/**`
    and from `data_layer/*.py` and its transitive imports, which lie outside `conform/` (`:713-717`;
    `permission_surface_manifest.py:71-80`).
  - `EXISTS file`: "manifest facts are missing" when `scripts/permission_surface_manifest.py` is absent
    (`:692-695`).
- **Hard parts:**
  - This is a snapshot check: the committed inventory is generated by `--write`. Equality is exact, and includes
    `line` numbers, so any edit above a test changes it. The stored record order is compared too, though both
    sides are sorted.
  - `matrix_case_errors` (`:598-602`) cannot fire: matrix records always get the seven cases.
  - Path tests run on the path string the engine passes in. Segments like `/benchmarks/` would match a
    directory above the repository if that path were absolute.
  - Collectability is the oracle's own approximation of pytest's rules, not pytest's.
  - The manifest's name table is order-sensitive: last binding wins. It does not follow `else`/`finally` of `try`,
    and it treats a binding `X = Name` as an alias to resolve.

### permission_test_expansion (hard-fail, 699 lines)
- **Computes:** It reads every `@permission_expansion(ids=…, contract=…, proofs=…)` decorator on test functions under `conform/`
  and checks each one against a table, held in the script, of about 150 requirement IDs (`Q2.count`, `G1.repair`, …).
  Each ID has a contract and a required proof set. It reports malformed decorators, unknown IDs, requirements that no
  test claims, requirements that more than one test claims, contract mismatches, a missing `read_permission_scenario`
  parameter, and missing proofs.
- **Inputs:** Python AST of `conform/**/*.py`, with tests included (`lint_permission_test_expansion.py:656`). The
  requirement table is code in the script (`:109-441`).
- **Needs:**
  - `EXISTS` inline facts plus `cat`: `req(cat("Q2.", c), "query-read-results") :- q2_case(c).`, and proof sets as unions
    (`proof("RESULT", p) :- proof("READ", p).`).
  - `EXISTS decorator, attr_of, call_arg, child, str, params, scope_node, def_name`: the decorator is found by
    its name (a Name or an Attribute's attr), and must be a call with keywords only. Its keyword set must be exactly
    {ids, contract, proofs}, written `count : { call_arg(c, -1, k, _), !meta_kw(k) } = 0`. The tuple/list/set
    elements must all be plain strings, with at least one. `**kw` is not counted, as in the oracle (`:489`). A
    positional `*xs` does count as positional (`:486`): `spread` plus `child` covers it.
  - `EXISTS` negation and `count`: an unknown ID (`:600`), an unclaimed requirement (`:607`), more than one claim
    (`:610`), a contract mismatch (`:613`), the fixture missing from the parameters (`:617`), and a missing proof
    (`:621`, `count : { rproof(id, p), !cproof(c, p) } > 0`).
  - `LANG` findings that belong to no syntax node. "`{id}: missing behavioral matrix`" and "claimed by more than one"
    are about a requirement ID from the policy's own table (`:608`, `:611`). The oracle reports them at the path of
    the first claim, or at `conform`, with line = the rank of the finding in the sorted list (`:644-645`). A
    `.violation` today needs `at: node`. Rules plus aggregates can derive the rows, but cannot give them a location.
    ```
    .violation PTE_missing(at: policy, id: symbol) "{id}: missing behavioral matrix"
    PTE_missing(id) :- req(id, _), !claimed(id, _).
    ```
  - `LANG` an ordered string aggregate for messages: "`is missing proofs: a, b`" joins the missing proofs, sorted, into
    one finding per (requirement, claim) (`:621-624`). Writing one finding per proof would change the number of
    findings, not just the text.
    ```
    missing_proofs(id, c, s) :- claim_of(id, c), s = cat ", " : { rproof(id, p), !cproof(c, p) } by p.
    ```
- **Hard parts:** A claim that lists the same ID twice is counted twice in `claims_by_id`. That raises "claimed by
  more than one" and duplicates the per-claim findings (`:599-603`), so set semantics lose the duplicate. To keep it,
  key on the element index. The qualname is `Class::name` only when the function's *direct* parent is a class
  (`:524`). In tree-sitter the parent is `decorated_definition` → `block` → `class_definition`, so use
  `scope_node(f, s), kind(s, "class_definition")`. `fixtures` comes from posonly, args and kwonly only (`:479`).
  `params` also names `*args`/`**kw`, which matters only for a parameter spelled `*read_permission_scenario`. An
  f-string element makes the oracle report "must be literal", because a JoinedStr is not a Constant. `str`
  must therefore have no row for f-strings, or the rule must test `kind`.

### provider_storage_permissions (hard-fail, 213 lines)
- **Computes:** It checks the four classes in a catalog held in the script (two files): each must be a top-level class
  decorated `@register_table(...)` and `@register_permissions(scheme_factory=SystemOnlyScheme)`. Codes: PVP001, a
  table class in a catalogued file that is not in the catalog; PVP002, no scheme; PVP003, the wrong scheme; PVP004,
  the class is missing or not a table (reported at line 1).
- **Inputs:** Python AST of `l4250_timers/timer.py` and `l5200_vector_index/quantized_projection_models.py`
  (`lint_provider_storage_permissions.py:37-46`, `:209`). The catalog is code in the script.
- **Needs:**
  - `EXISTS` inline catalog facts `catalog(path, class, "system-private")`, with `.scope include` on the two paths.
  - `EXISTS decorator, attr_of, call_arg, def_name, modnode`: top-level classes are `child(m, c, _)` of the module,
    or through a `decorated_definition`. The decorator call is matched by its name (Name or Attribute attr). The
    `scheme_factory` value counts only when it is a Name or Attribute. PVP004 is anchored at `modnode(d, m)` (line 1).
  - `EXISTS` `child` index order picks the *first* `register_permissions` decorator (`:58-61`, the loop returns the
    first match).
- **Hard parts:** `classes = {node.name: node}` keeps the *last* top-level class of a given name (`:81`). A
  `scheme_factory=SystemOnlyScheme()` call is not a Name or Attribute, so the oracle reports PVP002 (no
  registration), not PVP003 (`:49-54`, `:70`). A class nested in an `if` is not a top-level class. If a catalogued
  file does not exist, the oracle produces no PVP004, because the file is never in scope (`:209`).

### no_api_root_imports (hard-fail, 289 lines)
- **Computes:** It reports `from api import …` statements (including the relative `from .api import …`, whose
  `module` is also "api"), one finding per statement. It also reports every string constant whose decoded value has
  a line `[ \t]*from api import <something>` or `[ \t]*import api`, one finding per regex match. It does not report
  the code statement `import api`.
- **Inputs:** Python AST of `data_layer_production_dirs`: `impl`, `data_layer`, `_internal`, `l####_*`, with tests
  excluded (`lint_no_api_root_imports.py:243-244`).
- **Needs:**
  - `EXISTS field, child, text`: `field(i, "module_name", mn)`, whose dotted part reads "api", whether or not a
    `relative_import` prefix comes before it. `module_text` cannot be used, because it resolves relative imports to
    `pkg.api`.
  - `EXISTS text(modnode)`, `contains`: the source-trigger gate. The extractor runs only on files whose text contains
    "from api import" or "import api" (`:79`, `fact_engine.py:1177-1189`). The rule can test this on the module text.
  - `NATIVE` `str_const(n: node, v: symbol)`: every node that Python's ast reports as a str `Constant`, decoded. That
    is plain (non-bytes) strings, implicit concatenations (`"a" "b"`, one value at the first piece's
    line), and the literal parts of f-strings (`:121-123`). `value`/`str` are defined on one `string` node.
    The port's ledger records two false negatives here: an import split across adjacent literals, and two imports
    separated by an escaped `\n`. Decoding is literal parsing, which Datalog cannot do.
  - `LANG` a match offset on a symbol capture. Lines are `Constant.lineno + newlines in the *decoded* value before
    the match` (`:127-132`). `match_all` over the decoded symbol gives the right *number* of matches, but a capture
    on a symbol has no position. With an offset, the line is plain arithmetic:
    `strlen(p) - strlen(@replace_all(p, "\n", ""))` on `p = substr(v, 0, o)`.
    ```
    hit(n, o) :- str_const(n, v), match_all("(?:^|\n)[ \t]*(?<m>from api import [^\n]+)", v), o = m.start.
    at_line(n, o, l + strlen(p) - strlen(@replace_all(p, "\n", ""))) :- hit(n, o), line(n, l), str_const(n, v), p = substr(v, 0, o).
    ```
  - `LANG` (message only) an ordered string aggregate: the message lists the imported names in order, joined with
    ", " (`:112`). The same need as in permission_test_expansion.
- **Hard parts:** The trigger gate means `from .api import X` is reported only when the file also contains one of
  the literal trigger strings (`:79`). Any `str` Constant counts, docstrings included; bytes do not. Each of the two
  regexes reports separately, so a value that matches both gives two findings (`:126`). The oracle ignores files
  that fail to parse; tree-sitter recovers and would report them unless the rule excludes documents with `ERROR`
  nodes.

### data_layer_facade_purity (hard-fail, 315 lines)
- **Computes:** It checks each top-level statement of six named facade files. FAC001 is a def or class. FAC003 is an
  import of `impl.*`, one finding per alias for `import`. FAC004 is an assignment that does not have Name targets
  and a "simple" value (a constant, a name, an attribute chain over simple values, or a list/tuple/set/dict of
  simple values). FAC002 is any other statement except string expression statements.
- **Inputs:** Python AST of `data_layer/{_file_locking,_mime_sniffing,rippling,versioned_merge,versioned_rebase,
  versioned_squash}.py` (`lint_data_layer_facade_purity.py:223-230`).
- **Needs:**
  - `EXISTS stmt, child, kind, import_alias, field, text`: `stmt(st, m)` for module statements. FAC003 tests the
    *written* module text, prefix "impl." (`:181`, `:186`): `from impl import x` is not reported, but
    `from .impl.x import y` is.
  - `LIB` `nonsimple(n: node)`: the complement of `_is_allowed_value` (`:101-112`), written as *positive* recursion,
    so it stratifies. `nonsimple(n) :- kind(n, k), !simple_kind(k).`
    `nonsimple(n) :- kind(n, k), container(k), child(n, c, _), nonsimple(c).`
    `nonsimple(a) :- attr_of(a, o, _), nonsimple(o).` Here `parenthesized_expression` is transparent. A direct
    `simple(n)` defined with `count : { child(n, c, _), !simple(c) } = 0` would be recursion through negation.
  - `LIB` `assign_chain(a: node, t: node, v: node)`: tree-sitter nests `x = y = v` as assignment-in-`right`. Python
    has one Assign with several targets (`:115-126`). The relation is recursive along `right`, and gives every
    target t and the final value v. compat_aliases uses it too.
- **Hard parts:** `_is_docstring_expr` skips *every* top-level string expression statement, not only the first
  (`:94-98`, `:177`). An f-string statement is a JoinedStr, so the oracle reports it as FAC002. Grammar mapping:
  tree-sitter makes `comment` nodes children of the module, and the rule must skip them. A decorated def is a
  `decorated_definition`, and the oracle's line is the `def` line, not the decorator's. `x: int` with no value is
  allowed (value None). `-1` (UnaryOp), starred elements and f-strings are not simple. `**d` in a dict literal is
  allowed, because its key is None (`:111`). `x += …` is FAC002, not FAC004. The source-trigger gate (`:73`) skips
  a file containing none of "def ", "class ", "impl.", "=", "(", which is almost never the case.

### data_layer_compat_aliases (hard-fail, 361 lines)
- **Computes:** It reports module-level aliases in public facades, statements under `if`/`try` included but nothing
  inside def or class bodies. DLA001 is a public `as` name that differs from the imported name (for `import a.b as c`,
  from its last segment). DLA002 is an assignment of a Name or Attribute to a public target whose name differs from
  the value's leaf name.
- **Inputs:** Python AST of `data_layer/**/*.py` (recursive) and `testing.py` at the root, tests excluded
  (`lint_data_layer_compat_aliases.py:290-294`). The candidate port leaves out `testing.py`.
- **Needs:**
  - `EXISTS import_alias, split_dot, nearest(…, scope)`: module level is `scope_node(n, m), kind(m, "module")`.
    `import x as y` has no dot, so a rule `!contains(".", q)` supplies the last segment for `split_dot`.
  - `EXISTS attr_of, text`: the value's leaf name is a Name's id or an Attribute's attr (`:118-123`).
  - `LIB assign_chain` (shared with data_layer_facade_purity): every Name target of `A = B = mod.X` is checked
    against the final value (`:180-189`).
- **Hard parts:** The oracle emits one finding per alias, all at the statement's line (`:153-160`, `:165-172`). Under
  set semantics the alias node must be the anchor, or same-line findings collapse. An annotated assignment with no
  value is skipped (`:192`). A tuple target is skipped silently: no finding.

### connection_caches (hard-fail, 366 lines)
- **Computes:** A line-oriented regex scan, comment lines excluded, for four cache patterns on `self._*` / module
  `_*` attributes keyed by BranchId/RevisionId or named like tip/branch caches. There is one finding per (line,
  pattern), and an allowlist of (file-name suffix, substring in the line).
- **Inputs:** Source *text* (not the AST) of `api/**`, `impl/**`, skipping parts named `tests`, `__pycache__`,
  `.pytest_cache` (`lint_connection_caches.py:197-202`, `:244`).
- **Needs:**
  - `LIB` `src_line(d: number, l: span)`: one span per physical line of document d.
    `src_line(d, l) :- modnode(d, m), match_all("(?m)^(?<l>.*)$", m).` The span carries its `line` and `text`, so
    `match("[ \t]*#.*", l)` drops comment lines (`:248-250`). Each pattern's `search` is `match(".*pat.*", l)`.
    Pattern 3's `^` is anchored after the indentation (`:93`, applied to the stripped line).
  - `EXISTS file_name / doc, contains`: the allowlist is `file_rel.endswith(f)`, a string suffix that also matches
    `xstaging_bypass.py`, combined with `substr in line` (`:205-210`).
- **Hard parts:** The scan covers lines inside docstrings and strings as well; `src_line` does too. Python's
  `splitlines` also splits on `\x0b \x0c \x1c-\x1e \x85    ` (`:247`), which `(?m)$` does not. If the
  module node does not start at byte 0, the lines before it are only leading comments or blank lines, which the
  check ignores anyway. The source-trigger gate (`:112`) is implied by the patterns.

### schema_introspection_data_paths (hard-fail, 612 lines)
- **Computes:** Within the "data path" packages, it reports calls whose callee leaf name is in a forbidden set
  (`table_exists`, `inspect`, `get_table_columns`, …) and str constants containing SQL markers (`information_schema`,
  `sqlite_master`, …). Both are skipped when any enclosing def's name contains an authority marker (`create`,
  `schema`, `verify`, …), and three (path, innermost def, callee) primitives are exempt. It also reports
  `flush_schema_plan*` calls under `impl/common/l7245_versioned_backends/`, even inside authority functions. Findings
  in a signed approval list (`path:line:name`) are dropped.
- **Inputs:** Python AST of 16 path prefixes (`lint_schema_introspection_data_paths.py:98-115`), minus paths whose
  `"/" + rel` contains any of 14 markers (`:117-132`, `:235-240`). The signed list is a Python constant in the script
  (`:84-88`), empty today.
- **Needs:**
  - `EXISTS doc, contains, match`: scope by prefix and marker substring.
  - `EXISTS field, attr_of, text`: the callee leaf name (`:243-248`). `nearest(c, f, fdef)` + `def_name` give the
    innermost def for the primitive key (`:265-271`).
  - `LIB` `in_def(n: node, f: node)`: n is inside def f *or in its decorators*. The oracle pushes the name before it
    visits the decorator list (`:289-296`); tree-sitter puts decorators outside `function_definition`.
    `in_def(n, f) :- within(n, f), kind(f, "function_definition").`
    `in_def(n, f) :- field(dd, "definition", f), within(n, dd).`
    Authority is `in_def(n, f), def_name(f, x), match("(?i).*(bootstrap|create|…).*", x)` (`:261-262`).
  - `NATIVE str_const` (shared): str Constants include f-string literal parts and joined implicit concatenations
    (`:371-391`). The marker test is on the lowercased decoded value: `match("(?is).*information_schema.*", v)`.
  - `EXISTS signed_list`: approved keys are `path:line:name`, built with `cat`.
  - `POLICY` validating the allowlist: an entry under `impl/`, `scripts/` or `*/tests/*` is an error even when signed.
    Any such error, or a bad signature, *replaces* every finding with SIDP000 findings (`:486-513`). Negation can
    suppress SIDP001 (`!list_error()`), but the SIDP000 finding is anchored at the linter script, line 1, not at a
    document. It needs the same non-node anchor as the LANG need above, here the list file.
- **Hard parts:** The docstring skip covers every *expression statement* whose value is a str Constant, not only
  the first (`:257-258`, `:325-335`). An f-string statement is not skipped, but its literal parts are scanned. Each
  marker in a string gives its own finding, with `call_name` = the marker. The function stack ignores classes and
  lambdas (`:289-323`). The flush check runs before the authority return (`:343-356`).

### auto_api_route_descriptions (hard-fail, 285 lines)
- **Computes:** It reports each function decorated with `@<x>.get/post/put/patch/delete/options/head/trace/api_route(…)`
  (an attribute call) that has neither a non-blank string `summary=` or `description=` keyword on such a decorator
  nor a non-blank docstring. The message is prefixed by a qualname made of the enclosing classes only.
- **Inputs:** Python AST of `l9900_auto_api/**`, tests excluded (`lint_auto_api_route_descriptions.py:255`).
- **Needs:**
  - `EXISTS decorator, attr_of, call_arg, str, match("(?s).*\\S.*", v)` for "non-blank after strip"
    (`:113-120`).
  - `LIB` `docstring(f: node, s: node)`: the first body statement is an expression statement holding a string (plain
    or concatenated, not an f-string). This generalises `module_doc` to defs and classes.
    `docstring(f, s) :- field(f, "body", b), child(b, st, 0), kind(st, "expression_statement"), child(st, s, 0), kind(s, "string").`
    (`:134-135`)
  - `LIB` `class_path(f: node, q: symbol)`: the dotted names of the enclosing classes only (`:101-106`). `qualname`
    adds the module and the enclosing functions, so it does not fit. The relation is recursive over
    `nearest(…, "class_definition")`.
- **Hard parts:** `ast.get_docstring` cleans indentation (`inspect.cleandoc`) before `.strip()`, which only matters
  for blankness. A bare-name decorator `@get(…)` is not a route (`:110`). The source-trigger gate (`:83`) needs the
  literal text `.get(` and similar in the file. A route decorator with a space before `(` defeats the gate, so the
  file is not checked.

## Summary table

Each row is one distinct need, merged across names: `src_line`/`source_line`; `call_name`/`call_name_tail`;
`ast_name`/`expr_name`; "path-anchored", "document-less" and "summary" findings. Plain `.mode block` / zero-baseline
`.budget` entries are left out, because they already exist.

| tag | signature or construct | checks that need it |
|---|---|---|
| LANG | A finding anchor that is not a syntax node: a policy fact (requirement ID), a non-Python or missing file path, or the linter/allowlist file itself. Today `.violation` takes `at: node` only | permission_test_expansion, permission_test_classification, plan_optimizer_v2, schema_introspection_data_paths, dynamic_module_loading, unforgeable_grants, permission_context_system_flag, no_blanket_grammar_bypass (CLI only) |
| NATIVE | `src_line(d, l, s: span)`: physical source lines, numbered as Python's `str.splitlines()` numbers them. A LIB form, `match_all("(?m)^(?<s>.*)$", module)`, is exact except in files containing `\v \f \x1c-\x1e \x85    ` | no_blanket_grammar_bypass, backend_type_strings, type_safety_patterns, no_source_inspection, dynamic_module_loading, connection_caches |
| LANG | An ordered string aggregate, `s = cat ", " : { … } by k`: sorted or source-ordered joins in messages and in ratchet identities. The variant "first k in order, plus a count" is a preview. type_safety_patterns can use it or `line_range` | broad_type_surfaces, broad_type_surfaces_ratchet, permission_test_expansion, permission_test_classification, no_api_root_imports, type_safety_patterns |
| NATIVE | `signed_list` must verify this repository's scheme: Ed25519 over canonical JSON `{allowlist: sorted, linter, version}`, key `PUBLIC_KEY_HEX`, with the `NEEDS_SIGNING`/`UNSIGNED`/`""` sentinels. The lists live today as constants inside the scripts and must move into files | dynamic_module_loading, unforgeable_grants, permission_context_system_flag, schema_introspection_data_paths, bootstrap_cache_health |
| POLICY | A signature failure or an invalid allowlist entry (e.g. `impl/`, `scripts/`, `tests/` entries) becomes a finding (XX000) that *replaces* every other finding, or fails the check before scanning. Core's "refused" defines no finding | dynamic_module_loading, unforgeable_grants, permission_context_system_flag, schema_introspection_data_paths, bootstrap_cache_health |
| POLICY | Finding identity is the chosen columns: one finding per match, per alias or per operator form, and dedupe on (file, line, category) or (code, line, message). Set semantics give this only when the distinguishing field is a column or the anchor | backend_type_strings, classvar_check, type_safety_patterns, expression_owner_propagation, data_layer_compat_aliases |
| POLICY | Parse and extraction failures: files that CPython cannot parse give no AST findings (tree-sitter recovers, so a rule needs a `parse_failed(d)` fact or a test for `ERROR` nodes). Other extractor errors become `FACT-EXTRACTION-FAILED`/BCH000/PSF999 at line 1. Unreadable files are skipped silently | every AST plugin check (shared section); explicit in bootstrap_cache_health, permission_context_system_flag, plan_optimizer_v2 |
| POLICY | The scope that supplies facts is wider than the scope findings land on: imported modules and manifest sources outside the reported files | broad_type_surfaces, broad_type_surfaces_ratchet, permission_test_classification |
| LIB | `call_name(c, s)`: the callee's last identifier (a Name's id or an Attribute's attr, with subscripts stripped) | expression_owner_propagation, unforgeable_grants, permission_context_system_flag |
| NATIVE | `str_const(n, v)`: every node that Python's ast calls a str `Constant`, decoded. That covers plain strings, implicit concatenations (joined, at the first piece) and f-string literal parts, and excludes bytes | no_api_root_imports, schema_introspection_data_paths |
| LIB | `call_dotted(c, s)`: textual dotted callee as `_call_name` builds it, unresolved | unforgeable_grants, permission_context_system_flag |
| LIB | `ast_name(n, x)`: identifiers Python's ast reports as `ast.Name` (`nameref` minus parameter, import, global/nonlocal and keyword-pattern names) | plan_optimizer_v2, permission_context_system_flag |
| LIB | `assign_chain(a, t, v)`: every target and the final value of a tree-sitter-nested `x = y = v` | data_layer_facade_purity, data_layer_compat_aliases |
| NATIVE | `signed_jsonl(path, line, key, value)`, or `signed_list` extended to JSON lines | broad_type_surfaces, broad_type_surfaces_ratchet |
| NATIVE | `unparse(n, s)`: CPython `ast.unparse` text, for allowlist keys | broad_type_surfaces, broad_type_surfaces_ratchet |
| NATIVE | Grammar support for PEP 696 type-parameter defaults (`class C[T: X = X]`); unverified in the bundled tree-sitter-python | broad_type_surfaces, broad_type_surfaces_ratchet |
| LIB | `type_alias(d, name, alias, value)`, `alias_param`, `alias_ref`, `expanded_name`: recursive alias expansion across from-imports | broad_type_surfaces, broad_type_surfaces_ratchet |
| LIB | `class_field(d, cls, field, ann)`, `ctor_param(d, cls, param, ann)` across imports | broad_type_surfaces, broad_type_surfaces_ratchet |
| LIB | `nominal_text`, `explicit_contract`, `unbranded_text`, `for_elem_type`: BTS016 value flow | broad_type_surfaces, broad_type_surfaces_ratchet |
| LIB | `class_func_path(n, s)`: enclosing class names, then function names, joined | broad_type_surfaces, broad_type_surfaces_ratchet |
| LIB | `owner_path(n, s)`: true nesting path of the innermost def or class, else `<module>` | broad_type_surfaces_ratchet |
| LIB | `class_path(f, q)`: the enclosing class names only | auto_api_route_descriptions |
| POLICY | `.budget` by code from an external, optionally signed baseline; a missing code has ceiling 0; `total` is ratcheted as well | broad_type_surfaces |
| POLICY | Per-identity budget `(path, owner, code, surface, detail)` from a signed file that must exist; each key stands alone | broad_type_surfaces_ratchet |
| POLICY | Codes that no budget can admit (BTS008–014, BTS016) | broad_type_surfaces_ratchet |
| POLICY | Metric enforcement: findings are reported as non-blocking info; output that is not JSON becomes `INVENTORY_FAILED` | broad_type_surfaces |
| POLICY | A ratchet floor read from a committed file: a derived count must not drop (`minimum_matrix_tests`, `completed_contract_floor`) | permission_test_classification |
| POLICY | Runtime behaviour probes: the oracle imports and runs project code; keep it as a project test, or read a probe-result file | plan_composition_architecture |
| NATIVE | `op_at(n, i, s)`: operator tokens in order (core's `op` is a set) | classvar_check |
| LIB | `cmp_link(c, i, op, r)`: one link of a chained comparison, over `op_at` | classvar_check |
| NATIVE | `end_line(n, l)`: a node's last line (`end_lineno`) | type_safety_patterns |
| NATIVE | `line_range(d, first, last, s: span)`: one span over whole lines; or the ordered string aggregate | type_safety_patterns |
| NATIVE (optional) | `col(n, c)`: column, only if several matches on one line must stay distinct | backend_type_strings |
| LANG | Recursion through `max`/`min` over a well-founded source order ("latest earlier alias"). Soufflé rejects it as unstratified | expression_owner_propagation |
| LIB | `ast_expr_child`, `src_code`, `carrier` (fixpoint), `carrier_ref`, `expr_input` | expression_owner_propagation |
| LANG | A capture's offset within a symbol (`m.start`), for lines inside decoded string values | no_api_root_imports |
| NATIVE | `doc_lines(d, k)`: a file's line count | plan_optimizer_v2 |
| NATIVE | `repo_text(path, s)`: raw text of a repository file that is not code (a markdown doc) | plan_optimizer_v2 |
| NATIVE | `json(path, ptr, kind, value)`: a generic JSON / JSON-Lines reader keyed by JSON pointer | permission_test_classification |
| LIB | `in_docstring(d, l)`, `tc_guard_line(d, l)`: line-state approximations | dynamic_module_loading |
| LIB | `own_body_node(f, n)` (the stop set includes `decorated_definition`), `policy_sink`, `policy_connected`, `if_arm`, `no_policy_return` | permission_context_system_flag |
| LIB | `named_param(f, x)`: parameters without `*args`/`**kw` | permission_test_classification (permission_test_expansion only in an edge case) |
| LIB | `receiver_root`, `catalog_name/export/resolve/methods`, `path_exists`, `manifest_invalid` | permission_test_classification |
| LIB | `nonsimple(n)`: the complement of "simple literal value", as positive recursion | data_layer_facade_purity |
| LIB | `in_def(n, f)`: inside def f or its decorators | schema_introspection_data_paths |
| LIB | `docstring(f, s)`: `module_doc` generalised to defs and classes | auto_api_route_descriptions |
