# Rerail fixtures

Language conformance test data: inputs, the reports and values upstream tools record for them (`*.expected.json`),
and the `generate_*` scripts that record each, grouped by language family. Nothing here is published as a package
or shipped to users: no package's `files` includes this directory.

Rerail's code-rule DSL engine tests compare against these oracles. They read this directory, or `RERAIL_FIXTURES_DIR`
when it is set (for Rust through `native/build.rs`, for TypeScript through `test/oracles.ts`).

Maintainers run the generators by hand against the pinned upstream versions their headers name; CI and the product
never do.

## Layout

`<family>/<feature>/`: one directory per language family, then the engine feature or tier the oracle pins, as in
Rerail's `native/src/code_rule_dsl/` and `conformance/languages/`.

| Family | Upstream tools | Features |
|---|---|---|
| `python/` | Ruff 0.16.9, Pyflakes 3.4.0, Bandit 1.8.6, CPython 3.13, Jedi 0.19.2 | `calls`, `identifier`, `literal`, `scope`, `suppress`, `value` |
| `ecma/` | ESLint 10.11.0, typescript-eslint 8.70.1, eslint-utils 4.10.1, natural-compare 1.4.0 | `lines`, `scope`, `suppress`, `value` |
| `php/` | PHP 8.5.11, PHPStan 2.2.16, Psalm 6.19.1, PHP_CodeSniffer 4.0.4 and 3.13.6, nikic/php-parser 5.9.0, phpstan/phpdoc-parser 2.3.5 | `arguments`, `calls`, `compare`, `format_args`, `literal`, `rules`, `scope`, `suppress`, `value` |
| `react/` | eslint-plugin-react 7.37.5, eslint-plugin-react-hooks 7.1.1, ESLint 9.39.5 | `candidates`, `policies`, `probes`, `rules` |
| `vue/` | @vue/compiler-sfc 3.5.43, vue-eslint-parser 10.4.1, eslint-plugin-vue 10.11.1, ESLint 10.11.0 | `grammar`, `rules`, `view` |
| `rust/` | rustc, clippy and rust-analyzer 1.99.0, tree-sitter-rust 0.24.2, syn 2 | `feasibility`, `grammar`, `literal`, `rules`, `suppress` |
| `sql/` | SQLFluff 4.4.0, PostgreSQL 18 parser (pglast 8.5), SQLite 3.50.4, MySQL 8.4.11 | `names`, `rules`, `suppress`, `t1`, `t3`, `value` |

`frameworks/eslint_record.mjs` is the recorder the `next/`, `node/` and `react/` generators share. `rules/` holds a
pack's T4 recordings: each ported rule's upstream test cases (`fixtures/`) and pinned public corpora
(`corpus-<name>.expected.json`). `candidates/` and `probes/` are runnable packs scored against those recordings, and
`grammar/`, `t1/`, `t3/` and `feasibility/` hold the measurement tools of a pack's earlier tiers. A pack that has
moved to `packs/<id>/` ([pack format](../docs/pack-format.md#moving-to-this-layout)) takes its `rules/`,
`candidates/` and its rule tools' licences with it: Python's are in `packs/python/`.

Each oracle stays whole inside its feature directory (generator, inputs and recorded output together), so every
generator reads and writes relative to its own directory. A generator that needs pinned tools or corpus checkouts
takes them as arguments or finds them under `.rerail/tools/<pack>` and `.rerail/corpora/<pack>` in the nearest
ancestor checkout (gitignored there); its header says which.

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
| `php/licenses/phpstan/` | PHPStan 2.2.16 | MIT |
| `php/licenses/psalm/` | vimeo/psalm 6.19.1 | MIT |
| `php/licenses/php_codesniffer/` | PHP_CodeSniffer 4.0.4 | BSD-3-Clause |
| `blade/licenses/bagisto/`, `bookstack/`, `filament/`, `livewire/` | directive closures excerpted into `blade/view/directives.php` | MIT |
| `react/licenses/eslint-plugin-react/` | eslint-plugin-react 7.37.5 | MIT |
| `react/licenses/react/` | facebook/react (eslint-plugin-react-hooks 7.1.1) | MIT |
| `vue/licenses/eslint-plugin-vue/` | eslint-plugin-vue 10.11.1 | MIT |
| `rust/licenses/clippy/` | rust-clippy (Rust 1.99.0) | MIT OR Apache-2.0 |
| `sql/licenses/sqlfluff/` | SQLFluff 4.4.0 | MIT |

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
