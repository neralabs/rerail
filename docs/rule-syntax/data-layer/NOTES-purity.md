# Purity group: notes

This group has four data-layer checks, written in `.rr`, and two libraries:

- `lib/data-layer/purity-ast.rr` (102 lines) holds the `ast` idioms that the purity and
  authority scripts spell the same way: `ast_walk`, `short_name`, `walk_name`, `body_def`,
  `last_body_def`, `class_field`, `deco_at`, `walk_qualname` and `py_repr`. `short_name` now
  sees through parentheses, so `(a.foo)` gives "foo".
- `lib/data-layer/purity-calls.rr` (181 lines) is new. It holds pure_lazy_runtime_purity's
  call graph:
  - receiver chains through parentheses;
  - each qualname's last def;
  - imports;
  - resolution to the longest matching module prefix;
  - method lookup through the first base that resolves;
  - the edges `pc_edge(f, g, via)`.

  It uses python/security.rr's `pos_arg`, `arg_count` and `kwarg`.

## Scores

Each score is exact events over scored events, from plain `tools/rrlab/rrlab dl --jobs 2`.
Before this round, dl dropped events that compare messages or fields; it now scores them on
their keys. "Reader" means the census reader lowers the file with no refusal. `check_rr.py`
prints ok for every file.

| check | Python lines | .rr lines | before | after | reader |
|---|---|---|---|---|---|
| authentication_plan_purity | 322 | 95 | 8/8 | 10/10 | yes |
| readthrough_authority_architecture | 355 | 96 | 10/10 | 12/12 | yes |
| cached_pure_lazy_allowlist | 915 (verifier) + 9 (wrapper) | 543, plus a 26-line generated table | 29/60 | 29/60 | yes |
| pure_lazy_runtime_purity | 3,286 | 559, plus a 64-line generated table | 54/54 | 54/54 | yes |

In the "before" column:

- the two small checks were scored with the old dl;
- the other two were scored with the new dl, on the rules as they were before the review.

Each generated table has a generator with a `--check` mode, which prints ok:

- `cached-pure-lazy-allowlist.symbols.rr` comes from `gen-cached-pure-lazy-allowlist.py`;
- `pure-lazy-runtime-purity.tables.rr` comes from `gen-pure-lazy-runtime-purity.py`.

The second generator now also emits:

- the name sets that were hard-coded inside linter functions (`INLINE`);
- the two `allow_sig` rows.

## Fixes from the review

### pure_lazy_runtime_purity (revision 2)

- **Scope.** `.scope include` covers `impl`, `data_layer`, `_internal` and `l####_*`, and
  `.scope exclude "**/tests/**"` drops tests. These are globs because rrlab does not model
  `.scope dirs`. scan_source also has the linter's own `/tests/` path test.
- **The grandfathered allowlist (PLP-ALLOWLIST).** This applies in plugin mode only, that
  is, with no `entryPoint` option.
  - The 235 strict entries are read from `scripts/allowlists/architecture_grandfathered/pure_lazy_runtime_purity/`.
    Each entry is `path, id, code, qualname, stable_scope, message`.
  - Entries match findings on everything but the id.
  - For each payload with k findings and a entries, the rule reports the first k − a
    findings, by line. An entry beyond the count is stale and gives
    "spurious allowlist entry: …".
  - The rule also reports the list's own errors:
    - field-count errors;
    - an entry whose path is not its fragment's;
    - duplicate entries;
    - ceiling field errors.
  - The list and the burn-down ceiling are each signed. They are checked with signing.rr's
    `allow_entry`, `allow_sig` and `allow_status`. Both signatures verify on the real
    files, checked with libsodium.
- **Categories.** A mutated global that no other category clause covers now gets
  `mutated-cache-or-registry`.
- **Method lookup.** It takes the first base that resolves.
- **Stop set.** `decorator` is in it.
- **PLP-007.** It reads only the first top-level literal `__all__`, and otherwise falls back
  to imports and defs.
- **Parentheses.** Receivers can be parenthesised: `(_OBJ).lookup(k)` and `(_CTX).get()`.
- **`**_OPTS`.** It no longer counts as mutable state in a frozen dataclass call.
- **Idioms.**
  - The call graph moved to purity-calls.rr.
  - The r1…r7 ladder became `plp_rhit` ranks with `min`.
  - The name sets are generated.

The reviewer's cases (`review-b/pure/cases/` a–j) now give what the linter gives.

### cached_pure_lazy_allowlist (revision 2)

- **What makes an item.**
  - Only a def is an item: `kind(f, "function_definition")` in `cpl_dcall` and `mpl_dcall`.
  - `@cached_pure_lazy(…)` on a class is now an unsupported call item.
  - `@memoized_pure_lazy(…)` on a class is now an escaped-decorator item.
- **Dict semantics.** The verifier keeps the last def and the last policy entry of each name.
  - `item_entry` pairs the last item (`visit_index`) with the last entry (`pe_ord`).
  - A cached item's errors are reported only for that last item. Every memoized item's
    errors are reported, as the verifier does.
- **cache_types.** A list that names the same argument twice is reported. The rule counts
  distinct keys (`pe_key`) against the number of pairs.
- **Parentheses.**
  - `(cache).facts.get()` gives the member `facts.get`. The chain is `cpl_link`, a node-only
    recursion.
  - `@(cached_pure_lazy)(…)`, `(getattr)(…)` and the policy constructors are read through
    `cpl_callee`.
- **`.pyi` files and `tests` directories.** rrlab's `.scope` filters findings only, and
  every finding of this check is at the policy module. Documents are therefore filtered in
  the rule (`cpl_doc`): `*.py` under the four roots, with no `tests` path part and no
  `test_` file.
  - A `test` directory is still scanned, as the verifier scans it. `.scope tests false`
    was wrong there, so it is gone.
- **`max_entries: null`.** It reads as "null" on both sides, so there is no false "bound
  changed".
- **Missing runtime symbols.** These raise in the verifier, so the run ends with that one
  message, which names a sorted Python tuple. The rule emits that message alone
  (`cpl_crash`).
- **Idioms.**
  - One `item_entry` relation now drives the cached and memoized comparisons.
  - The shared messages are written once, with `cpl_word`.
  - `pe_lit_pair` is factored out.

The reviewer's cases (`review-b/cached/a/b/` cx, cx2 and rt) now give the verifier's
messages. Hashes are the exception (below).

## Misses and approximations

### cached_pure_lazy_allowlist

The 60 events are scored in two runs, because one run of all of them takes over two minutes:
`--entry policy_violations` scores 42 events (18 exact) and `--entry memoized_policy_violations`
scores 18 (11 exact). None of the review's fixes changes a recorded event, so the score
stays 29/60. All 31 misses come from rrlab's stubs, not from the rule:

- 29 misses involve hashes only. The extra or missing findings are only
  `…: function body hash changed` or `…: function signature hash changed`.
- 2 misses come from the `@nfkc` stub: `cached-policy-unicode-decorator` and
  `cached-policy-unicode-function`.

- **Hashes.** The recorded entries carry `sha256(ast.dump(…))` digests. The rule hashes
  `@tree_digest` instead, as OPEN-ISSUES.md asks: the policy is re-signed once, since there
  is no `@ast_dump`. rrlab's `@tree_digest` is a stub, so no recorded digest matches.
- **`@nfkc`** is a stub in rrlab. The two full-width fixtures (`cached-policy-unicode-*`)
  depend on it.
- **One runtime-name corner.** A runtime name defined in two of the three source files is
  kept from the last file by the verifier. The rule keeps one row per file.

### pure_lazy_runtime_purity

- **The allowlist model has four assumptions.**
  - **The file column.** A line of `.input …(IO=lines, dir=…)` carries a path relative to
    that directory, which is what an entry's path is checked against.
  - **rrlab does not model `dir=`.** It gives no rows and warns "missing". No recorded
    event covers the allowlist anyway, because the tests call the scanners directly. The
    real-repo smoke test read the 235 entries through a scratch copy, with `filename=`.
  - **First by line.** "The first k − a findings" means the first by line, which is the
    linter's sort order.
  - **Qualname ties.** These follow the same line order.
- **The opaque payload field.** The linter names the latest assignment before the use. The
  rule names any earlier one, because a `max` inside the taint recursion would not stratify.
  Codes and lines are unaffected.
- **The shared-facts narrowing of scan_paths, approximated.**
  - The token test reads as "the file's text contains `_lazy` or `__pure_lazy_surface__`".
  - The call test reads as "edges not resolved through a base class".
- **Visit order.** Taint follows `earlier`. rrlab's `earlier` is preorder start order, not
  CPython's `ast` order. The two differ only inside one statement, for example in the order
  of keyword values.

### authentication_plan_purity

- **Annotations.** The linter compares the `ast.unparse` text of annotations; the rule
  compares their source text. The two differ only in parentheses and comments inside an
  annotation.
- **Duplicate method names.** `last_body_def` gives the linter's dict semantics where that
  dict is used (APP003).

### readthrough_authority_architecture

No misses, and no approximations beyond the linter's own spelling tests.

## Change requests

### 1. lib/data-layer/signing.rr: the allowlist version

`allow_message` hard-codes `"version":1`. The cached pure-lazy policy signs `"version":2`, so
`cached-pure-lazy-allowlist.rr` builds its message itself (`cpl_sign_msg`). The proposed
text replaces the current `allow_message` rule:

```
// allow_version("cached_pure_lazy", 2): the "version" the list signs; 1 when absent.
.decl allow_version(list: symbol, v: number)
.decl allow_ver(list: symbol, v: number)
allow_ver(l, v) :- allow_version(l, v).
allow_ver(l, 1) :- allow_sig(l, _, _), !allow_version(l, _).
allow_message(l, msg) :- allow_sig(l, linter, _), allow_ver(l, ver),
    body = join q by e sep "," : { allow_entry(l, e), q = @json_str(e) },
    msg = cat("{\"allowlist\":[", body, "],\"linter\":", @json_str(linter), ",\"version\":", to_string(ver), "}").
```

pure_lazy_runtime_purity needs no signing.rr change. Its two lists sign version 1.

### 2. rrlab lowering: `split_dot` with `_`

`split_dot(q, p, _)` lowers to `_ = @rr_dot_…(q)`, and Soufflé rejects that with
"Underscore in binary relation". The workaround is to name the column:
`split_dot(q, p, qx)`. The fix is to skip the `=` for an underscore column when lowering.

### 3. Census reader: a grounding failure it does not catch

Soufflé aborts with "Unable to ground parameter in materialisation-requiring aggregate
body" when an outer variable defined by a functor is used inside a multi-atom aggregate.
For example, `n = strlen(p), count : { plp_pre(s, p2), …, strlen(p2) > n } = 0`. The
reader accepts this shape. It should refuse it, or the lowering should materialise `n` first.

### 4. rrlab

- **`.scope dirs` is not modelled.** The rules use `.scope include` globs instead.
- **`.input … dir=` is not modelled.** It gives no rows.
- **`.scope` filters findings, not input documents.** A check whose findings all land on
  one file must filter documents itself, as `cpl_doc` does.
- **Tabs break the output.** A span finding whose message holds a tab crashes rrlab's
  output. PLP-ALLOWLIST messages can carry entry text, so they are placed at a fixed
  `@place`.
- **Stubs.** `@tree_digest` and `@nfkc` are stubs; they are what blocks the cached check's
  remaining scores.

No change is requested to `lib/core.rr` or to any other library file outside this group.

## Engine reader refusal (coordinator)

The final-shape reader (`rr-census`) refuses `walk_qualname` in `lib/data-layer/purity-ast.rr` (lines 85 and 87)
as an `UnboundedValueCycle`: the recursion builds a string with `@dot`. Each def has one qualified name, a function
of its node, so the cycle is bounded by the tree; python/ast's `dotted_expr` is refused for the same reason. The
request is for the reader to accept a value cycle whose head value is functionally determined by a node column
that strictly descends the tree, rather than for the rule to change shape.
