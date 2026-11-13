# Group 7: what `.rr` lacks for 13 data-layer checks

Oracle: `scripts/` at 2b1504ad. Citations are `file:LINE` in that tree. `facts.py` is short for
`scripts/conditional_architecture_facts.py`, and `strict_allowlist.py` and `source_lines.py` are in `scripts/lint_lib/`.
"Production dirs" means `data_layer_production_dirs` (`lint_lib/discovery.py:32`): `api`, `impl`, `data_layer`,
`_internal` and every top-level `lNNNN_*`. The `.scope dirs` line of plan-scope-side-effects.rr already expresses it.

Several needs come up in more than one check. Each is described in full once and named after that:

- **N-SRC**: the raw text of a whole file as a span. Some checks read raw lines: comments, blank lines, docstring
  parity, `# noqa`, line counts. The tape gives node text, and it is not certain that the module node covers leading
  and trailing trivia. So `NATIVE source(d: number, sp: span)` gives one span per document covering every byte, and
  `LIB src_line(d, l, s)` is built from it (`match_all("(?m)^(?<L>.*)$", sp)`, then `line(L, l)` and `text(L, s)`).
  This is IO, not AST, so it is native.
- **N-PATHLOC**: a finding located at a repository path and line rather than at a node. This is for a file that is
  not a parsed document (Markdown, allowlist TSV, budget JSON, a directory), or one that may not exist.
- **N-DYNCODE**: a violation whose code comes from a column, such as `SEC123` or
  `transitive:non-lazy-call:*.execute()`, not from the fixed `.violation` name.
- **N-SIGNED**: signed lists in formats that `signed_list` does not cover. The cases are a detached signature over the
  sorted entries, the TSV fragment directory, and per-entry signatures.

---

### conditional_architecture (hard-fail, 426 lines)
- **Computes:** 1. Tripwires inside `l2070_batching/conditional_step.py`: CONDA-001 and CONDA-002 for imports from
  `impl.common.l2100_sql_grammar`/`impl.common.l2070_server_plan` or of `Sql*` names; inside `ConditionalStep.compile`,
  calls to `compile`, `Sql*` or `CompiledStep(grammar=…)`; and a missing `compile`. CONDA-003 for `cs|conditional|
  conditional_step.compile()` in the dialect test file. CONDA-004 when the names `_DDL_TAGS`/`_UNGATABLE_TAG` are absent,
  or `__init__` has fewer than 2 `_validate_branch_content` calls. 2. CONDA-005: cross-file "typed contract" checks on
  4 canonical owners (`facts.py:35-39`), kept only when the message contains one of 3 markers.
- **Inputs:** Python AST of 5 explicit files: `l2070_batching/conditional_step.py`, `impl/common/l2070_server_plan/steps/
  {_conditional_compile.py, sql_conditional.py, tests/test_conditional_compile_dialects.py}`,
  `impl/common/l2070_server_plan/sql_backend_executor.py`. Tests are included.
- **Needs:**
  - EXISTS `doc`, `kind`, `field`, `def_name`, `within_stop`/`nearest` with a scope stop: "innermost class is
    ConditionalStep and innermost def is compile" (`lint_conditional_architecture.py:285-289`). Also `count`
    (CONDA-004), and `text` + `match` on raw import module text and alias names. `.scope include` covers the 5 files.
  - EXISTS `class_def`, `class_base`, `subclass` for the conditional-class closure (`facts.py:509-523`), and `params` plus
    an annotation via `sym_of` for `parameters["step"] == SqlConditionalStep` (`facts.py:599`). The `min` aggregate
    covers "validation call line < first strategy call line" (`facts.py:600-612`).
  - LIB `always_raises(b: node, q: symbol)`: block b reaches a `raise q` before any `return`, on every path. Body: go
    through children by `child` index. The decisive child is a raise of TypeError, an `if` with an `else` (elif chains
    unrolled from `elif_clause`/`else_clause`) whose branches all `always_raises`, or a `match` whose cases all do. Stop
    at an earlier `raise` of another type, or a `return`. Recursive and stratified (`facts.py:99-115`).
  - N-PATHLOC: when a canonical owner is absent, the finding goes to a fixed path at line 1, even if that file does not
    exist (`facts.py:562`, `583`, `614`, `634`). The four checks run only if one owner exists (`architecture_present`,
    `facts.py:549-552`). The nullary rule is EXISTS; the location is not.
  - POLICY: one code (CONDA-005) whose findings are filtered by message substring
    (`lint_conditional_architecture.py:364-379`). In `.rr` this is just "do not declare the dropped rules". Six messages
    survive: four "typed conditional contract", "must reject direct compilation" and "must define
    _lower_conditional_step". Callback-field, state-write, backend-control, child-compile and level checks are computed
    and dropped (`facts.py:660-741`), so they must not be ported.
- **Hard parts:** `_contains_backend_selection` is true if any attribute `backend_type|is_sql_backend|dialect` and any
  `if/ifexp/match` appear anywhere in the function. They need not be related; the `if` may not enclose it
  (`facts.py:126-133`). Methods are keyed by the simple class name, and a nested def's facts are looked up by its own
  name in the enclosing class's method table. Aliases resolve only the head segment of a dotted name
  (`facts.py:79-84`), weaker than `sym_of`. The raw import test uses `module.startswith(prefix)` with no dot boundary.
  A `from x import SqlA, SqlB` gives identical keys (same code, line, message): multiplicity is lost under a set of
  keys. Source triggers gate the collector on whole-file substrings (`lint_conditional_architecture.py:82-88`), which is
  always satisfied in these files.

### admin_tools (ratchet, 673 lines)
- **Computes:** Anti-patterns in `api/l8000_admin/**` and `conform/l8000_admin/**`. R1: calls of `.nonportably_execute_sql`,
  `._fetchall` and `._fetchone`. R3: `except`/`except Exception`/`except BaseException` whose only statement is `pass`
  or `...`. Four line-regex rules: R2-FSTRING-SQL, R2-RAW-SQL-STRING (warning), R5-UNTYPED-FILTER, R4-BACKEND-SWITCH.
  In `test_*` files, T1/T2: test functions without meaningful assertions. The ratchet counts `{errors, warnings}`.
- **Inputs:** Python AST and raw source lines of the files above, minus 6 excluded file names (`lint_admin_tools.py:446-453`).
  Signed baseline `.ratchet-baselines/lint_admin_tools.json` (`{"errors": 3, "warnings": 0}`).
- **Needs:**
  - EXISTS `attr_of`/`callee` (R1), `except_clause` children with comments counted as trivial, as swallowed-exceptions.rr
    does, and `text` of the type name (R3, by spelling, not resolved). `def_name` + `match("test_.*")`, `decorator`,
    `within` and `kind` cover T1. `file_name` covers the exclusions.
  - N-SRC + LIB `src_line`: R2, R4 and R5 are per-line regexes over raw text, with per-line skips (`# noqa`, `assert `,
    `raise `, ` failed:`/` error:`, leading `#`) (`lint_admin_tools.py:207-232`, `302-319`, `335-366`). The R3 noqa test
    reads the except line's text (`:169-173`).
  - LIB `doc_state(d, l)`: two different docstring line-parity state machines. Raw SQL toggles on lines with exactly one
    triple quote and skips lines with two or more of one kind (`:239-253`). Backend switching toggles on any line that
    contains `"""` or `'''` (`:334-342`). Body: parity of `count : { src_line(d, j, s), j < l, toggles(s) }`, using `%`.
    The previous-1-to-3-lines test is `src_line(d, l-k, _)` (`:267-273`).
  - POLICY: a budget keyed by severity group, not by code. `errors` is R1, R2-FSTRING, R3, R4, R5; `warnings` is R2-RAW,
    T1, T2 (`:650-666`). `.budget * by code` cannot name a group of codes. Example:
    `.budget {R1, R2_FSTRING, R3, R4, R5} as "errors" 3`.
- **Hard parts:** T2 can never fire. `has_meaningful_assert` and `only_checks_success` are always set together
  (`:403-440`). T1 skips only `@x.fixture`/`@x.fixture(...)`, not a bare `@fixture`, and checks only sync `def`. Its
  `ast.walk` includes nested functions. The suite plugin runs T1/T2 on every `test_*` file in both directories
  (`:513-520`), but the CLI that produces the ratchet counts runs them only under `conform/` (`:611-618`): two different
  counts. The extractor's source triggers (`:66-79`) skip a file with none of `except`, `assert`, `SELECT`, … so a test
  file with no `assert` at all and no other trigger yields no T1. Reproducing that needs `match` on N-SRC. R2/R4 regexes
  use `re.IGNORECASE` and run on raw lines, so they also fire inside strings and comments that are not at the line start.

### admin_async_purity (hard-fail, 338 lines)
- **Computes:** AAP001 for each line in `l8000_admin/tools/**/*.py` matching `\basyncio\.to_thread\b`,
  `\brun_in_executor\b` or `\bdisable_sync_in_async_check\b`. A line with several patterns gives one finding per
  pattern. Lines inside a docstring, lines that contain a triple quote, and lines starting with `#` are skipped.
- **Inputs:** raw source of `l8000_admin/tools/**/*.py`. Paths whose relative path matches `/tests/`, `test_` (anywhere,
  so `latest_x.py` is skipped) or `__pycache__` are excluded. A signed, empty allowlist sits in the script
  (`lint_admin_async_purity.py:91-96`).
- **Needs:**
  - N-SRC + LIB `src_line`, and `doc_state` as above with parity `count('"""') + count("'''")` odd. The current line is
    included in the toggle, and any line with triple quotes is skipped (`:211-217`). The count of occurrences in a line
    is `count : { match_all("\"\"\"|'''", L) }`.
  - EXISTS `doc(d, p, _), !match(".*(/tests/|test_|__pycache__).*", p)` for the skips (`:112-131`).
  - N-SIGNED: the allowlist is a list constant in the script, signed by a detached signature constant over the sorted
    entries (`compute_allowlist_message`, `lint_lib/signing.py:65`). The CLI exits 1 when the signature fails (`:307-312`).
    `signed_list` reads a list file with its own signature. The list is empty today, so this can be dropped.
- **Hard parts:** the message is `"{pattern}: {line.rstrip()}"` (`:264`). Two patterns on one line are two findings at the
  same line, differing only by message. The candidate JSON used a `lines.state` parity automaton, which the
  `src_line` + count formulation replaces.

### noqa_sec_ratchet (ratchet, 276 lines)
- **Computes:** a warning per SEC code on every line matching `#\s*noqa:\s*(?:\w+,\s*)*SEC\d+`. The codes are all
  `SEC\d+` on the line, anywhere, including outside the noqa directive. The scan covers every Python file outside
  skipped directories. The ratchet counts the total.
- **Inputs:** raw source of all discovered Python files, minus any path with a part in `tests, benchmarks, conform,
  __pycache__, .git, tmp, node_modules, .venv, .tox, data_layer_testing`, and minus `scripts/` (`:83-109`). Signed
  baseline `.ratchet-baselines/lint_noqa_sec_ratchet.json` (`noqa_sec_annotations: 0`).
- **Needs:**
  - N-SRC + LIB `src_line`. Then `match(".*#\\s*noqa:\\s*(\\w+,\\s*)*SEC\\d+.*", L)` and `match_all("(?<c>SEC\\d+)", L)`
    give one span per occurrence, so duplicates stay distinct (`:133-141`).
  - N-DYNCODE: the finding code is the SEC code itself (`:175-176`). For example:
    `.violation noqa_sec(at: span, sec: symbol) code {sec} "{sec} suppression annotation: {line}"`.
  - EXISTS `.scope exclude` + `path_part`; `.budget * { "total": 0 }`.
- **Hard parts:** the count has multiplicity. `# noqa: SEC001 ... SEC001` counts 2, so findings must be anchored at the
  code's span, not the line. The match runs on the raw line, so a `# noqa: SEC…` inside a string literal counts too.
  Comment nodes alone are not enough.

### grammar_mutation_invalidation (hard-fail, 303 lines)
- **Computes:** MUTATION_WITHOUT_INVALIDATION on each method of a "memoizable" grammar class that mutates a private
  `self._x` and never calls `forget_memoized_render_fragments`. A class is memoizable if its name is in a list of 15, or
  it has a direct `render` method with a decorator whose text ends with `memoize_in_read_only_scope`. Construction
  methods (`__init__`, `__new__`, `__post_init__`) and construction-only helpers are exempt.
- **Inputs:** Python AST under `impl/common/l2100_sql_grammar/`, excluding `/tests/`.
- **Needs:**
  - LIB `self_private_attr(e: node, a: symbol)`: e is `self._a`, or a subscript of one, recursively (`:84-89`).
    Then `mutates(f, a, at)`: the target of an assignment, augmented assignment or `del` (direct targets only, no tuple
    unpacking), or a receiver of `.add/.append/…/.update` (`:92-114`).
  - LIB `self_call(f: node, m: symbol)`: `self.m(...)` anywhere in f, nested defs included (`:128-136`). Then two
    recursive closures over the class's methods. `ctor_reach`: from the construction methods, through private (`_`)
    callees only. `rt_reach`: from public non-construction methods, through any callee. The result is
    `construction_only = ctor_reach − rt_reach`, restricted to `_`-prefixed names (`:148-170`). Plain recursion and
    negation; LIB because it is a reusable "method closure" relation.
  - EXISTS `decorator` + `text` + `match(".*memoize_in_read_only_scope", t)` (`:139-145`). EXISTS `callee` by Name or
    attribute text for the invalidation call (`:117-125`).
- **Hard parts:** the methods table is `{name: def}` over the class body, so a later def of the same name (property
  setter) replaces the earlier one. That needs "last def of that name in the class", by `max` start offset. The
  message's `mutated[0]` is the first attribute in `ast.walk` order, which is breadth-first (`:222`). Python-AST depth
  differs from tree-sitter depth: an `Assign` sits one level above the `Call` of `x.append()` in Python, but at the same
  level in tree-sitter. Source-order `min` gives a different first attribute in mixed cases. This affects the message
  only. `ast.unparse(decorator)` of `@memoize_in_read_only_scope()` ends in `()`, so a called decorator does not count.

### declared_fact_interrogation (hard-fail, 321 lines)
- **Computes:** CONTRACT_ATTR_PROBE for `getattr`/`hasattr` (bare names) whose second positional argument is a string
  literal in an 18-name contract set. OVERRIDE_IDENTITY_PROBE for comparisons whose operators are all `is`/`is not`,
  where `type(x).a` is compared with `Other.a` (same attribute, x not `self`).
- **Inputs:** Python AST of production dirs, tests excluded.
- **Needs:**
  - EXISTS `call_arg(c, 1, "", v)`, `str`, a fact set of names, `callee` + `text` (`:137-147`).
  - EXISTS for the comparison: `kind(c, "comparison_operator")`, operands by `child(c, n, i)`, and "every operator is
    is/is not" as `!op(c, o)` for o outside `{"is","not"}` with no `"in"` token. `cmp` (names.rr) is not usable because it
    drops chains, and the oracle accepts chains (`:161`). The left operand is paired with every comparator, not with
    adjacent ones (`:163-179`). Use `min` over comparator index for the reported attribute.
- **Hard parts:** `getattr` is matched by spelling, so a shadowed `getattr` still counts. `len(node.args) < 2` counts
  starred arguments. In Python's AST, implicit concatenation `"de" "fines"` folds into one Constant. `str` must
  fold `concatenated_string` too, and must reject `b"…"` and f-strings, which are not `str` Constants.

### publication_registry_writes (hard-fail, 271 lines)
- **Computes:** REGISTRY_DIRECT_WRITE for `x.cte_refs[k] = / += / : T =`, `del x.cte_refs[k]`, and
  `x.cte_refs.clear|pop|popitem|setdefault|update(...)`, outside a file whose path ends with
  `l2070_batching/compilation.py`.
- **Inputs:** Python AST of production dirs, tests excluded.
- **Needs:**
  - EXISTS everything: `kind` of `assignment`/`augmented_assignment`/`delete_statement`, `field(…,"left",s)`,
    `kind(s,"subscript")`, `attr_of(o, _, "cte_refs")` tests, `callee` + `attr_of`. The owner test is
    `match(".*l2070_batching/compilation\\.py", p)`, a suffix test with no directory boundary (`:117-119`).
- **Hard parts:** only direct targets count. `a, x.cte_refs[k] = …` (tuple target), `x.cte_refs[k][j] = …` and
  `x.cte_refs = {}` are not reported (`:176-186`). One finding per statement even with several matching targets. The
  `del` targets of `del a, b` are an `expression_list` in tree-sitter, separate targets in Python.

### unknown_contract_defaults (hard-fail, 297 lines)
- **Computes:** UNKNOWN_DEFAULTED for `x.f or <empty>`, where f is in `{auxiliary_query, relation_role, source_tables}`
  and `<empty>` is `()`, `[]`, `{}`, `""`, or `dict()/frozenset()/list()/set()/tuple()` with no arguments. It looks at
  adjacent operands of one `or` chain. UNKNOWN_AS_FALSY for `not x.f` and `bool(x.f)` (exactly one positional argument).
- **Inputs:** Python AST of production dirs, tests excluded.
- **Needs:**
  - LIB `or_operand(chain: node, i: number, v: node)`: the flattened operands of one `or` chain. Python's `BoolOp` holds
    `a or b or c` as one node with three values (`:193-199`). Tree-sitter nests left-associative `boolean_operator`s, so
    the pair (b, c) spans two nodes. Body: recurse into the `left` child while it is a `boolean_operator` with op `or`
    and not parenthesized. The chain root is the outermost such node; the finding is at its line. Then
    `or_operand(t,i,l), or_operand(t,i+1,r)`.
  - EXISTS empty-value tests (`:120-131`) with `kind` + child count + `str(n, "")` + bare-name `callee`
    spelling, and `not_operator` and `call_arg` for the falsy forms (`:206-217`).
- **Hard parts:** one finding per BoolOp, for the first adjacent pair that matches. `(a or b) or c` stays nested in
  Python too, so parentheses must stop the flattening. `b""` is not `== ""`. Without receiver typing, any receiver
  counts.

### regex_sql_parsing (ratchet, 620 lines)
- **Computes:** a warning, whose code is one of 9 categories, per `re.search|sub|match|findall|fullmatch|split|compile(p, …)`
  whose first argument is a string literal or f-string, and whose pattern text is classified as SQL parsing. The
  classifier takes the first match: a false-positive list returns no finding, error-message indicators give
  ERROR_MSG_PARSE, a required SQL-syntax indicator set comes next, then an ordered category cascade with func-name
  fallbacks.
- **Inputs:** Python AST of production dirs, tests excluded. Signed baseline `.ratchet-baselines/lint_regex_sql_parsing.json`
  (`total: 0`).
- **Needs:**
  - EXISTS: the receiver `re` matched by spelling (`:387-396`), `call_arg(c, 0, "", a)`, `value(a, s)` for decoded string
    literals, raw prefixes honoured (`:321-322`), and `match` with `(?i)` for each indicator list. The ordered cascade
    (`:243-316`) is a chain of rules, each negating the earlier categories (stratified `!`). It is verbose, but rules can
    say it.
  - NATIVE `fstring_part(s: node, i: number, v: symbol)`: part i of an f-string. v is the decoded literal text for a
    literal part (escapes, raw prefix and `{{`/`}}` applied) or the slot marker for an interpolation. Format specs and
    nested strings are excluded. Native because decoding string escapes is lexing, which the tape does not do
    (`:323-331`). Then LIB `fstring_template(s, t)`, folding parts in index order with `cat` (recursive on i).
  - NATIVE functor `@py_repr(s: symbol): symbol` and EXISTS `@slice`/`strlen` for the message
    `re.X() with SQL pattern: {preview!r}` (`:417-424`). Message only.
  - EXISTS `.budget * { "total": 0 }` (`:579-613`; the per-category counts are informational).
- **Hard parts:** the pattern indicators include `(?<!%)%s` (`:145`, `:267`). The Rust `regex` crate has no lookbehind,
  so the regexes must be rewritten (`(^|[^%])%s`, as the candidate did). `pattern_str.upper()` followed by a
  case-sensitive regex equals `(?i)` only for ASCII. Many indicators look for literal backslashes, such as `\\b` in the
  decoded pattern, so double-escaping must be exact. Implicit concatenation `"a" "b"` is one Constant, and literal-plus-f
  concatenation is one JoinedStr in Python. `value` and the template must fold `concatenated_string` the same way.

### file_length (hard-fail, 417 lines)
- **Computes:** FILE_LENGTH at line 1 of each Python file under `api`, `impl` or `conform` (excluding
  `__pycache__/.pytest_cache/node_modules/.git/.hypothesis`) with more than 2000 lines, unless the allowlist gives it a
  limit it does not exceed. Messages differ for new and grown files.
- **Inputs:** raw source (line count) and `.file-length-allowlist.json` (`{path: max_lines}`), whose signature
  constant sits in the script (`lint_file_length.py:113-115`), over entries `"path:count"` (`:118-123`).
- **Needs:**
  - N-SRC + LIB `doc_lines(d, k)`: k = `count : { match_all("\n", sp) }`, plus 1 if the file is non-empty and does not
    end in `\n` (plugin, `:189`). The CLI uses `splitlines()`, which also breaks on `\r`, `\x0b`, `\x0c`, `\x1c-\x1e`,
    `\x85`, ` ` and ` ` (`:147`), so the two counts can differ.
  - EXISTS `.input allow(IO=file, filename=".file-length-allowlist.json")` for a JSON object read as (key, value) rows,
    `to_number`, `option("threshold")`, and anchoring at `modnode`.
  - N-SIGNED: a detached signature over `sorted("path:count")` entries (`:316-324`). Only the CLI verifies it: the
    plugin reporter loads the allowlist unsigned (`:195-196`).
- **Hard parts:** the plugin declares `enforcement="warning"` and severity warning (`:229`, `:220`), while the inventory
  says hard-fail and the CLI `--check` exits 1. The `.mode` must follow one of them. The shrunk and stale entries are
  CLI hints, not findings.

### markdown_docs (ratchet, 612 lines)
- **Computes:** for each git-tracked `*.md`: MISSING (error) if it is tracked but absent from the working tree. Otherwise
  UNREFERENCED (warning) unless one of these holds: the name upper-cased starts with `README` and ends with `.MD`; the
  file name is `CLAUDE.md` (any directory); the first path component is `.claude`, `.cursor` or `.github`; or the path
  or bare name is referenced from a non-`.md` tracked text file or from `docs/design/SERVERPLAN_ARCHITECTURE.md`. The
  ratchet has two metrics: `total_markdown_files` (ratcheted Markdown files) and `unreferenced`.
- **Inputs:** git index (`git ls-files`) and working tree, content of every tracked non-binary text file
  (`git grep -I -n -F .md`), and `.ratchet-baselines/lint_markdown_docs.json` (`total_markdown_files: 360, unreferenced: 0`).
- **Needs:**
  - NATIVE `git_tracked(path: symbol)`, kept apart from `file(path)` for files present on disk. MISSING is "tracked but
    not on disk" (`lint_markdown_docs.py:204-213`, `432-443`). `file` alone cannot express the gap.
  - NATIVE `grep_line(path: symbol, needle: symbol, l: number, s: symbol)`: each line of a tracked non-binary file
    containing the fixed string `needle` (`:285-317`). These files are not parsed documents. Reading every file's whole
    text as symbols would intern about 9k files of text, so the needle prefilter belongs in the reader.
  - LIB `md_ref(s, cand)`: tokens from `match_all("\\S+", s)` after `\`→`/`, stripped of `'"\`()[]{}<>,:;`, cut after the
    first `.md`, with leading `./` characters removed (`:337-349`). Then every suffix after a `/`, `-` or `_` at index
    > 0, plus each one's basename (`:352-362`). That is a recursive capture over the remaining suffix; the recursion
    must allow a separator at the suffix's first character. EXISTS-composable with `match`/`@replace_all`, but it is a
    library relation.
  - N-PATHLOC: findings are at `<md path>:1`, and Markdown files are not documents (`:456-464`).
  - POLICY: a ratchet over a derived metric that is not a finding count. `total_markdown_files` counts tracked `.md`
    files minus README, CLAUDE.md, config dirs and 7 linter-doc prefixes (`:239-246`, `:470-475`). Example:
    `.output ratcheted_md` plus `.budget count(ratcheted_md) 360`. `.budget` today covers violations only.
- **Hard parts:** the linter's own script is a non-`.md` tracked file and names `CLAUDE.md` and
  `SERVERPLAN_ARCHITECTURE.md`, so it justifies them itself. The git-grep path and the file-reading fallback differ on
  binary files (`-I`). `.md` detection is case-sensitive, while README detection is not. `unreferenced` also counts
  MISSING findings. The plugin runs the whole analysis once, anchored on `scripts/lint_markdown_docs.py`; in `.rr` it is
  simply a project-level rule.

### eager_resolve (hard-fail, 2666 lines)
- **Computes:** "sites" of eager LazyResult resolution, each a (pattern_name, symbol) at a line:
  1. Text-line regexes: `.resolve()`, `.resolve_async()`, `.result()`, `int|float|bool(lazy…`. They skip blank and comment
     lines and lines covered by multi-line string tokens. The symbol is the deepest enclosing def by line range.
  2. `preprocess:*` sites in functions named `preprocess`.
  3. `lower:/compile:/preprocess:*` sites in functions reached forward from a def named preprocess, lower or compile,
     over the per-file call graph (phase propagation).
  4. `transitive:*` sites. Direct eager patterns (about 20 families: non-lazy DB terminals on txn-like receivers, plan
     scope `.execute()`, bound `.resolve`, forbidden helpers and attributes, `getattr` probes, lazy-python
     `if/for/[]/compare/list()` on lazy locals, SQL-derived permission checks, calls into legacy boundaries) propagate to
     every caller over reverse per-file edges. The site is at each def.
  5. `external-io-boundary:{name}()` definition sites.
  Sites are deduped by `pattern::symbol` (min line). They are reconciled one-to-one against a signed, strict, per-source
  TSV burn-down allowlist. Unapproved sites, invalid external-I/O allowlist entries and allowlist integrity errors are
  findings.
- **Inputs:** Python AST and raw text of `api`, `impl` and every `lNNNN_*`, minus test files: `conform/`, a `tests` part,
  or a `test_*` part (`lint_eager_resolve.py:535-537`). `scripts/allowlists/architecture_grandfathered/eager_resolve/**/*.txt`
  (4 tab-separated fields; empty today), with its signature constant (`:95-96`). The burn-down ceiling
  `scripts/allowlists/architecture_burn_down_ceilings/eager_resolve.txt` and its signature (`:97-98`). The external-I/O
  list `scripts/allowlists/lint_eager_resolve_external_io_boundaries.txt` (`symbol base64sig` per line).
- **Needs:**
  - N-SRC + LIB `src_line` for the text patterns (`:2245-2280`). LIB `end_line(n, l)`: `line(n, l0)` plus
    `count : { match_all("\n", n) }`, needed for "lines covered by a multi-line string" (`source_lines.py:9-24`) and for
    the def line range in `_symbol_for_line` (`:1516-1524`). The deepest scope is `max` depth, then `min` span.
  - LIB `er_symbol(f, s)`: the module from the whole relative path, `/`→`.`, `.py` dropped, `__init__` kept (`:530-532`),
    then the class and def nesting, functions included (`:477-479`). This is not `qualname`: core's module name and the
    handling of `__init__` differ. Built with `@replace_all`/`@dot`.
  - LIB `body_node(f, n)`: n is in f's own body. It excludes nested defs, lambdas and class bodies (`within_stop`), and
    nodes on lines before the first body statement's line (`:407-418`, `480`).
  - LIB `er_edge(f, g)`: a per-file, syntactic call graph. `name()` goes to the module-level def of that name;
    `self.m()` goes to the def m directly in the same class stack; the last def of a name wins (`:625-653`, `577-588`).
    Both closures use it. Forward phases are `phase(f, p)`, recursive (`:599-622`). Backward taint is
    `dirty(c, p) :- er_edge(c, f), dirty(f, p), !skip(c)`, seeded by direct patterns minus symbol filters
    (`:1464-1513`, `1286-1302`).
  - LIB `ordered_alias(f, x, p)`: a name x becomes "lazy-result"/"sql-derived" from position p, when an assignment at p
    has a value classified using only aliases established before p. This is a monotone single forward pass, with no kill
    and keyed by spelling (`:1000-1022`, `1036-1051`). The same pattern builds the module-level legacy decorator aliases
    in event order (`:420-433`). It is recursive over start offsets.
  - LIB for the structural classifiers: `_annotation_mentions_name` (recursive through Subscript, `|` and Tuple,
    `:549-562`), `_contains_sql_derived_expr` (`:1074-1114`), `_receiver_root_name` (`:1131-1140`). The exact wrapper
    shapes (`:1167-1269`) are EXISTS with code patterns `~` plus `match(".*_lazy", …)`.
  - EXISTS: per-pattern names built with `cat` (`transitive:non-lazy-call:*.{attr}()`); `min` line for the
    dedupe (`:1527-1539`); path-prefix and exact-symbol suppressions (`:1542-1584`); the substring gate
    `"external_io_boundary_eager" in content` (`:2285`) via `match` on N-SRC. The ordered seeding of the propagation
    does not change the fixpoint.
  - N-DYNCODE: the finding code is the pattern name (`:2452`), an open set.
  - N-SIGNED. (a) A strict fragment directory: one `*.txt` per source path, tab-separated
    `path, id, pattern, symbol` (`strict_allowlist.py:32-68`), with a detached signature constant over the sorted
    entries (`:2416`). (b) Per-line Ed25519 signatures over `compute_allowlist_message("lint_eager_resolve:external_io_boundary",
    [symbol])` (`:2037-2056`). This needs NATIVE functor `@ed25519_ok(msg: symbol, sig: symbol): number`, with the key
    held by the policy, or a reader that gives `signed_entry(file, line, entry, ok)`. (c) A burn-down ceiling file with
    its own signature, only checked for format and signature (`:1990-1999`). The reader must also give rows for its
    format errors: blank line, field count, non-normalized path, path ≠ fragment, duplicates (`strict_allowlist.py:40-64`).
  - EXISTS stale and unapproved reconciliation as rules: `stale(e) :- allow(_,_,id,p,s), !site(p, pat, s)`. Detected
    payloads are unique after the dedupe, so the multiset matching of `strict_allowlist.py:188-225` reduces to negation.
  - N-PATHLOC: integrity errors go to `ALLOWLIST_DIR` (a directory) at line 1 (`:2470-2478`), and fabricated entries to
    the allowlist file at the entry's line (`:2059-2077`).
  - POLICY: a signature failure, a stale entry or a ceiling error is a hard-fail finding (EAGER-ALLOWLIST) beside the
    normal findings, not a refusal to run (`:2463-2479`).
  - NATIVE functor `@py_unparse(n: node): symbol` (first line) for the transitive sites' `text`
    (`:1961`). It enters only the message, so it can stay out of the key.
- **Hard parts:**
  - Order: the alias sets depend on the pre-order position of assignments (as above). Propagation does not depend on
    order.
  - Python version: `tokenize` in 3.12+ gives f-strings as FSTRING_* tokens, not STRING, so multi-line f-string lines are
    not ignored by `multiline_string_line_numbers`. The rule must exclude only `string` nodes without an f prefix.
  - Text sites: the text sites run on raw lines, so `.resolve()` in a one-line string literal counts. The symbol uses
    line ranges, not offsets, and `lineno` of a decorated def is the `def` line.
  - Call graph: the call graph is per file and by spelling. It does not use `calls.rr` `call_target`; using that instead
    would change the findings.
  - Removed patterns: `_is_allowed_legacy_boundary_bound_resolve` is constantly false (`:1157-1159`), and
    `_extract_eager_lazy_wrapper_signature` removes only the `_resolve_legacy_result_*` pattern of the function itself
    (`:1440-1446`).

### lazy_architecture (hard-fail, 3840 lines)
- **Computes:** 18 rule families (LA-001 to LA-018) over "function facts". The facts are collected per top-level and
  class-level def; defs nested in functions or in module-level `if`/`try` are not collected. Findings come from three
  sources: per-function predicates, cross-file transitive searches over a symbol-string call graph
  (`_transitive_line`, `_transitive_first_*`), and the sync/async pair comparison LA-008. LA-013 and LA-009 also have
  extra per-call findings, and the CLI adds the LA-018 deleted-surface text scan. Findings are deduped by `RULE::symbol`
  and suppressed by a signed allowlist of such keys. Raw per-rule counts are checked against a signed per-rule budget,
  all 0.
- **Inputs:** Python AST and, for the CLI only, raw text of production dirs (tests excluded); paths are cut by many
  hard-coded prefixes and files (`lint_lazy_architecture.py:403-534`). `scripts/allowlists/lint_lazy_architecture_allowlist.txt`
  (8 `LA-xxx::symbol` lines) with a signature constant (`:124-125`). `.ratchet-baselines/lint_lazy_architecture.json`
  (signed per-rule counts).
- **Needs:**
  - LIB `la_fn(f)`: the collected defs only (`:913-955`). LIB `la_symbol(f, s)`, from the same path-derived module as
    `er_symbol` (`:823-825`) plus the class stack only. LIB pseudo-facts for class-body side-queue fields (`:957-990`).
  - LIB `scoped_rhs(r: node, a: node)`: the binding visible at reference r is the latest assignment to the same name
    that precedes r in a block enclosing r's statement. Assignments in sibling branches, loop bodies and even a `with`
    body do not flow out; a nested def sees a copy of the outer bindings (`:1161-1420`). It uses `max` start and
    `within`, with no recursion through the aggregate. The same relation gives `callable_aliases` with a kill on
    reassignment (`:1195-1200`). `flow.rr`'s `before` covers one block only.
  - LIB `la_leaf(n, s)`: the "callee leaf string". It follows a name → rhs chain through `scoped_rhs`, then module-level
    assignments, then the `from` imports of the module body (`:845-905`). It is recursive, with a cycle guard and depth
    caps of 32 and 64, which Datalog recursion handles (it terminates, and only differs past depth 32).
  - LIB `la_callee(c, sym)`: the cross-file target symbol string (`:2144-2174`). It tries scoped imports, gathered
    through `if/for/while/with/try/match` bodies but not `except` types; then module `from` imports; then
    `self.`/`cls.` → `module.Class.attr` through class-body aliases; then `import m` → `m.attr`. Edges are joined to
    `la_symbol` by string equality (`:2409-2427`). `sym_of` is not a substitute: it follows re-exports and core module
    names, and this check does not.
  - LIB for the fact families: `plan_io_calls`, eager sinks over `_looks_like_plan_expr`/`_looks_like_lazy_expr` name
    heuristics (`:2013-2142`), intent/native/grammar calls, dolt strings, LazyResult internals reads, side-queue and
    bootstrap mutations, and reconciliation surfaces. Each is EXISTS-composable (`match`, `attr_of`, `call_arg`,
    `str`) over `la_leaf`; together they are a library.
  - LIB `reaches_pred(f, P)` with a witness line: `_transitive_line` returns the line of the first call, in walk order,
    whose callee has P directly or reaches it (`:2469-2505`). Datalog gives `reach` and a `min` over call offset for the
    line. The first-matching-callee and eager-taint variants also return the symbol, through a chain of chosen edges
    (`:2508-2646`).
  - N-PATHLOC: budget findings go to the JSON at line 1 (`:3566-3575`), and LA-000 to the script or the budget file
    (`:3527-3550`).
  - POLICY: (a) A budget over raw counts. `counts[rule]` counts every appended violation, before the `RULE::symbol`
    dedupe and before the allowlist (`:2658`, `:3555-3564`). A budget over the deduped, unsuppressed set of keys
    undercounts. (b) Signature failure replaces the report. If the allowlist signature fails, the only finding is
    `LA-000` (`:3524-3535`). A budget-file load error gives the same (`:3538-3550`). (c) A signed allowlist of finding
    keys `RULE::symbol` that suppresses findings but not counts. `signed_list` covers the entries; the detached signature
    constant is N-SIGNED.
  - EXISTS: LA-018's "first occurrence of a literal surface in the raw file, line = 1 + newlines before it" (`:3271-3306`),
    with N-SRC + `match_all` + `min`. It is CLI-only: the plugin path never calls `_scan_deleted_cold_bootstrap_surfaces`
    (`:3309-3313` vs `:3521-3580`).
- **Hard parts:**
  - Memoized searches: the transitive searches memoize per file. A node met while still on the stack is memoized as
    `None` (`:2443-2444`, `2484-2486`), and callees on the stack are skipped. On call-graph cycles the oracle can
    therefore miss a reachable target, depending on walk order. Datalog `reach` is order-independent and sound, so expect
    extra findings on cycles. Exact parity would need an ordered depth-first search with a poisoned memo, which no
    `.rr` construct should copy.
  - Function bodies: `_iter_nodes_skip_nested_funcs` skips only `ClassDef` (`:1039-1044`), so lambdas and nested defs
    in expressions count toward the outer function. Nested `def` statements are analyzed with the outer function's
    facts, except route handlers (`:1031-1037`, `:1173-1190`).
  - Name heuristics: many classifiers test name spellings: `endswith("plan")`, `"lazy" in`, `endswith("result")`.
  - Shared machinery: shares nothing with eager_resolve beyond the path-derived symbol scheme and the signing library.

---

## Summary table

| tag | signature or construct | checks that need it |
|---|---|---|
| NATIVE | `source(d, sp: span)`: the whole file's raw text as one span (N-SRC) | admin_tools, admin_async_purity, noqa_sec_ratchet, file_length, eager_resolve, lazy_architecture (LA-018, CLI) |
| LIB | `src_line(d, l, s)` over `source`, via `match_all("(?m)^(?<L>.*)$")` | admin_tools, admin_async_purity, noqa_sec_ratchet, eager_resolve |
| LANG | path-anchored finding: `at` is a (path, line) for a non-document or a missing file (N-PATHLOC) | conditional_architecture, markdown_docs, eager_resolve, lazy_architecture |
| NATIVE | signed-list variants: detached signature constant over sorted entries; strict TSV fragment directory with format-error rows; per-entry Ed25519 (`@ed25519_ok(msg, sig)`) (N-SIGNED) | admin_async_purity, file_length, eager_resolve, lazy_architecture |
| LIB | `ordered_alias` / `scoped_rhs`: the binding of a name by source position (monotone forward pass, or latest assignment in an enclosing block, no merge out of branches) | eager_resolve, lazy_architecture |
| LIB | path-derived dotted symbol (full relative path, `__init__` kept) + nesting; a per-file or cross-file syntactic call graph keyed by spelling; forward and backward closures over it | eager_resolve, lazy_architecture, grammar_mutation_invalidation (`self_call` method closures) |
| LANG | violation code taken from a column (N-DYNCODE) | noqa_sec_ratchet, eager_resolve |
| LIB | `doc_state(d, l)`: line-parity docstring state (two toggle variants) | admin_tools, admin_async_purity |
| POLICY | budget basis: group of codes (severity groups), raw occurrence counts before dedupe or allowlist, or a derived non-finding metric | admin_tools, lazy_architecture, markdown_docs |
| POLICY | signature or allowlist-integrity failure as findings (EAGER-ALLOWLIST, LA-000, which replaces the whole report) | eager_resolve, lazy_architecture |
| LIB | `end_line(n, l)` (start line + newline count in the node's text) | eager_resolve |
| LIB | `always_raises(block, exc)`: every path raises before returning | conditional_architecture |
| LIB | `or_operand(chain, i, v)`: flatten tree-sitter's nested `or` like Python's `BoolOp` | unknown_contract_defaults |
| NATIVE | `fstring_part(s, i, v)` (decoded literal parts and slot markers) + LIB `fstring_template` by recursive `cat` | regex_sql_parsing |
| NATIVE | `git_tracked(path)`, kept apart from `file(path)` | markdown_docs |
| NATIVE | `grep_line(path, needle, l, s)`: lines of tracked non-binary non-Python files containing a fixed string | markdown_docs |
| LIB | `md_ref(s, cand)`: Markdown reference token suffixes | markdown_docs |
| LIB | `self_private_attr`, `mutates(f, a, at)` | grammar_mutation_invalidation |
| NATIVE | message-only functors `@py_repr(s)`, `@py_unparse(n)` | regex_sql_parsing, eager_resolve |
| POLICY | output filtered to the six CONDA-005 messages that pass the marker filter (do not port the dropped families) | conditional_architecture |

Checks that need nothing beyond EXISTS: publication_registry_writes and declared_fact_interrogation (the latter needs
`str` to fold `concatenated_string`; check that core does).
