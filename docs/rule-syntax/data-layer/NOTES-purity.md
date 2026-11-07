# Purity group: notes

This group has four data-layer checks, written in `.rr`, plus the library they share,
`lib/data-layer/purity-ast.rr` (101 lines). That library holds the `ast` idioms the purity
and authority scripts spell the same way: `ast_walk`, `short_name`, `walk_name`, `body_def`,
`last_body_def`, `class_field`, `deco_at`, `walk_qualname` and `py_repr`.

## Scores

Each score is exact events over scored events, from `rrlab dl --jobs 2`. "Reader" means the
census reader lowers the file with no refusal; `check_rr.py` prints ok for every file.

| check | Python lines | .rr lines | score | reader |
|---|---|---|---|---|
| authentication_plan_purity | 322 | 95 | 8/8 | yes |
| readthrough_authority_architecture | 355 | 96 | 10/10 | yes |
| cached_pure_lazy_allowlist | 915 (verifier) + 9 (wrapper) | 498, plus a 26-line generated table | 28/59 | yes |
| pure_lazy_runtime_purity | 3,286 | 658, plus a 46-line generated table | 54/54 (see below) | yes |

The generated tables have a generator and a check:

- `cached-pure-lazy-allowlist.symbols.rr` comes from `gen-cached-pure-lazy-allowlist.py`.
- `pure-lazy-runtime-purity.tables.rr` comes from `gen-pure-lazy-runtime-purity.py`.

Run either generator with `--check`; it prints ok.

### How pure_lazy_runtime_purity was scored

Plain `rrlab dl pure_lazy_runtime_purity` scores **0 events**. Every one of its 54 events sets
`compare_fields` or `compare_message`, and `dl.py` drops such events. To score it anyway, the
scratch wrapper `dlx.py --fields` deletes those two keys, so the events are compared on
(path, line, code) keys only, the loose contract. Results:

- `--entry scan_source`: 34/34 exact.
- `--entry scan_paths`: 20/20 exact.

The rule follows the `entryPoint` option. With `scan_source` it analyses each file on its
own. Otherwise it runs the `scan_paths` closure, which is the default and what the plugin uses.

## Misses and refusals

### cached_pure_lazy_allowlist: 31 misses

There are 59 scored events (18 memoized, 41 cached). All 31 misses come from rrlab's stubs,
not from the rule.

- **29 misses involve hashes only.** The extra or missing findings are only
  `…: function body hash changed` or `…: signature hash changed`. The recorded policy entries
  carry `sha256(ast.dump(…))` digests, and the rule hashes `@tree_digest` instead, as
  OPEN-ISSUES.md asks: the digests are re-signed once, since there is no `@ast_dump`. rrlab's
  `@tree_digest` is a stub (`node:N`), so no recorded digest can match until the policy is
  re-signed with the real functor.
  - 7 are memoized events.
  - 22 are cached events.
- **2 misses come from the `@nfkc` stub** (`cached-policy-unicode-decorator` and
  `-unicode-function`). The fixture spells `ｄerive` and `@ｃached_pure_lazy` in full-width
  letters. CPython reads both as NFKC, but rrlab's `@nfkc` returns its input unchanged, so
  the names do not match.
- **One more event is not scored.** Its file has an absolute path
  (`/private/var/…/optimizer_run_cache.py`), and rrlab hangs before Soufflé starts.
  `dlx.py` skips that event.

### pure_lazy_runtime_purity: what is not modelled

These parts of the linter have no recorded event, so the scores above do not test them.

- **The plugin's grandfathered allowlist.** In the plugin, findings are reconciled against
  `scripts/allowlists/architecture_grandfathered/pure_lazy_runtime_purity/**.txt`.
  - Each fragment uses the strict 6-field inventory: a content id, the code, the qualname,
    the stable scope, the message, and the path from the file name. The ids are computed by
    `lint_lib`'s `load_strict_allowlist`.
  - The reconciliation also counts multiplicity, checks the burn-down ceiling file (3 fields)
    and checks two ed25519 signatures. Both signatures are in the linter source.
  - None of this is in the rule. In a full-repository run, every grandfathered finding is
    therefore reported. This is a refusal for now, pending a strict-inventory section in
    signing.rr (see the change requests).
- **The shared-facts narrowing of scan_paths, approximated.**
  - In the linter, `_pure_lazy_candidate_paths_from_shared_facts` keeps a file when:
    - the file's tokens include `pure_lazy`, `__pure_lazy_surface__` or `_lazy`; or
    - the file defines a global-state factory; or
    - a seed reaches the file through name, module-attribute or own-class calls (never
      through base classes).
  - The rule reads the token test as "the file's text contains `_lazy` or
    `__pure_lazy_surface__`". It reads the call test as edges that are not resolved through
    a base class.
  - Two probes pin this down: inherited-method-admitted-import and transitive-inherited-method.
- **The `/tests/` path test of scan_source.** `.scope tests false` replaces it.
- **Visit order.** Taint follows `earlier`. rrlab's `earlier` is preorder start order, not
  CPython's `ast` order. The two differ only inside one statement, for example the order of
  keyword values.
- **Category messages.** When a global is bound more than once at top level, the linter
  keeps the last binding's PLP-009 category. The rule may give one row per binding. Only
  messages differ; codes and lines do not.
- **Messages in general** are paraphrased where they would need `{x!r}`, except PLP-009.
  The findings contract compares keys only.

### authentication_plan_purity

- **Annotations.** The linter compares `ast.unparse` text of annotations; the rule compares
  the source text. The two differ only in parentheses and comments inside an annotation.
- **Duplicate method names.** In a class body, the linter's dict keeps the last def of a
  name. The rule uses `last_body_def` for that only where the dict is used (APP003).

### readthrough_authority_architecture

No misses and no approximations beyond the linter's own spelling tests.

## Change requests

### 1. lib/data-layer/signing.rr: the allowlist version

`allow_message` hard-codes `"version":1`. The cached pure-lazy policy signs `"version":2`, so
`cached-pure-lazy-allowlist.rr` builds its message itself (`cpl_sign_msg`). Proposed text,
replacing the current `allow_message` rule:

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

### 2. lib/data-layer/signing.rr: strict allowlist inventories

These are the 6-field fragments under `architecture_grandfathered/<linter>/`, with content
ids, plus the burn-down ceiling files.

`pure_lazy_runtime_purity` needs this to be complete, and other `architecture_grandfathered`
linters probably need it too. I have not written exact text, because the id and the
reconciliation rules live in `scripts/lint_lib` (`load_strict_allowlist`,
`reconcile_strict_allowlist`, `load_burn_down_ceiling`), which this group did not port. This
is an open item for whoever owns signing.rr.

### 3. rrlab lowering: `split_dot` with `_`

`split_dot(q, p, _)` lowers to `_ = @rr_dot_…(q)`. Soufflé rejects that with "Underscore in
binary relation". The workaround is to name the column (`split_dot(q, p, qx)`).

Fix: when lowering `split_dot`, skip the `=` for an underscore column, as for any functor
result that is not used.

### 4. Census reader and Soufflé: a grounding failure the reader does not catch

The census reader accepted this rule:

```
plp_res(s, g) :- plp_pre(s, p), plp_fq0(p, g), n = strlen(p),
    count : { plp_pre(s, p2), plp_fq0(p2, _), strlen(p2) > n } = 0.
```

Soufflé 2.x then aborts with "Unable to ground parameter in materialisation-requiring
aggregate body". The outer variable `n` comes from a functor and is used inside a
multi-atom aggregate.

The rule now uses `max k : { … }` over a helper relation. Either the reader should refuse
this shape, or the lowering should materialise `n` first.

### 5. rrlab dl

- **(a) Do not drop events that compare messages or fields.** Under the loose contract,
  score their keys. Exact text in `dl.events`: replace

  ```
  if any(e.get(k) for k in ("compare_message", "compare_fields")) or e.get("path_skip_override"):
  ```

  with

  ```
  if e.get("path_skip_override"):
  ```

  Without this change, pure_lazy_runtime_purity has no scored event at all.
- **(b) Do not hang on an event whose file path is absolute.** Either skip such an event or
  rebase its path under the work root.
- **(c) Stubs that block exact scores:**
  - `@tree_digest` returns `node:N`;
  - `@nfkc` returns its input;
  - `@ed25519_ok` returns 0.

  The cached check's 31 misses all come from the first two.

No change is requested to `lib/core.rr` or to any other library file.

## What cannot be expressed

Nothing in these four checks is out of reach of the language. The two gaps are both data
the rules cannot reach yet:

- The `ast.dump` digests need a re-signing with `@tree_digest`, already planned in
  OPEN-ISSUES.md.
- The strict-inventory allowlist needs a signing.rr section (change request 2).
