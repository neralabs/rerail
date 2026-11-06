# rrlab

rrlab is a lab tool for converting and reviewing rules. It runs a hand-written `.rr` rule on real source files
and compares its findings with what the upstream tool recorded on the same fixtures. The real engine is not
involved. rrlab lowers the rule to plain Soufflé, feeds it tree-sitter facts, and reads back the violation rows.

It is not the engine. Where the two differ, the engine's semantics (`lib/core.rr`) are the reference. The
gaps are listed below.

Three parts:

- `rrlab-facts` (Rust, `src/main.rs`): parses files with tree-sitter and writes Soufflé `.facts` for core.rr's
  tape relations.
- `lower.py` and `patterns.py`: read a rule and its includes (core.rr is always included first), and print a
  plain Soufflé program.
- `functors/functors.cpp`: core.rr's `@functors` plus the lowering's text-test helpers, loaded with
  `souffle -l`.

`rrlab.py` builds the first and third parts on first use: `cargo build --release --offline` and `c++ -shared`.
The CLI then glues the parts together. It needs Soufflé 2.5 on PATH, a Rust toolchain and a C++ compiler.
It does not need Node or CPython inside the engine; Python is only the glue.

## Usage

```sh
# Score a rule against its pack's upstream recording. Default: the rule's own fixture directory.
#   TP/FP/FN are matched on (path, line, code); a column-level line is added when the pack asks for it.
tools/rrlab/rrlab score javascript eslint-curly
tools/rrlab/rrlab score javascript eslint-curly --all               # every fixture of the recording
tools/rrlab/rrlab score javascript eslint-strict --options '{"sourceType": "script"}'

# Run a rule on files. Options are rerail.policy/v1 `options`; an ESLint array is also accepted.
tools/rrlab/rrlab run packs/javascript/eslint-curly.rr a.js b.ts --options '["multi", "consistent"]'

# Inspect what the rule lowered to, and what a pattern compiles to.
tools/rrlab/rrlab lower packs/javascript/eslint-one-var.rr > /tmp/one-var.dl
tools/rrlab/rrlab pattern typescript 'foo($A, $$$REST)'
tools/rrlab/rrlab pattern typescript 'class $C { $$$ }' class_body  # with a selector, as `at "class_body"`
```

How `score` finds its inputs:

- **Recording:** `packs/<pack>/upstream/<tool>/fixtures.expected.json` plus `fixtures/`, read from `--data DIR`,
  else `$RRLAB_DATA`, else this checkout, else the sibling checkout `../Data-layer-rule-expressivity`.
- **Upstream rule:** looked up by `<rule-id>` in `packs/<pack>/conformance.json`. A rule that file does not list
  is scored by its `.rr` `code`.
- **Rule file:** defaults to `docs/rule-syntax/packs/<pack>/<rule-id>.rr`.
- **Explained mismatches:** unmatched points on a line that `divergences.json` names are explained, as in
  `conformance/languages/score.ts`.
- **Options:** every fixture runs at default options, as the recording does, unless `--options` is given. When
  a recording carries `source.units[path].languageOptions`, files run in groups with that `sourceType`
  (and `globalReturn`/`impliedStrict`) as options. Today's recordings carry none; see the strict row below.

Work files go to `tools/rrlab/target/work/<rule>/`: `rule.dl`, `in/*.facts` and `out/*.csv`. They are kept
for inspection.

## Data-layer rules (`rrlab dl`)

```
# Score data-layer/bootstrap-cache-health.rr on the data-layer port's recorded linter calls
tools/rrlab/rrlab dl bootstrap_cache_health
# Only one entry function, or the tests whose name contains a text
tools/rrlab/rrlab dl lazy_architecture --entry scan_files --test deleted_cold
```

The events are the upstream test suite's linter calls, as the data-layer port recorded them. They come from the
Data-layer-rule-expressivity worktree's `fixtures/host/data-layer-port`:
- `batch-NN-recording.json` and `*probes.json`;
- `batch-NN-complete-api.json[.gz]` for batches 9 and 10.

Each event runs on its own files and is compared as a set of (path, line, code) keys.

Many events call one of the linter's internal functions, and such a function reports only some of the linter's
codes. Each event is therefore compared on the codes its entry function reports in any event of the check.

Scope and options:
- A direct call ignores `.scope`.
- A production (plugin) event keeps `.scope` and drops the rule's `.mode log` codes.
- An event's options are passed as `option` rows.

`dl.py` has the details.

## Scores (2026-11-05)

On the Data-layer-rule-expressivity recording (ESLint 10.11.0, eslint-plugin-n 18.4.1), matched on
(path, line, code). Each run takes under 11 s wall (under 6 s CPU) on all 10,344 fixtures.

| Rule | Own fixtures | Every fixture |
|---|---|---|
| eslint-curly | 239 TP / 0 FP / 0 FN | 432 / 0 / 0 |
| eslint-one-var | 222 / 0 / 0 | 422 / 0 / 0 |
| eslint-sort-imports | 43 / 0 / 0 | 73 / 0 / 1 |
| eslint-strict | 55 / 23 / 26 (with `sourceType: script`: 75 / 9 / 6) | 83 / 32 / 679 |
| n-no-unsupported-features-node-builtins | n/a: no fixtures recorded | 0 / 4 / 0 (rule not recorded) |

The column-level scores are the same, except for strict.

Notes on the rows that are not clean:

- **strict:** the recording drops each RuleTester case's `languageOptions`. A `sourceType: "script"` case is
  therefore read as a module, which flips the rule's mode. This is a harness gap, not a rule gap: forcing
  `script` lifts the rule's own fixtures to 75 / 9 / 6, and what remains is cases with other options
  (`globalReturn`, `impliedStrict`, `commonjs`).
- **sort-imports `--all`:** the one miss is `import { default as foo, a } from 'module'`
  (no-unused-vars/393.js). tree-sitter-javascript writes `default` there as an anonymous token, so the specifier
  has no `name` field and `lib/tsx/names.rr`'s `import_binding` gives no row. This is a library gap.
- **node-builtins:** the eslint-plugin-n recording does not run this rule. Its four findings over every fixture
  are `import.meta.dirname`/`filename` under the fixtures' `engines` range, which looks right. Use `run` for
  ad-hoc cases (a `package.json` beside the files gives `engines.node`).

## What is modelled

### Languages

| Language | Extensions | Grammar source |
|---|---|---|
| javascript | `.js` `.mjs` `.cjs` `.jsx` | `native/grammars`, generated at build time with tree-sitter-generate 0.27.1, as `native/build.rs` does |
| typescript | `.ts` `.mts` `.cts` | same |
| tsx | `.tsx` | same |
| python | `.py` `.pyi` | same |
| vue | `.vue` | same |
| sql | `.sql` | committed `parser.generated.c.gz` |
| rust | `.rs` | crates.io tree-sitter-rust 0.24.2, from the offline registry |
| php | `.php` | crates.io tree-sitter-php 0.24.2, from the offline registry |

- `--lang` forces a grammar. Any other file is read as text only (`source`, `src_line`, no tree).
- **Blade:** no grammar is vendored or in the cargo registry, so it is skipped.
- **Vue:** the SFC grammar, and its code parsed again as guest trees of the same document, at the file's own lines
  and columns (everything outside the code is blanked, newlines kept). Each guest has a core `parsed(host, grammar,
  root)` row; a guest root has no parent edge to its host, and a guest's syntax errors are not reported:
  - a `<script>` block's raw_text: "javascript", or "typescript" / "tsx" by its `lang`;
  - a directive's value, a `{{ }}`'s text and a dynamic argument `:[x]`: "vue-expression", wrapped as `( … )`;
  - a v-on value: "vue-statements", as written;
  - v-slot, `#x`, slot-scope and scope: "vue-params", wrapped as `( … )=>0`;
  - v-for: its alias as "vue-for-alias", wrapped as `let [ … ]=0` (holes kept), and its list as "vue-for-source",
    wrapped as `( … )`; the split is Vue's forAliasRE.

### Facts (`rrlab-facts facts OUTDIR --root DIR FILE…`)

All documents share one id space, and id 0 is `unset`. Every relation below is written, even when it has no rows.

- **Exact:**
  - Documents and paths: doc (the module is the path without extension; Python's is dotted, with `src/` and
    `__init__` handled), pkg, path_part, file_name.
  - Tree: node, kind, field, child, op, op_at (punctuation left out; Python's `not in`/`is not` are one token),
    next_sibling, precedes.
  - Positions and text: text, line, end_line, col, docof.
  - Source: source, src_line (Python splits lines as `str.splitlines` does), line_count.
  - Tokens: first_token, last_token, token_before, token_after.
  - Order: visit_index (preorder), ast_depth.
  - Also: spread, unset.
- **Approximate:**
  - value / str / strprefix / value_type: string literals unescaped, JS templates without `${}`, JS numbers
    normalised, Python and Rust integers, and the boolean, None, null and undefined keywords. A literal with
    interpolations (a Python f-string with `{x}`, a PHP "…$x…") has strprefix but no str or value, as core.rr
    and the engine have it.
  - name: identifier-like nodes and dotted chains.
  - comment: the text without its marker.
  - static_value: the same as value (no constant following).
  - syntax_error: the first tree-sitter ERROR or MISSING node. For Python this is not Ruff's verdict.
  - visit_index: start order. That differs from CPython's ast order: decorators, defaults, annotations.
- **From the files under `--root`** (computed in rrlab.py): file, package, package_field (keys joined with `/`,
  list indices as segments, as `values.rr` reads `engines/node`), package_for (the nearest `package.json`),
  option and option_item.
- **Policy-file inputs:** `.input r(IO=json|lines|file, filename=…)` is read from `--root`. `IO=json` takes
  `path` but not `*` steps or `records`.
- **Empty, with a warning when a rule uses them:**
  - dfs_first, dfs_first_memo, match_all, str_part, string_annotation; parsed outside Vue files (a rule whose
    `.language` is not vue gets the warning).
  - signed_list and the git_* relations.
  - tsconfig, tsconfig_for, tsconfig_path, package_export.
  - input_error, list_len.

### Lowering (`lower.py`)

- **Violations:** `.violation` becomes `.decl` + `.output`. The code (or code column), the message and the
  `suppress` lines are kept. Messages fill `{col}` and `{x.line}`; a node column shows its text.
- **Dropped:** rules with `=> edits` (fixes are not applied), `may` rules (`must` rules run as plain rules), and
  the policy directives (`.policy`, `.mode`, `.scope` and so on). `.lattice` is dropped with a warning.
- **Types:** node, span, loc and number subtypes erase to number; relation and symbol subtypes erase to symbol.
- **earlier(a, b):** a `visit_index` comparison inside the same document. Under `!` it reads a materialised
  `rr_earlier`.
- **Axes:** `within`, and `inside`/`has`/`nearest`/`within_stop`/`precedes_stop` with a relation-valued set, get
  one generated relation per (axis, set). They are built over the extractor's `rrlab_parent` edges and a
  generated `rr_setN` membership relation. A set can be a kind string, a relation of kinds, or a relation of
  nodes.
- **Text tests:** glob, prefix, suffix, contains, word, their `i` forms, and match. On a node they read the node's
  `text`. Each one is an `@rr_*` functor, so a regex is the same price as a glob, as core.rr asks.
  - Regexes are Python- or Rust-style: `(?i)`, `(?P<x>)`, `(?<x>)`, `\A`, `\Z` are mapped onto `std::regex`
    ECMAScript.
- **Captures:** `(?<X>…)` binds X to the captured *text*. A capture used as a span (a finding's `at`, an edit
  target) is warned about, not modelled.
- **split_dot:** becomes functors, whichever way it is bound.
- **Aggregates:** count, sum, min, max and mean are Soufflé's own. `join x by k sep s` becomes rows, a rank by
  (k…, x), a concatenation chain, and `""` when there are no rows. Symbols compare by bytes, which for UTF-8 is
  code-point order.
- **ordered by earlier:** a self-use under `!` or inside an aggregate reads `R__prev`. rrlab reruns Soufflé until
  R stops changing, at most 60 rounds. one-var takes 2.
- **Suppression:** a `suppress eslint "rule"` line honours `eslint-disable[-line|-next-line]` and
  `eslint-enable`, with or without rule lists. A `suppress ruff|flake8` line honours `# noqa[: codes]`.
- **Not withheld:** a document with a syntax error still reports. The engine withholds it, so `score` stars
  such files (`*`).

### Functors (`functors/functors.cpp`)

- **Exact:** dot, qname, pkg_up, head, dir, join, glob_sub, package_of (the identity, as core.rr says),
  replace_all (`${1}` becomes `$1`), slice, count_matches, json_str (`json.dumps` escaping, surrogate pairs),
  sha256, and place. place hands out a span id, and its path and line go to a side table rrlab reads back.
- **Approximate:**
  - semver_valid / semver_subset: ranges as unions of intervals over major.minor.patch, prerelease tags ignored.
    Supported: `||`, space-AND, `<`/`<=`/`>`/`>=`/`=`, `^`, `~`, x-ranges, hyphen ranges, and a leading `v`.
  - lower: `towlower` per code point.
  - convert: camel, snake, kebab, pascal and the like, on ASCII words.
  - to_precision: `%e`/`%f` with JavaScript's exponent rule.
- **Stubs**, with a warning when used: nfkc (the identity), tree_digest, unparse, ed25519_ok (always 0).

### Patterns (`patterns.py`)

`n ~ \`pattern\`` (and `!n ~ …`) parses the pattern with the policy language's grammar and emits structural
constraints into a generated `rr_patN(root, X…)` relation:

- **Named nodes:** a node becomes `kind`, its fields `field`, and its children `child` with their position.
  The number of named children must match exactly, as in ast-grep; comments are not counted.
- **Leaves and tokens:** keywords and operators become `op`, and named leaves compare their `text`.
- **Metavariables:**
  - `$X` binds X to the node; a repeated `$X` compares text with the first.
  - `$_` is any node.
  - `$$$X` and `$$$` are any run in a list: children before the run are pinned from the start, children after
    it from the end.
- **Statement wrapper:** a metavariable that fills a whole statement (`$B;`) binds that statement.
- **Selector:** `at "kind"` takes the pattern's node of that kind as the match.
- **Root:** the outermost node whose text is the whole pattern.
- **Parsing:** JavaScript-family patterns parse as written. Others write `µ` for `$` (as ast-grep does). Rust
  snippets are wrapped in a function, and PHP gets `<?php `.
- **Not covered** (reported as `partial` in the output):
  - metavariables inside a token, such as a string's contents or an operator;
  - more than one `$$$` in one list (positions after the first are loose);
  - patterns that do not parse.
- **Exact text:** text is compared exactly at named leaves, whitespace included inside a token.

## Known gaps

- Fixes (`=> edits`, `.edit`) are parsed and dropped. Nothing applies them.
- Findings are reported at the `at` node's start (line, column). Spans from captures are text only.
- Facts are rebuilt on every run. There is no caching or incremental work; the whole recording takes seconds.
- Documents with tree-sitter errors still report findings (see Lowering).
- Not modelled: three-valued `must`/`may` (only the must side runs), `.gate`, `.budget`, `.on`, `.scope` and
  `.mode`.
