# Aligning `.rr` with the final-shape engine

## Intro

The final-shape engine (branch `jeremybarnes/final-rerail-shape`, worktree `wolfeel`) reads `.rr` as its only rule
format. Its reader is `logic::rr::parse` and `parse_file`. They lower a program, with `lib/core.rr` as its prelude,
to the engine's `Program`. This page compares what `core.rr` promises with what that reader and the engine's base
relations deliver. It records which side should move, and what was changed on this branch.

In short:

- The engine is behind `core.rr` far more often than the reverse. Most rows below are engine work.
- Two syntax forms stopped all 930 pack rules at the reader: the constant `code "X"` and the `suppress` line.
  The reader has since learnt both (section 4, status).
- One engine lowering is wrong rather than missing: `has(a, n, set)` puts the set on the ancestor.
- This branch changed descriptions in `core.rr`, one `rrlab` fact (`str` of an interpolated literal), two rules
  whose refused construct had an equally clear form, and one malformed message string.

Section 6 says where the two engine branches stand: the reader work of section 4 is on a sibling branch that
`final-rerail-shape` has not merged yet.

Engine sources read for sections 1 and 2 (worktree state of 2026-11-06, which has uncommitted work on top of
`e501fa254`):
`native/src/code_rule_dsl/logic/rr/{syntax,lower}.rs`, `logic/front.rs`, `logic/strings/functions.rs`,
`logic/native/setfix.rs` (`sym_cat`), `logic/plan/corpus.rs` (`NOT_MADE`), and `facts/vtab/syntax.rs` (the base
relations). Decisions 0036, 0037, 0042 and 0043; open items O70 to O79.

## 1. Mismatch table

"Engine" says what the engine reads, provides or lowers today. "Missing" means no source provides the relation: a
program that reads it fails at planning ("no statistics for …"). "Refused" means the reader stops with
`not lowered yet: …`. "Side" is the side that should change.

### Tape relations

| Name/arity | core.rr | Engine | Side | Action |
|---|---|---|---|---|
| `doc/3` | `(d, path, module)`; module `""` when the file has none | provided; the module is the path with `/` as `.` for every file (`src/a.ts` gives `src.a.ts`), `crate.a` for Rust. The translated prelude declares `doc/2` (O74) | engine | `""` for files with no dotted module; the prelude includes `core.rr` (O74). Whether `src/` is a source root (rrlab strips it, the engine does not) is a setting, not a rule matter |
| `pkg/2` | the Python package | provided, for every document | engine | rows for Python only (minor) |
| `path_part/2`, `file_name/2` | as written | same | — | — |
| `node/5` | every node, comments included | named nodes, comments left out (T's projection) | engine | comments become nodes (see `child`) |
| `kind/2` | grammar kind | a projection of `node` | — | — |
| `field/3` | `(p, f, c)` | same | — | — |
| `child/3` | i-th named child; comments are named children and are counted (now stated) | named children without comments; `i` counts only those | engine | comment nodes in `node`, `child` and `tape`. The libraries are written against tree-sitter's children: they filter comments themselves (`!kind(c, "comment")`, `body_stmt`, `call_arg`), and `php/phpdoc` and `has(b, _, "comment")` need comments in the tree. T's programs can keep T's projection |
| `text/2` | every node's and span's source text (now stated) | identifiers only (T's `Text` kinds), and none with `\t \n \r \\` | engine | text of every node. 0042 already plans the vocabulary for token-like nodes and a file scan for the rest |
| `op/2` | every anonymous token child, punctuation and keywords included (now stated) | T's: the `operator` field of five Python operator kinds, and `async`. No comparisons (O79), nothing for TypeScript, PHP or Rust | engine | `op` as `core.rr` says; T's narrow view under its own name for T's parity tests. Rules read 60 distinct tokens through `op` (`readonly`, `static`, `get`, `,`, `?.`, …) in 177 files; T's `op` has fewer than 20 of them, in Python only |
| `op_at/3` | ordered non-punctuation tokens (now stated exactly) | tokens of the `operator`/`operators` fields only (uncommitted worktree code) | engine | every non-punctuation anonymous token: `lib/complexity/rust.rr` reads `op_at(chain, i, "&&")` of a `let` chain, which has no operator field |
| `docof/2`, `line/2` | as written | `docof` a projection of `node`; `line` a view | — | spans have no rows (no spans yet) |
| `unset/1` | one row, the empty value; `text(x, "")` holds | the constant -1; `text(-1, "")` does not hold; refused inside an aggregate, `nearest` or `ast` (O77) | engine | as O77 says |
| `value/2` | static value: decoded strings, normalised numbers, keywords | decoded Python strings and decimal integers only (`0xff` has none) | engine | the other literals and languages |
| `str/2` | decoded contents; no row for a literal with interpolations (now stated) | the text between the quotes as written, escapes kept; no row with interpolations; Python only | engine | decode, as 0037's table says; other languages. This branch adopted the engine's "no row when interpolated" (section 3) |
| `strprefix/2` | prefix letters, lowercased | same, Python only | engine | other languages |
| `name/2` | an identifier, or a dotted chain, no spaces | identifier-like kinds and Python `attribute`; `dotted_name` keeps inner spaces | engine | member chains of the other languages (`member_expression`, `scoped_identifier`, …); drop spaces |
| `comment/2` | a comment's text without its marker | missing (0037: "not built yet") | engine | with comment nodes |
| `static_value/2`, `value_type/2`, `list_len/2` | as written | missing | engine | — |
| `source/2`, `src_line/3`, `line_count/2`, `end_line/2`, `col/2` | the raw text of every document | missing | engine | file-length and test-budget need them |
| `first_token`, `last_token`, `token_before`, `token_after` (2) | tokens as spans | missing | engine | — |
| `syntax_error/3` | `(d, line, msg)` | `syntax_error(d)`, one column | engine | add the line and the message |
| `intval/2`, `tape/4`, `core_node/3`, `py_*` | not in core.rr | provided (T's) | — | internal; `tape` is what `within` lowers to |

### Tree axes and order

| Name/arity | core.rr | Engine | Side | Action |
|---|---|---|---|---|
| `within/2` | n strictly below a | a range over the tape's ids; when nothing else gives n, a walk down `child` (O77) | — | the same set |
| `inside/3` | a a strict ancestor of n, a's kind in the set | `inside_<set>(n, a)`, made by the transducer | — | — |
| `has/3` | n a strict descendant of a, **n's** kind in the set | lowered as `inside_<set>(n, a)`: the set is tested on **a** | engine (bug) | `has(a, n, s)` is `within(n, a), kind(n, k), s(k)`. All 105 uses read it so (`!has(b, _, "comment")`: no comment inside b) |
| `nearest/3` | a is n's nearest strict ancestor in the set | lowered; `nearest(n, _, s)` as `inside`; `!nearest(n, a, s)` with a named is refused | engine | negation |
| `within_stop/3` | n below a, no stop node strictly between | lowered (positive), the same meaning | engine | `!within_stop` |
| kind sets | a relation of kinds, a relation of nodes, or one kind string | kinds only: "the kind set X is not a relation of kinds" | engine | node sets (`block_node`, `block_scope_at`, `scope_node`; 2 libraries, 22 packs) |
| `precedes/2` | among one parent's named children (now stated as `child` order) | a rule over `child`, `i < j` | — | the same, once comments are children |
| `next_sibling/2` | the next named sibling | missing | engine | a rule over `child`, `j = i + 1` |
| `precedes_stop/3` | as `precedes`, no stop sibling between | refused | engine | — |
| `earlier/2` | CPython's ast order for Python, else start order | preorder ids (start order); `!earlier` refused | engine | CPython order (rrlab is also start order) |
| `visit_index/2`, `ast_depth/2` | the visit order as a number; ast depth | missing | engine | — |
| `split_dot/3` | `q` is `m.x`, split at the last dot | positive: `q = @cat(m, x)`; negated it is left an undeclared relation | engine | negation (minor) |
| `parsed/3` | the tree of code inside a string | read as empty (O76) | engine | — |
| `spread/1` | `*args`, `...xs` | missing (`py_call_arg` has a spread column) | engine | — |
| `dfs_first/5`, `dfs_first_memo/6` | ordered first match | refused | engine | — |

### Text tests, functions and aggregates

| Name/arity | core.rr | Engine | Side | Action |
|---|---|---|---|---|
| `glob`, `prefix`, `suffix`, `contains`, `word` and `i` forms (2) | pattern first, both bound | the engine's tests; sets of one test are one test | — | — |
| `match/2` | whole-string regex; named groups bind | anchored `rx`; captures computed per row (O77); a named group outside a body is refused | engine | per O77 |
| `match_all/2` | one row per match | not lowered: an atom of an undeclared relation | engine | — |
| `@dot/2` | `"a.b"`, and `"" . b` is `b` | `sym_cat`: `"" . b` is `".b"` | engine | the empty case |
| `@qname`, `@pkg_up`, `@head`, `@dir`, `@join`, `@glob_sub`, `@package_of`, `@replace_all`, `@convert`, `@slice`, `@count_matches`, `@sha256`, `@json_str`, `@nfkc` | as written | provided (`strings/functions.rs`) | — | — |
| `@lower/1` | Unicode lowercase | not registered: SQLite's own `lower()` answers, ASCII only | engine | register it |
| `@to_precision`, `@semver_valid`, `@semver_subset`, `@place`, `@ed25519_ok`, `@tree_digest`, `@unparse` | as written | missing | engine | — |
| Soufflé `cat`, `strlen`, `to_string`, `substr` | Soufflé's | `cat/2` as `concat`; the others provided | engine | `cat` of 3 to 7 arguments is refused (13 library clauses; the first refusal of 37 packs) |
| Soufflé `to_number`, `to_float`, `max/2`, `min/2`, `%`, decimals | Soufflé's | refused | engine | — |
| `count`, `min`, `max` | Soufflé's | lowered; an aggregate inside a term (`1 + count : { … }`) refused | engine | — |
| `sum`, `mean`, `join … by … sep` | Soufflé's, and addition 5 | refused; `sum strlen(x) - strlen(a) : { … }` is a syntax error | engine | O73 |
| recursion that makes values (`i + 1`, `@cat(q, a)`), bounded by the data | allowed, as in Soufflé | `UnboundedValueCycle` in `front::finish` (29 library clauses; the first error of 10 packs) | engine | bound it as O77 bounds shrinking functions, or by the tree |
| `!(a, b)`, a negated conjunction | Soufflé's | refused ("a negated disjunction") | both | two rewritten here (section 3); 17 left in packs; the engine can name a helper relation itself |

### Additions and directives

| Construct | core.rr | Engine | Side | Action |
|---|---|---|---|---|
| `n ~ \`pattern\`` | addition 1 | refused | engine | O73 |
| `must`, `may` | addition 3 | refused | engine | O73 |
| `ordered by earlier` | addition 6 | refused (`ordered` on `.decl`) | engine | O73 |
| `=> edits`, `.edit` | addition 4 | fixes are left out | — | — |
| `.violation V(at: node, …)` | at a node, a span or a `@place` | lowered at a node; at a span or a place refused | engine | O73 |
| `.violation … code "E501"` | a constant code (916 packs); now listed in core.rr | syntax error: only `code {column}` | engine | parse it; the finding's code is the string |
| `suppress TOOL "rule", …` | continuation line (910 packs); now listed in core.rr | syntax error | engine | parse it; the findings layer applies it |
| `tag {t}` | an extra key field | checked, not carried | engine | — |
| `.mode`, `.scope`, `.gate`, `.budget`, `.on`, … | findings-layer settings | kept as settings | engine | the findings layer |
| `.input r(IO=…, filename=…)` | a policy file | empty when absent, an error when present (O76) | engine | — |
| `option/2`, `option_item/3`, `file/1`, `package*`, `tsconfig*`, `git_*`, `signed_list/2`, `str_part/5`, `input_error/3` | repository inputs | missing (`string_annotation` read as empty, O76). `option` alone is read by 135 files | engine | — |
| relation names `inside_*` | free | `inside_` is the engine's axis prefix; five rule relations use it (`inside_def`, `inside_concat` ×2, `inside_right`, `inside_braces`) | engine | a prefix rules cannot write |

### O79: `op` of a comparison

O79 leaves it to the user: either `op` follows `core.rr`, or comparisons get their own relation. The census above
settles more than comparisons. TypeScript, PHP and Rust rules read keywords and punctuation through `op`, and T's
`op` has none of them. A comparison relation would fix one row of the table and leave the rest. The recommendation
is that `op` follows `core.rr`, and that T's parity tests read T's view under another name. Until then,
`op_at(c, _, s)` gives the comparison operators in both implementations.

## 2. Parse census

### Method

The engine crate is large (cranelift, gix, tokenizers and the grammars). Its reader, though, depends only on
`std`, `serde_json` and `serde_yaml_ng`. So the reader was copied verbatim into a scratch crate:
`logic/front.rs`, `front/cycles.rs`, `front/yaml.rs`, `rr.rs` and `rr/{syntax,lower,check,print,translate}.rs`.
Two stubs stand in for the rest: `strings.rs`, with the three items the reader uses (`TESTS`, `is_test`, `SET`),
and `magic::{Report, Gate}`, with two struct definitions. The copy was taken from the worktree on 2026-11-06.

The scratch crate sits beside a symlink `docs → this branch's docs`. So `rr::library()`
(`CARGO_MANIFEST_DIR/../docs/rule-syntax/lib`) resolves `#include` against this branch's `lib/`. A driver
(`rr/census.rs`) does two things:

1. It calls `parse_file` on each file: lowering, includes and the `core.rr` prelude.
2. It lowers each of the file's own rule clauses alone, with every declaration and fact of its program. This shows
   every refusal, not only the first, and places each one at its clause.

A second build adds a census-only shim to `syntax.rs`. It skips `code "X"` and `suppress …` lines, so the packs'
next layer shows. The wolfeel worktree was not touched.

### Files (`parse_file`, the first error only)

| Area | Files | Parse | Refused | Syntax error | Other |
|---|---:|---:|---:|---:|---:|
| lib | 106 | 16 | 84 | 0 | 6 |
| data-layer | 12 | 5 | 7 | 0 | 0 |
| packs, the engine's reader | 949 | 19 | 0 | 930 | 0 |
| packs, with the shim | 949 | 81 | 835 | 1 | 32 |

Every pack that parses is a facts file with no `.violation`. The first error of a library file is usually in a
library it includes. The clause view below places each refusal where it is written.

Syntax errors in the packs, by the engine's reader:

| Count | Error | Cause |
|---:|---|---|
| 930 | `expected a string, got Id("code")` | `.violation V(…) code "X" "message"`, the constant code |

With `code "X"` accepted, 910 stop at `expected "(", got Id("eslint")`, the `suppress TOOL "rule"` line. With both
accepted, two syntax errors are left:

| File | Error | Cause | Side |
|---|---|---|---|
| `packs/python/ruff-up032-f-string.rr:71` | `expected ".", got Id("strlen")` | `g = sum strlen(x) - strlen(a) : { … }`, an aggregate over an expression | engine (and `sum` is refused anyway) |
| `packs/python/ruff-ytt102-sys-version2.rr:13` | `expected an identifier, got Num(1)` | an unescaped `"1"` inside the message | this branch: fixed |

### Refusals by construct, per clause

Each clause is lowered alone, so each refusal is counted where it is written.

Libraries: 3,905 clauses, 3,831 lowered.

| Clauses | Refusal | Examples |
|---:|---|---|
| 29 | `UnboundedValueCycle` (recursion making values, bounded by the data) | `lib/php/format.rr:71`, `lib/python/ast.rr:103`, `lib/python/names.rr:275` |
| 12 | the built-in function `cat/3` (and 1 of `cat/5`) | `lib/complexity/cyclomatic.rr:41`, `lib/php/format.rr:51` |
| 10 | the aggregate `join` | `lib/data-layer/signing.rr:35` |
| 10 | the built-in function `to_number/1` | `lib/php/arrays.rr:76`, `lib/php/values.rr:78` |
| 4 | the built-in function `to_float/1` | `lib/python/compare.rr:55`, `lib/python/security.rr:165` |
| 2 | an aggregate inside a term | `lib/complexity/cyclomatic.rr:36`, `lib/python/ruff.rr:82` |
| 2 | the built-in function `max/2` | `lib/php/format.rr:60`, `lib/tsx/modules.rr:198` |
| 1 | `precedes_stop` | `lib/php/phpdoc.rr:22` |
| 1 | the aggregate `sum` | `lib/python/misc2-messages.rr:58` |
| 1 | `!nearest(n, s, set)` with s named | `lib/python/misc2-self.rr:35` |
| 1 | arithmetic `%` | `lib/rust/syntax-literals.rr:153` |
| 0 (was 1) | a negated conjunction | `lib/python/docstrings.rr:89`, rewritten |

There are also two file-level errors in the libraries: a kind set that is a relation of nodes (`lib/sql/syntax.rr`
`block_scope_at`, `lib/tsx/statements.rr` `block_node`), and four `UnboundedValueCycle` errors in `lib/rust/*`.

Data layer: 347 clauses, 268 lowered. The counts moved from 315 and 236 during the run, because of concurrent edits
to `data-layer/` that are not this work.

| Clauses | Refusal | Examples |
|---:|---|---|
| 31 | `.violation` at a `loc` (a `@place`) | `data-layer/file-length.rr:33`, `test-budget.rr` |
| 21 | `must` | `data-layer/plan-scope-side-effects.rr:72` |
| 13 | `.violation` at a span | `data-layer/noqa-sec-ratchet.rr:28` |
| 10 | `ordered by earlier` | `data-layer/sketches/owner-code.rr:22` |
| 1 each | `dfs_first`, `dfs_first_memo`, a pattern `~`, `precedes_stop` | `sketches/transitive-eager.rr`, `versioned-table-names.rr:191`, `:201` |

The data layer's `join` refusals are in files, through `lib/data-layer/signing.rr`. Five data-layer files parse
whole: layer-violations, post-op-no-internal-dispatch, public-api-triples and swallowed-exceptions, and
missing-async-pairs before the concurrent edits. It now stops at the `join` of `lib/python/ast.rr:300`, which
it reaches through an include.

Packs, with the shim: 5,242 of the packs' own clauses, 4,602 lowered. The first error of a pack file is
usually in a library (`join` 500, `max/2` 214, `cat/3` 34, a node-valued kind set 22, `to_number` 15,
`UnboundedValueCycle` 10, an aggregate inside a term 8). The packs' own clauses stop at:

| Clauses | Refusal | Examples |
|---:|---|---|
| 373 | `.violation` at a `loc` (a `@place`, a token) | `packs/javascript/eslint-accessor-pairs.rr:23` |
| 109 | `.violation` at a span | `packs/javascript/eslint-eqeqeq.rr:17` |
| 94 | `cat` of 3 to 22 arguments | `eslint-capitalized-comments.rr:42`, `ruff-era001-commented-out-code.rr:80` |
| 17 (was 18) | a negated conjunction, mostly "an option is set and …": `!(option("x", "true"), afterthought(u))` | `eslint-no-plusplus.rr:20`, `typescript-eslint-no-use-before-define.rr:29` |
| 14 | `to_number/1` | `eslint-id-length.rr:21` |
| 11 | the aggregate `join` | `eslint-no-console.rr:20` |
| 6 | `to_float/1` | `eslint-yoda.rr:66`, `bandit-b505-weak-cryptographic-key.rr:25` |
| 6 | `UnboundedValueCycle` | `ruff-b005-strip-with-multi-characters.rr:26` |
| 5 | a decimal number | `eslint-yoda.rr:63` |
| 2 | `%` | `bandit-b103-set-bad-file-permissions.rr:20` |
| 2 | an aggregate inside a term | `ruff-pyi036-bad-exit-annotation.rr:44` |
| 1 | `max/2` | `eslint-max-classes-per-file.rr:23` |

The 17 negated conjunctions are left as they are. Each reads "unless the option is on and this holds", in the
place it applies. A helper relation per option would scatter them. The engine can name the helper itself, or
apply De Morgan to a conjunction of tests.

## 3. Changes on this branch

| File | Change | Why | Semantics |
|---|---|---|---|
| `lib/core.rr` | `child`: comments are named children and are counted | the engine leaves them out; the libraries rely on them | unchanged (rrlab already did this) |
| `lib/core.rr` | `text`: every node's and span's text, as written | the engine reads it as "identifiers" | unchanged |
| `lib/core.rr` | `op`: every anonymous token, punctuation and keywords included, with examples; `op_at`: the punctuation it leaves out, and any field | O79; the engine's `op` and `op_at` are narrower | unchanged (as rrlab and 177 files use it) |
| `lib/core.rr` | `str`: decoded (0037), and no row for a literal with interpolations | the engine's and 0037's built `str` have no row there | **changed**, rrlab follows |
| `lib/core.rr` | `has`: "not `inside` with its columns swapped", with its Datalog meaning | the engine's lowering swaps the columns | unchanged |
| `lib/core.rr` | `precedes` given as `child` order | exactness | unchanged |
| `lib/core.rr` | `match("re", s)`, which the text called `glob("re", s)` | typo | — |
| `lib/core.rr` | the `.violation` list gains `code "X"`, `suppress TOOL "rule", …` and `fix [safe] F "title"` | 930 packs use them; core.rr did not list them | — |
| `tools/rrlab/src/main.rs`, `README.md` | no `str` or `value` for a Python string with an `interpolation` child, or a PHP `encapsed_string` with a part other than `string_content`/`escape_sequence`; `strprefix` kept | the `str` change | facts |
| `lib/python/docstrings.rr` | `!(prefix("__", x), suffix("__", x))` becomes `!doc_dunder_seg(x)`, a named helper | refused construct, equally clear named form | none |
| `packs/javascript/eslint-no-warning-comments.rr` | `!(eslint_directive(c), match(…, v))` becomes `!names_this_rule(c)` | the same | none |
| `packs/python/ruff-ytt102-sys-version2.rr` | the message's `"1"` becomes `` `1` `` | an unescaped quote ended the string | message only |

Not changed, and why:

- **No library or rule was rewritten around a missing relation or a refused construct** other than the two above.
  `cat/3`, `max/2`, `to_number`, `join`, `sum`, bounded value recursion, node-valued sets, `precedes_stop` and the
  rest have no equally clear form. Nesting `cat` three deep, or spelling `max(a, b)` as two rules, would make the
  rules worse.
- **`op` and `cmp` (O79):** `lib/python` reads `op(c, "in")`. Moving it to `op_at(c, _, "in")` would fix one
  library for the engine, while 176 other files still need `op` as `core.rr` has it. The fix belongs in the engine.
- **`lib/python/names.rr` and `imports.rr`** (O78) are the coordinator's. Nothing here needs them changed.

`python3 site/check_rr.py` prints `ok` after the changes.

## 4. Engine-side work

In order of what it unblocks:

1. **Reader syntax:** `code "X"` on `.violation` (930 packs) and `suppress TOOL "rule", …` lines (910 packs).
   Also an aggregate over an expression (`sum e : { … }`).
2. **`has(a, n, set)`:** the set must be tested on `n`. Today it is tested on `a`, so `!has(b, _, "comment")` and
   `has(b, k, "break_statement")` test the wrong node.
3. **Soufflé's built-ins:** `cat` of any arity, `max/2`, `min/2`, `to_number`, `to_float`, `%`, decimals, and an
   aggregate inside a term. These are small. They are the first refusal of 41 library files and of 274 packs
   (with the shim; section 2).
4. **Base relations as `core.rr` states them:** comment nodes (in `node`, `child`, `tape`) with `comment`; `text`
   of every node; `op` of every anonymous token (T's view kept under another name: O79); `op_at` beyond operator
   fields; `syntax_error/3`; `next_sibling`; `str` decoded and in every language; `value`, `strprefix` and
   `name` for the other languages; `@dot("", b) = b`; `@lower` registered.
5. **Missing relations:** the source-text family (`source`, `src_line`, `line_count`, `end_line`, `col`), tokens,
   `visit_index`, `ast_depth`, `spread`, `static_value`, `value_type`, `list_len`, the repository inputs (`option`
   and `option_item` first: 135 and 48 files), `str_part`, `parsed` and `string_annotation` (O76), and `match_all`.
6. **Lowering (O73, step 30):** node-valued sets in the axes; `!nearest` with a named ancestor, `!within_stop`
   and `!earlier`; `precedes_stop`; `dfs_first`; `.violation` at a span or a place; patterns; `must`/`may`;
   `ordered by`; `sum`, `mean` and `join`; negated conjunctions, through a helper relation; and recursion that makes
   values but is bounded by the data (`UnboundedValueCycle`, 29 library clauses).
7. **O77 shortcuts:** `unset` as a value with `text(x, "")`; `within` as a range read; captures chunk-at-a-time;
   `earlier` in CPython's order for Python.
8. **Hygiene:** an axis prefix that rule names cannot collide with (`inside_def` and four others exist);
   `doc`'s module `""` for files with none, and `pkg` for Python only; translated programs include `core.rr`
   (O74).

### Status after the engine work (2026-11-06)

The work was done on the sibling branch `jeremybarnes/final-rerail-shape-rr-align`, recorded in its decision
0046-rr-as-core-rr-states-it.md. The `op` question (O79) was settled as `core.rr` says: `op` is every anonymous
token, and T's narrower view is kept as `op_t` for T's parity tests.

| Census | Before | After |
|---|---|---|
| Library files that parse whole | 51 of 106 | 98 of 106 |
| Library clauses that lower | 3,831 of 3,905 | 3,890 of 3,906 |
| Pack files that parse whole | 0 (all stopped at `code "X"`) | 871 of 949 |
| Pack clauses that lower | 4,602 of 5,242 (with the shim) | 5,220 of 5,264 (no shim) |
| Data-layer clauses that lower | 293 | 359 of 372 |

Done:
- items 1 to 5 and 8, apart from the items listed below;
- from item 6, everything except `ordered by`, `dfs_first`, patterns, `mean` and the unproven value cycles;
- from item 7, `unset`.

Left, with the reason:
- **`ordered by`** (37 clauses) needs evaluation in visit order per document, which the planner does not do.
- **`@place` as a value outside a violation's head** (12 files) needs a place value in the node/span domain.
- **Value cycles with bounds the check cannot prove yet** (73 files, 15 library clauses): `hex_place`,
  `norm`/`sci`, `unescape`, `slot_walk`. They are refused, never run to a budget.
- **Still refused:** `dfs_first`, `~` patterns and `mean`.
- **Text layer:** captures and `match_all` are still computed per row, not chunk-at-a-time (decisions 0041, 0042).
- **Not yet made:** `parsed` and `string_annotation` (O80), and `earlier` in CPython's order.
- **O74:** translated programs still read T's shape.
- **A relation named `set`** breaks the SQL lowering. Table names need quoting.

Findings replay (path, line, code), as `events / compared / exact / errors`:

| Run | Recordings 1–6 | Batches 7–8 and probes |
|---|---|---|
| 0043 and O72 | `2350 155 153 0` | `1638 20 16 0` |
| After the base relations | `2350 178 176 0` | `1638 95 91 0` |
| After the reader | `2350 178 171 0` | `1638 117 100 0` |
| After the PSE fixes | `2350 178 177 0` | `1638 117 113 0` |

The hand-written data-layer rules now lower, and are compared instead of the converted JSON. The drop of 5 was
plan-scope-side-effects. The recorded findings spell its codes `PSE-050`, but `core.rr` names a violation's code
after the violation. Its rule now gives each code with `code "PSE-…"`, and it sees through a parenthesized `if`
test. The replay also drops `.mode log` codes on the suite plugin's events, as the plugin does. The check is then
7 of 7 and 22 of 22. The last row adds that check's targeted replay to the reader run's other checks.

## 5. Scores before and after

Only the `str` change touches semantics. In Python, only `lib/python/pyflakes.rr`'s `all_entry` reads `str`.
Through it, F822, F405, F406, the D1xx visibility rules (`doc_listed`) and the ANN2xx rules read it too. In PHP,
`php_value` reads `str`, and it already guards interpolation itself. Each rule was scored with
`tools/rrlab/rrlab score` before and after (own fixtures):

| Rule | Before | After |
|---|---|---|
| python/ruff-f405-undefined-local-with-import-star-usage | 4 / 0 / 0 | 4 / 0 / 0 |
| python/ruff-f406-undefined-local-with-nested-import-star-usage | 4 / 0 / 0 | 4 / 0 / 0 |
| python/ruff-f541-f-string-missing-placeholders | 22 / 0 / 0 | 22 / 0 / 0 |
| python/ruff-f822-undefined-export | 7 / 0 / 0 | 7 / 0 / 0 |
| php/phpstan-argument-sprintf | 21 / 0 / 0 (2 explained) | 21 / 0 / 0 (2 explained) |
| php/phpstan-argument-sscanf | 3 / 0 / 0 | 3 / 0 / 0 |
| php/phpstan-array-duplicate-key | 17 / 0 / 0 (10 explained) | 17 / 0 / 0 (10 explained) |
| php/phpstan-if-always-false | 7 / 0 / 0 (2 explained) | 7 / 0 / 0 (2 explained) |
| php/phpstan-random-int-parameters | 9 / 0 / 0 (1 explained) | 9 / 0 / 0 (1 explained) |
| blade/bladestan-identical-always-true | 0 / 0 / 1 | 0 / 0 / 1 |
| blade/bladestan-if-always-true | 0 / 0 / 1 | 0 / 0 / 1 |

The recording has no fixtures for ANN201–206, D101–D107, PYI056, RUF001, RUF022 and RUF068; their scores are
`n/a` both times. The facts were checked directly: `f"x{y}z"` has `strprefix` "f" and no `str`;
`f"plain"` has `str` "plain"; PHP `"x $y"` has no `str`.

The two rewrites (section 3) do not change semantics, and were checked as well:

| Rule | Before | After |
|---|---|---|
| javascript/eslint-no-warning-comments (own fixtures) | 8 / 0 / 0 | 8 / 0 / 0 |
| javascript/eslint-no-warning-comments (`--all`) | 10 / 0 / 0 | 10 / 0 / 0 |
| javascript/eslint-no-warning-comments, every finding on its 44 fixtures | — | identical |
| python D101 and D103 (`doc_private_module`), `rrlab run` on a package tree with `_priv.py`, `__main__.py`, `__.py`, `___.py`, `_sub/`, `__x__/` | 7 + 7 findings | the same 14 findings |

## 6. Where the engine branches stand (2026-11-06)

There are two engine branches:
- **`jeremybarnes/final-rerail-shape`** at `a2667aed2`: the final-shape engine itself. Since `42af062ab` it has
  added steps of decisions 0044 and 0045, and copies of this branch's files.
- **`jeremybarnes/final-rerail-shape-rr-align`** at `38d105533`: the reader work of section 4, cut from
  `42af062ab`. It is not merged into `final-rerail-shape`.

How they compare:

| | `final-rerail-shape` | `-rr-align` |
|---|---|---|
| `.rr` reader (`rr/syntax.rs`, `rr/lower.rs`) | as `42af062ab`: section 2's first census | section 4's status table |
| Pack files the reader takes | none (`code "X"` stops all of them) | 871 of 949 |
| Replay, recordings 1–8 together | `3988 273 266 0` | `3988 295 290 0` |
| Library files hashed, program cached by them (0044 step 1) | yes | no |
| Node sets in chunked containers (0045) | steps 1–3 | no |

The rr-align branch was cut before 0044 step 1. A trial merge (`git merge-tree`) conflicts in two files:
- **`logic/rr.rs`:** step 1 moved `parse_file`'s body into `read_file`, the version that caches the program.
  rr-align added a check to `parse_file` that refuses rule names starting `rr_`. The check belongs in
  `read_file`, before lowering, so that a cached program is checked too.
- **`logic/rr/tests.rs`:** step 1 made `read` and `program` also return the files they read. rr-align's tests
  call both, so they take the statements from the result.

Neither side changes what the other does. The merge keeps both.

### What 0044 means for the rules

0044's later steps change what `core.rr` describes. Nothing on this branch changes yet:
- **`file`:** its L0 seeded fact is `file(path, hash)`, but `core.rr` declares `file(path)` for import
  resolution. Either `core.rr` adds the hash, or the engine keeps `file/1` as a projection. The rules read only
  the path.
- **Path facts (step 3):** `module`, `pkg`, `path_part` and `file_name` become `.rr` rules over `file` in the
  library, instead of base relations computed in Rust. Their `.input` lines in `core.rr` would then become rules,
  with the same meaning: `doc`'s module is `""` for a file with no dotted module, and `pkg` is for Python only.
- **`library_file` and `config_file`:** the engine reads these for itself; no rule reads them.
  `config_file` waits for O76, so that `.input` files (`option`, the ignore lists) become hashed inputs.

0045 changes no rule: it is how the engine stores a set of nodes.
