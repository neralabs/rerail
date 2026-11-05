# What the data-layer checks need, and how the language provides it

## Intro

The data-layer repository enforces its architecture with 142 lint checks, written as Python scripts. They are the
hardest test the `.rr` rule language has: if it can say what these scripts say, with the same findings, it can
replace them. Seven checks were already written by hand. This page asks the question for all 142: what does the
language, its native core or its libraries lack? It then records what was added.

The answer is mostly facts, not syntax. The scripts read things Python gives them for free: CPython's `ast` (where
a decorator belongs, which names are `Name` nodes, the order a visitor walks), its string decoder, the raw lines
of a file, the repository's own files (allowlists, signed baselines, Markdown), and git. Each of those became a
native fact or functor in `lib/core.rr`, or a Datalog relation in a library. Two needs became language:

- `join`, an ordered string aggregate, because nothing else can build the exact text a signature covers;
- `ordered by earlier`, for state where only some writes count.

A few directives were added for how findings are reported: findings at a file with no node, a code taken from a
column, gates, and budgets read from files.

Some needs are outside the language on purpose. Checks that run repository code (pytest collection, import-time
registries, behaviour probes) get those facts from a file a CI step writes. Rerail never runs the code it checks.

Two companion pages say how the engine runs what this page designs:

- [FINDINGS-LAYER.md](FINDINGS-LAYER.md): how rows become findings, which files are Unknown, and what `.gate`,
  `.budget`, `.on parse_error`, `code {c}`, `.mode` and `.severity` do;
- [RUN-FACTS.md](RUN-FACTS.md): the artifacts a CI step writes, how Rerail knows they are for the code being
  checked, and the five checks that need them.

How this was done:

1. Each check was read in the pinned scripts (revision `2b1504ad`) and in its candidate JSON port, if it had one.
   Every need was tagged as already expressible, library, native, language or policy, with `script.py:LINE`
   citations. Eight files, one per group of checks: [checks/group-1.md](checks/group-1.md) …
   [checks/group-8.md](checks/group-8.md).
2. The needs were merged and designed into the core, the libraries and the directives (below).
3. Three checks were written as worked ports that use the new parts, plus one sketch:
   - [file-length.rr](file-length.rr): a JSON allowlist, a signature checked in Datalog, a gate.
   - [noqa-sec-ratchet.rr](noqa-sec-ratchet.rr): a line scan, codes from a column, a ratchet against a signed
     baseline.
   - [test-budget.rr](test-budget.rr): a run artifact (pytest's count), a static bound that works without it, a
     gate. Its signed layout was checked against the real signature too.
   - [versioned-table-names.rr](versioned-table-names.rr): file-wide state in visit order, as plain recursion.
   - [sketches/owner-code.rr](sketches/owner-code.rr): the one shape that needs `ordered by earlier`.
4. Every check was then mapped onto the design: [checks/COVERAGE.md](checks/COVERAGE.md).
5. `site/check_rr.py` checks every `.rr` file for grounded rules, duplicate declarations and, new here,
   stratification. All files pass.

## The two language additions

### 5. `join`: an ordered string aggregate

```
s = join x by k sep "," : { r(k, x) }
```

`s` is every `x` of the rows, ordered by `k` (ties broken by `x`), separated by `","`. It is `""` when there are no
rows. Symbols sort by Unicode code point and numbers by value, as Python's `sorted()` does.

Why it is needed. Soufflé's aggregates are `count`, `sum`, `min`, `max` and `mean`; none builds a string. The
data-layer repository signs its allowlists and baselines. Each signature covers
`json.dumps({...}, sort_keys=True, separators=(",", ":"))` of sorted entries. To check one, a rule must rebuild
those exact bytes from rows, and only an ordered string aggregate can. The same holds where a sorted list is part
of a finding's identity. Most uses the analyses listed were for messages; those drop out, because the findings
contract leaves messages free.

```
// lib/data-layer/signing.rr: the canonical message of an allowlist.
allow_message(l, msg) :- allow_sig(l, linter, _),
    body = join q by e sep "," : { allow_entry(l, e), q = @json_str(e) },
    msg = cat("{\"allowlist\":[", body, "],\"linter\":", @json_str(linter), ",\"version\":1}").
```

Both layouts in `signing.rr` (allowlist and ratchet baseline) were checked against the repository's real
signatures: built as these rules build them, each message verifies. The alternative was a native `signed=` option
on the readers, one per layout. It was rejected: it fixes one repository's scheme in the engine, and a second
repository would need new engine code.

### 6. `ordered by earlier`: recursion through `!` and aggregates

```
.decl src_code(v: node, c: symbol) ordered by earlier
```

Soufflé rejects any relation that depends on itself through `!` or an aggregate. A few checks need exactly that.
A name's state at node n is set by its latest earlier write *that qualifies*, and whether a write qualifies depends
on the state where the write happens. In expression_owner_propagation, a name's code is the code of its latest
assignment whose value has a code. Rebinding to a value without one keeps the old code. The value's code may come
from another name's state at that assignment. "Latest qualifying" is a `max` over the recursive relation.

It is well founded: every step goes to an earlier node. A relation declared `ordered by earlier` may be used under
`!` or in an aggregate in its own recursion when the use's first column `m` is tied to the head's first column `n`
by `earlier(m, n)`, in the same body or aggregate. `site/check_rr.py` enforces the tie: it rejects the sketch if
either the declaration or the `earlier` tie is removed. The engine evaluates such a relation per document in visit
order, so the rows at n depend only on rows already final.

```
// data-layer/sketches/owner-code.rr
src_code(n, c) :- ast_name(n, x), expr_ctx(n, "load"), frame(n, f),
    k = max i : { write(w2, f, x, v2), earlier(v2, n), src_code(v2, _), visit_index(w2, i) },
    visit_index(w, k), write(w, f, x, v), src_code(v, c).
```

**Most state does not need it.** Four analyses asked for ordered recursion. On a closer reading, two of them are
plain positive recursion, which Soufflé already accepts:

- versioned_table_name_manipulation keeps two file-wide maps updated as the visitor walks. Every assignment resets
  the name before it stores, so "the latest write before n" is a `max` over syntax. Only what that write stored
  is recursive, and it is used positively. A def that returns a suffix adds itself without resetting, which is a
  union, also positive. [versioned-table-names.rr](versioned-table-names.rr) is the whole check, and it
  stratifies.
- python/flow's `always_raises` ("every branch of the last `if` raises") was first written with
  `count : { … !always_raises(…) } = 0` and `ordered by within`. "Every branch" is a chain over the `elif`/`else`
  clauses, so it is now positive recursion, and `ordered by within` was dropped from the design.

The rule of thumb, in core.rr: use `ordered by earlier` only when *which* write counts depends on the recursion.
The coverage pass ([checks/COVERAGE.md](checks/COVERAGE.md), "Ordered recursion verdicts") applies it to every
check that claimed the need.

## Native facts and functors (lib/core.rr)

Native means the binary computes it: IO, parsing a format, or a fact the tree-sitter tree does not show. Each entry
says why Datalog cannot derive it.

| Fact or functor | What it gives | Why native |
|---|---|---|
| `source(d, sp)`, `src_line(d, l, sp)`, `line_count(d, k)`, `end_line(n, l)`, `col(n, c)` | the raw text, physical lines (Python's `str.splitlines()` terminators), extents | Comments and blank lines are not in the tree; splitlines has eight terminators |
| `.scope text "**/*.md"` | Markdown, Rust or SQL files read as text, with no tree | No grammar for them |
| `op_at(n, i, s)` | operator tokens in order; `not in`, `is not` as one token | `op` is unordered; chained comparisons need the order |
| `parse_error(d, l, msg)` | CPython rejects the file | Tree-sitter recovers from errors, so the tree cannot show it |
| `earlier(m, n)`, `visit_index(n, i)`, `ast_depth(n, k)` | CPython's `ast` visit order and depth | Decorators and return annotations come after a def's body, defaults after all parameters; rebuilding that from fields needs ancestor pairs |
| `str_part(lit, i, v, at, spec)` | str constants as CPython builds them: escapes decoded, adjacent literals merged, f-string pieces | CPython's decoder |
| `@place(path, line)` | a location with no node: a file, a line of it, a missing file | A finding needs a location |
| `@json_str`, `@sha256`, `@nfkc`, `@count_matches` | JSON escaping as `json.dumps`, digests, identifier normalisation, match counts | Byte-level text functions |
| `@ed25519_ok(key, message, sig)` | signature check | Cryptography |
| `@tree_digest(n)`, `@unparse(n)` | `ast.dump` comparison, `ast.unparse` text | CPython's printer and dumper |
| `git_tracked`, `git_rev`, `git_has` | the git index, the revision under check and its base | Git |
| `.input … IO=json \| yaml \| lines (dir=) \| file`, `path=`, `at="base"`, `input_error` | repository files as rows; a file at the base revision; a row per missing or malformed file | Format parsing and IO |

A fact that only a run of repository code can produce is read with `IO=json` from a file a CI step writes. It is a
declared input of the policy, so a missing file is an `input_error`, never a silent "no findings".

## Library relations

**lib/python/ast.rr** (new) is Python as CPython's `ast` module sees it. The scripts walk `ast`, not tree-sitter,
and the two differ in ways that change findings:

- `ast_name(n, x)`, `expr_ctx(n, c)`: which identifiers are `ast.Name`, and their Load/Store/Del context;
- `py_frame(n, f)`, `def_frame(n, f)`: the def that holds n, with decorators, defaults and annotations inside
  it, as `ast` nests them;
- `sig_param(f, p, nm, i, kind, ann, dflt)`, `param_named`: parameters by kind (positional-only, positional,
  `*args`, keyword-only, `**kw`);
- `type_param(o, p, nm, i, kind, bound, dflt)`, `tparam_named`: PEP 695 type parameters (TypeVar, TypeVarTuple,
  ParamSpec) with bound and PEP 696 default (broad_type_surfaces' BTS008/BTS009);
- `dotted_expr`, `leaf_name`, `call_name`, `call_dotted`, `chain_root`, `base_name`, `from_module_raw`:
  names as the scripts spell them, unresolved;
- `assign_target`, `assign_value`, `assign_ann`, `body_stmt`, `body_len`, `docstring`, `py_if`, `py_orelse`,
  `bool_operand`, `cmp_pair`: statement shapes as `ast` has them (`a = b = v` is one Assign; a chained comparison
  has ordered pairs);
- `fstring`, `string_form`, `str_const`, `ast_constant`: strings and constants;
- `class_path`, `def_chain`, `py_qualname`, `path_module`: qualified names, `<locals>` included;
- `ann_node`, `ann_name`, `ann_head`: names inside annotations, string annotations parsed;
- `latest_assign`: last write before a read, a `max` over visit order;
- `same_file_call`, `name_reach`: calls followed by spelling, as several scripts do.

**lib/data-layer/signing.rr** (new) holds the repository's key, its unsigned placeholders, and its three signed
layouts: allowlist, named multi-allowlist and ratchet baseline. Each layout gives a `*_status` of `valid`, `invalid`
or `unsigned`. The library reports; each check decides what a status means, because the scripts differ. An empty
allowlist is not checked, and a missing baseline passes.

**Changes to existing libraries.**

- imports: `exported(m, x)`.
- flow: `always_raises`.
- names: `type_checking_block` tested by qualified name. Two real cycles that Soufflé would reject were found and
  fixed (see "Bugs found" below).

**What stays in the check.** Decision 0005 admits a library relation only when it is concept-named and used by at
least two rules. Check-specific classifiers stay in the check file. The coverage table marks them LOCAL. They need
nothing new.

## Directives: how findings are reported

The findings contract makes a finding a key: (policy, code, path, line), with path or line absent for
repository- or file-level findings. Order, multiplicity and messages are free. Several needs from the analyses
dropped out under it: messages that list joined names, two findings at one key, and first-witness choice.

| Directive | Meaning | Needed by |
|---|---|---|
| `V(@place("a.json", 1), …) :- …` | a finding at a file and line with no node there; line 0 for the whole file | missing files, allowlist and baseline outcomes, Markdown lines |
| `.violation V(at: span, c: symbol) code {c}` | the code is a column | open code sets: SEC codes, pattern names |
| `.violation V(…, t: symbol) tag {t}` | an extra key field, so findings at one place stay apart | one finding per failing probe |
| `.severity V warning` | per-code severity, for presentation; it never decides blocking | warning-only checks |
| `.mode log CODE` | these codes are reported and never block | findings that only a ratchet enforces |
| `.gate V` | when V has keys they are the report, and the policy's other files are Unknown | bad signatures, missing baselines: the scripts exit before scanning |
| `.on parse_error report CODE`, `.on input_error report CODE` | report those as findings too | scripts that report their own extraction failures |
| `.budget V by code from base [floor] [drop N%]` | findings may exist up to ceilings that are rows of a relation, usually a verified baseline; counts are of distinct keys | ratchets over findings, read from `.ratchet-baselines/*.json` and similar |
| `.artifact "name" inputs …`, `.input … artifact=` | facts from a file a CI step wrote, and the files it was built from | the five checks that run repository code |
| `.group G = "A", "B"` | a named set of codes for budgets | ratchets over category groups |
| `.scope report dirs …`, `.scope text …`, `.scope tests false` | where findings land; text-only documents; test directories | checks that read outside where they report |

A missing or badly signed baseline is an ordinary violation, usually a `.gate`. It is not a special outcome of
`.budget`: the check states the script's own behaviour. In noqa-sec-ratchet, a missing baseline passes and an
unsigned one fails.

A ratchet on a metric (annotations, tests, tracked documents) is a rule that compares the metric with its
ceiling, not a `.budget`. A budget counts finding keys, and a metric counts what the script counts: two SEC codes
on one line are two annotations but one key per code. FINDINGS-LAYER.md, section 6.

## Bugs found on the way

The new stratification check in `site/check_rr.py` found two cycles through negation in the existing Python
libraries. Soufflé would have rejected both:

1. `resolve_export` → `!exports` → … → `resolve_export`. It now negates `exported(m, x)`, which does not depend
   on resolution.
2. `import_time` → `!type_checking_block` → `sym_of` → `resolve_export` → `exports` → `import_time`.
   `type_checking_block` now tests the qualified name (`qn`). The cost is one case: a `TYPE_CHECKING` that is
   re-exported through another module is not recognised.

One analysis misread its script. versioned_table_name_manipulation's f-string exemption returns early when the
number of placeholders is *not* one. So `f"{a}_log"` is reported and `f"{a}{b}_log"` is not, the reverse of what
group-2.md says. The port follows the code.

## Coverage

135 checks were analysed; the other 7 were already written by hand. Each was mapped onto the design in
[checks/COVERAGE.md](checks/COVERAGE.md). Where a check has both a suite plugin and a CLI, the mapping follows the
plugin.

| Status | Checks |
|---|---|
| expressible with the design | 120 |
| expressible, with facts from a CI-written artifact | 5 (linter_enforcement, explicit_semantic_ids, plan_composition_architecture, serverplan_target_architecture, test_budget) |
| approximate | 3 |
| gap found by the mapping | 7, all now designed in (below) |

What the checks use most:

| Element | Checks |
|---|---|
| python/ast.rr relations | 76; `sig_param` alone is 28, `ast_name` 19 |
| check-local helpers | 60 |
| findings at `@place` | 46 |
| source text and lines | 38 |
| signing.rr | 30 |
| `.gate` | 27 |
| `.scope report / text` | 23 |
| file readers | 21 |
| dropped by the findings contract | 20 |
| `.budget … from` (metric ratchets among them are now rules; FINDINGS-LAYER.md §6) | 16 |
| `join` | 16 |
| `parse_error` | 14 |
| `str_part` | 12 |
| visit order | 10 |
| `ordered by earlier` | 2 (plan_helpers_allowlist, expression_owner_propagation) |

The three approximations:
- execution_authority_bypass reads its registry statically, so the port matches only while the registry stays a
  literal.
- serverplan_state_tripwires and legacy_execution_surfaces use a memoised search. It answers "not reached" on some
  paths through cycles. A Datalog fixpoint does reach them, so the ports would report more.

The mapping's gaps, and how each was closed in core.rr:

| Check | Need | Resolution |
|---|---|---|
| cached_pure_lazy_allowlist | stored hashes are SHA-256 of `ast.dump` text (verify_cached_pure_lazy_allowlist.py:160-170, :470) | `@ast_dump(n)`: CPython's exact text; `@sha256(@ast_dump(n))`, composite payloads with `cat` and `join` |
| permission_test_classification | JSON Lines files; arrays nested in arrays (`groups[].operations[]`, :552-597) | `IO=jsonl`; a `*` step in `path=`, each `*` a leading column |
| matching_architecture | scope from `--application-root` (:1162-1172) | `.scope dirs option("application_root")` |
| architecture_rule_docs | a cited commit exists; files at that commit (:219-247) | `git_commit(sha)`; `git_has` for any commit reachable from head |
| standalone_linter_entrypoints | a suite fails when a member policy fails or times out (plugin.py:127-152) | `.requires "policy", …`: one finding per member that blocks or ends Unknown(budget or timeout) |
| broad_type_surfaces, broad_type_surfaces_ratchet | PEP 696 type-parameter defaults (lint_broad_type_surfaces.py:701) | not a language need: Rerail's vendored tree-sitter-python already parses them (`type_default`, its patch 1); python/ast.rr's `type_param` reads bound and default |

## Prior work consulted

Recorded as for a decision: the source, what it says, and whether this design agrees.

- **Findings contract** (`rule-engine-direction/findings-contract.md`: keys :19-20, multiplicity :45).
  Agrees. Findings are keys, so message-only joins, duplicate findings and witness choice are not needs. File- and
  repository-level findings have no line, which is what `@place(path, 0)` and a missing path give.
- **Decision 0005, named derived facts** (admission rule 3). Agrees. New library relations are concept-named and
  shared by at least two checks (python/ast.rr, signing.rr). Single-check helpers stay in the check.
- **Stateful-reset semantics are a limit of relational lowering** (ast-index-spike history index, topic 2.21:
  DH L3791-3795; design index §7: LCS L191-195). Agrees for SQL, departs for this language. A guard cleared at
  each reassignment cannot be one flat anti-join. As Datalog it is "the latest reset before n", a `max` over
  syntax, followed by positive recursion; versioned-table-names.rr is that case. The remaining case, "latest
  qualifying write", is what `ordered by earlier` adds. It is evaluated in visit order, not lowered to SQL.
- **The policy layer (allowlist, ratchet, signatures) sits outside the query** (design index §18, LA). Departs, in
  part. Verification moves into Datalog (signing.rr), because each repository signs its own layout and the
  engine should not hard-code one. Enforcement stays outside the query: `.gate` and `.budget` are directives
  applied to the finished relations, not rules.
- **Lessons ledger #18: structured facts over text matching.** Agrees. `str_part`, `op_at`, `ast_name` and
  `expr_ctx` are structured facts where the analyses would otherwise match text. Text scans (`src_line`) are kept
  for checks that are about text: comments, line counts, Markdown.
- **Lessons ledger #21: expose extents and ordinals as columns, and let the caller join.** Agrees. `earlier`,
  `visit_index`, `ast_depth`, `end_line` and `col` are columns. `ordered by earlier` is evaluated over the same
  order, and no closure is materialised.
- **Rerail never runs repository code** (target architecture). Agrees: facts that only a run can produce are
  artifact inputs, not functors.

## Status

Nothing here has been compiled or run against an engine. `site/check_rr.py` checks groundedness, duplicate
declarations and stratification statically; it passes on every file. The signing layouts were checked against the
real signatures with PyNaCl. The rules that build those messages were not run. The findings layer and the run
artifacts are designed (FINDINGS-LAYER.md, RUN-FACTS.md), not built; the producers are sketches over the data
layer's existing functions.
