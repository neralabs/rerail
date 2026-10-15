# Rerail fixtures

Every test input and oracle the repository's tests read, in one place: language conformance data (inputs, the
reports and values upstream tools record for them, `*.expected.json`, and the `generate_*` scripts that record each),
the host tests' data and the pack validator's inputs. Nothing here is published as a package or shipped to users: no
package's `files` includes this directory. A rule pack's own tool recordings live with the pack, in
`packs/<id>/upstream/<tool>/` ([pack format](../docs/pack-format.md)); everything else is here.

Tests resolve this directory in one place per language: Rust through `native/build.rs` (the `fixtures_dir!` and
`oracle!` macros of `native/src/code_rule_dsl/mod.rs` read what it found), TypeScript through `fixturesPath()` in
`test/support/oracles.ts`. Both read `RERAIL_ORACLES_DIR` (relative to the repository root) when it is set, else this
directory.

Maintainers run the generators by hand against the pinned upstream versions their headers name; CI and the product
never do.

## Layout

`<family>/<feature>/`: one directory per language family, then the engine module the data tests, named exactly as
the module under `native/src/code_rule_dsl/` (`scope/`, `value/`, `compile/`, …). Below the feature, directories are
kebab-case and name the part of the module that reads them (`php/compile/format-args/` is the oracle of
`compile/format_args/php.rs`). Three groups are not engine modules:

- `<family>/licenses/<project>/`: third-party records (below).
- `<family>/upstream/<tool>/`: a language's upstream linter recordings, recorded ahead of its pack (below).
- `host/<area>/` and `packs/<pack>/`: not language families. `host/` holds the TypeScript host tests' data, one
  directory per area; `packs/` holds pack-shaped test inputs: the pack validator's and the conformance runner's
  ([README](packs/README.md)).

`node scripts/check-structure/fixtures.mjs` prints the table below from the tree and fails on a feature directory that
names no engine module, and on fixture data outside `fixtures/` and `packs/<id>/upstream/`.

| Feature directory | Engine module | What reads it |
|---|---|---|
| `blade/blade/` | `blade` | the compiled view, against illuminate/view and PHPStan (`blade/oracle_tests.rs`); `forte/`, the Forte port's parity with Forte 1.1.0 (`blade/forte/tests.rs`), and `grammar-gaps/`, the templates tree-sitter-blade failed on |
| `{ecma,python}/compile/examples/` | `compile` | the worked examples (`compile/examples_tests.rs`, the DSL reference's coverage) |
| `php/compile/format-args/` | `compile` | `formatArgs` with style `printf`, against PHPStan (`compile/format_args/php.rs`) |
| `{ecma,python,php,sql}/scope/` | `scope` | name resolution and usedness; `sql/scope/` is PostgreSQL's reading of names (T2) and the corpus input (T3) |
| `{ecma,python,php,sql}/value/` | `value` | constant values; `php/value/compare/` is `compare`'s PHP keys, names and structures (`value/php_compare`, `value/php_structure`) |
| `{python,php}/calls/` | `calls` | call graphs |
| `python/identifier/` | `identifier` | identifier normalisation |
| `ecma/lines/` | `lines` | line and comment positions |
| `{python,php,rust}/literal/` | `literal` | format strings, doc comments and literals |
| `{ecma,python,php,rust,sql}/suppress/` | `suppress` | suppression comments |
| `{ecma,python}/matcher/ast-grep/` | `matcher` | relational rules, `field`, `nthChild`, `stopBy`, `utils` against ast-grep (`test/code-rule-dsl-upstream-differential.test.ts`) |
| `{php,sql}/matcher/arguments/` | `matcher` | argument slots (`matcher/arguments.rs`; SQL's, PostgreSQL's binding, `matcher/arguments_sql.rs`) |
| `{rust,sql,vue}/language/` | `language` | measurements that choose a planned language's registry entry: candidate grammars (`grammar/`, T1), Rust's capability census (`feasibility/`), Vue's embedded view (`view/`) |
| `host/benchmark/` | — | the query-parity benchmark's semantic catalogs (`benchmark/query-parity.ts`) |
| `host/code-rule-dsl/` | — | engine-contract rule documents (`test/native-code-rule-dsl-runtime.test.ts`) |
| `host/constraint-code-rules/` | — | constraint examples (`test/engine/code-rule/code-rule-dsl-constraints.test.ts`) |
| `host/database/` | — | the database schema at the migration baseline (`test/engine/storage/database-baseline.test.ts`) |
| `host/mixed-repository/` | — | a repository mixing four languages (`test/mixed-repository.test.ts`) |
| `host/session-enforcement/` | — | golden output of each coding agent in the session enforcement design's scenes: hook JSON, session notes, the OMP steer renderer and the footer (`test/hosts/*/session-enforcement.golden.test.ts`, `test/hosts/omp/steer-renderer.test.ts`, `test/presentation/footer-golden.test.ts`); review a change with design |
| `host/tool-call-rules/` | — | tool-call rules (`test/tool-call-rules.test.ts`, the rule store and catalog tests) |
| `packs/` | — | pack validator and conformance runner inputs (`test/packs.test.ts`, `test/language-conformance.test.ts`) |

| Family | Upstream tools |
|---|---|
| `python/` | Ruff 0.16.9, Pyflakes 3.4.0, Bandit 1.8.6, CPython 3.13, Jedi 0.19.2 |
| `ecma/` | ESLint 10.11.0, typescript-eslint 8.70.1, eslint-utils 4.10.1, natural-compare 1.4.0, ast-grep 0.45.3 |
| `php/` | PHP 8.5.11, PHPStan 2.2.16, Psalm 6.19.1, PHP_CodeSniffer 4.0.4 and 3.13.6, nikic/php-parser 5.9.0, phpstan/phpdoc-parser 2.3.5 |
| `blade/` | illuminate/view 13.34.0, PHP 8.5.11, PHPStan 2.2.16, Forte 1.1.0 |
| `vue/` | @vue/compiler-sfc 3.5.43, vue-eslint-parser 10.4.1, eslint-plugin-vue 10.11.1, ESLint 10.11.0 |
| `rust/` | rustc, clippy and rust-analyzer 1.99.0, tree-sitter-rust 0.24.2, syn 2 |
| `sql/` | SQLFluff 4.4.0, PostgreSQL 18 parser (pglast 8.5), SQLite 3.50.4, MySQL 8.4.11 |

`ecma` is shared by JavaScript and TypeScript. The rule packs (python, php, blade, typescript, react, node, next, vue,
complexity, sql) live in `packs/<id>/` ([pack format](../docs/pack-format.md#moving-to-this-layout)): their upstream
tools' copied tests and recordings, their candidates and their licence records. `rust/` has upstream recordings but no
rules yet, so it has no pack directory: `rust/upstream/` holds the upstream linters' recorders, their copied tests
(`<tool>/fixtures/`) and recordings on those tests and on pinned public corpora (`<tool>/corpus-<name>.expected.json`),
laid out as a pack's `upstream/`, beside the planned rule list (`rust/upstream/conformance.draft.json`). When a
language's pack gains its first rules, its draft becomes `packs/<id>/conformance.json` and `<id>/upstream/` moves to
`packs/<id>/upstream/`. `../conformance/recorders/eslint-record.mjs` is the recorder the ESLint-based generators share.

Each oracle stays whole inside its feature directory (generator, inputs and recorded output together), so every
generator reads and writes relative to its own directory; a family's shared generator modules sit at the family's
top (`sql/common.py`, `vue/tools.mjs`). A generator that needs pinned tools or corpus checkouts takes them as
arguments or finds them under `.rerail/tools/<pack>` and `.rerail/corpora/<pack>` in the nearest ancestor checkout
(gitignored there); its header says which.

## Third-party material

Some fixtures are copied from upstream projects. Each family keeps them under `<family>/licenses/<project>/`: the
upstream licence, copied verbatim at the pinned version, and an `UPSTREAM.md` listing every copied file with its
upstream path and whether the copy is verbatim, derived or an excerpt. Those files stay under the upstream licence;
`REUSE.toml` maps each of them to it, and `reuse lint` must pass.

| Record | Upstream | Licence |
|---|---|---|
| `python/licenses/ruff/` | Ruff 0.16.9 (its `LICENSE` carries Pyflakes' notice) | MIT |
| `python/licenses/bandit/` | Bandit 1.8.6 | Apache-2.0 |
| `ecma/licenses/eslint/` | ESLint 10.11.0 | MIT |
| `ecma/licenses/typescript-eslint/` | typescript-eslint 8.70.1 | MIT |
| `ecma/licenses/eslint-utils/` | eslint-utils 4.10.1 | MIT |
| `php/licenses/laravel/` | laravel/framework e5f3d6265f | MIT |
| `php/licenses/symfony/` | symfony/symfony 2c4011aa7e | MIT |
| `blade/licenses/bagisto/`, `bookstack/`, `filament/`, `livewire/` | directive closures excerpted into `blade/blade/directives.php`; templates in `blade/blade/forte/corpus/` | MIT |
| `blade/licenses/forte/` | fortephp/forte 1.1.0 (its test suite's templates in `blade/blade/forte/suite.expected.json`) | MIT |
| `vue/licenses/eslint-plugin-vue/` | eslint-plugin-vue 10.11.1 | MIT |
| `rust/licenses/clippy/` | rust-clippy (Rust 1.99.0) | MIT OR Apache-2.0 |

Verbatim copies keep the upstream licence alone; derived files and excerpts are under the upstream licence and
Apache-2.0 together.

Adding an oracle that copies upstream material: list each copied file in the family's
`licenses/<project>/UPSTREAM.md`, adding the project's licence file there first if it is new to the family, and add
the file to that project's entry in the repository's `REUSE.toml` (paths start with `fixtures/`). Recording an
upstream tool's output does not copy its source. Only permissively licensed material is accepted: the repository's
licence check rejects any licence outside its allowlist.

## Licence

Apache License 2.0 for everything that is not third-party material listed above. See the repository's
[LICENSE](../LICENSE) and [NOTICE](../NOTICE).
