# Repository structure

One place for rules, one place for fixtures, one place for the engine. This page states the layout as
rules a machine checks. The structure gate, `npm run check:structure`, checks every rule; it runs in
`npm run check`, so CI's lint job fails when the layout regresses. The metrics are the 24 of the
structure review on PR #1 plus #25, the generated-file manifest.

## Layout

| Path | Holds |
|---|---|
| `src/engine/`, `src/hosts/<harness>/`, `src/<layer>/` | The TypeScript package, one directory per layer, each with an `index.ts` entry point |
| `native/` | The Rust engine; it compiles only from here (#4) |
| `packs/<id>/` | A rule pack: `pack.json`, `package.json`, `README.md`, `LICENSE`, `NOTICE`, `rules/` (shipped), `candidates/` (scored, never shipped), `upstream/<tool>/` (recordings), `licenses/` |
| `fixtures/<family>/<feature>/` | Engine and host test data; `<feature>` names an engine module (#10) |
| `fixtures/packs/` | Pack-validator inputs |
| `test/`, `integration/` | Unit and integration tests; helpers in `support/` |
| `playground/` | The rule playground, its own npm package |
| `scripts/generate/` | Generators of engine, host and pack data, listed in `scripts/generate/README.md` |

## Running the gate

```sh
npm run check:structure                       # table, then every finding of each failing metric
node scripts/check/check-structure.mjs --only 4,12  # selected metrics
node scripts/check/check-structure.mjs --json       # machine output: [{ id, title, target, actual, pass, findings }]
```

The gate exits 1 when any metric fails and prints each finding as `path[:line]: message`. Its only
inputs are `git ls-files` (tracked files present in the working tree) and file contents: no build, no
network, no clock. Two runs print the same bytes. New files count once they are tracked (`git add`).

Each metric is a module, `scripts/check/check-structure/metrics/<NN>-<slug>.mjs`, exporting
`{ id, title, target, check(repo) }`; `check` returns `{ actual, findings: [{ path, line?, message }] }`
and the metric passes when there are no findings. The module's header comment is the authoritative
predicate. Exemptions exist only as named rules inside a module, each with its reason. Shared code lives
beside the metrics: `repo.mjs` (the repository view; `memoryRepo` builds one in memory for tests),
`lib.mjs`, `generated.mjs` (#6, #20, #25) and `fixtures.mjs` (#3, #10). Tests: `test/structure/`, where
`boundaries.test.mjs` covers the layer boundary lint #13 runs.

Terms used below. A *rule document* is a JSON file whose `format` is `rerail.policy/v1`. A *test file* is
a `*.test.*`/`*.spec.*` script, a pytest `test_*.py`/`*_test.py` or a PHPUnit `*Test.php`. The *data
roots* are `fixtures/**` and `packs/<id>/upstream/**`: recorded or copied material, not the repository's
own code.

## Location

### 1. `pack.json` outside `packs/<id>/` — target 0

Every `pack.json` is `packs/<id>/pack.json`.
Exemption `validator-inputs`: `fixtures/packs/**/pack.json` are pack-validator test inputs.

### 2. Rule documents outside `packs/*/{rules,candidates}` — target 0

Every rule document is under `packs/<id>/rules/` or `packs/<id>/candidates/`.
Exemption `engine-test-inputs`: a rule document may be an engine or host test input under `fixtures/`
(`fixtures/<family>/compile/…`, `fixtures/host/…`), never shipped (#23). No directory on its path below
`fixtures/` may be named `rules`, `candidates`, `probes` or `policies`: those names mark rule sets, which
belong in a pack. A benchmark or test that needs a pack's rule references the pack's document.
Exemption `validator-inputs`: `fixtures/packs/**`, whose pack format puts rule documents in `rules/`.

### 3. Fixture data outside `fixtures/` and `packs/*/upstream` — target 0

A file is fixture data when a directory in its path is `fixtures`/`__fixtures__`; it is a
`*.expected.json` recording or `*.cases.json` case list; it is a code-rule document; it sits in a test
root (`test/`, `integration/`, `conformance/`, `benchmark/`) and is not TS/JS source or Markdown; or it sits
in `test/`/`integration/` with a fixture name. It may live only under `fixtures/` or `packs/<id>/upstream/`.
Exemptions: `packs/<id>/candidates/**` (pack data) and, for code-rule documents, `packs/<id>/rules/**`.

### 4. Engine source roots — target 1 (`native/`)

No `include_str!`/`include_bytes!`/`include!` in a `native/**/*.rs` file resolves outside `native/`
(string-literal paths against the including file, `concat!(env!("CARGO_MANIFEST_DIR"), "…")` against the
crate; other arguments such as `fixtures_dir!()` or `OUT_DIR` are skipped), and nothing is tracked under
the retired `code-rule-dsl/`.
Exemption `test-inputs`: a test-only include may read `fixtures/**`, `packs/**` or a `*.generated.*` file.
An include is test-only in `tests.rs`, `*_tests.rs`, `test_support.rs` or under `native/tests/`, or inside
the item a `#[cfg(test)]` attribute applies to: its brace-matched body (comments and string and character
literals skipped), or up to its `;` when it has none. `#[cfg(test)] mod tests;` declares a module in
another file and exempts nothing after it.

### 5. Test roots — target `test/`, `integration/` (+ `playground/`)

Every test file is under `test/` or `integration/`.
Exemptions: `playground-package` (`playground/` is its own package) and `data-roots` (upstream projects'
copied tests). Rust `#[cfg(test)]` modules are part of the crate and not counted.

### 6. Generators follow one known pattern — target 0 misplaced

A generator is every manifest row's generator (`scripts/generate/README.md`, #25), every path a generated
file's `Generated by <path>; do not edit.` header names, and any script named `generate`, `generate_*` or
`generate-*` (extension `.mjs .cjs .js .ts .mts .py .php`). Each is under `scripts/generate/`, never beside
its output in `native/src/` or `src/`, whatever its name (`scripts/generate/schema/schema-sql.ts`, not
`src/engine/storage/schema-sql.ts`).
Exemption `data-roots`: recorders sit beside the recordings they write, under `fixtures/` or
`packs/<id>/upstream/`; generated files there are not scanned for headers.

### 7. One location pattern for licences — target 100%

For each rule document of a pack, REUSE's resolved licence (REUSE.toml: the last matching
`[[annotations]]` applies; `override` wins outright, otherwise an in-file `SPDX-License-Identifier` or a
`.license` sidecar wins, `aggregate` combines), compared as a set of `AND` terms with the pack's
`package.json` `license`:

- `packs/<id>/rules/**`: equal (the package's licence describes exactly what it ships);
- `packs/<id>/candidates/**`: only terms the pack declares (Rerail's own probe rules are Apache-2.0 alone).

## Contract conformance

### 8. Packs pass the pack-format layout — target 100%

Every `packs/<id>/` has `pack.json`, `package.json`, `README.md`, `LICENSE`, `NOTICE`; `<id>` is
lowercase letters, digits and dashes; `pack.json` is `rerail.rule-pack/v1` with `id` = `<id>`, lists only
tracked documents, and lists every file in `rules/`; `package.json` is `@neralabs/rerail-pack-<id>`, has a
`license`, no `dependencies`, and `files` publishing `pack.json`, `rules`, `LICENSE`. The compile half of
the [validation contract](pack-format.md#the-validation-contract) runs in the pack tests.

### 9. Pack directories that import the engine — target 0

No JS/TS file under `packs/` imports a path resolving into `src/`, or `@neralabs/rerail[/…]`.

### 10. `fixtures/<family>/<feature>` maps to an engine module — target 100%

Every `fixtures/<family>/<feature>/` names a top-level module of `native/src/code_rule_dsl/`
(`<feature>.rs` or `<feature>/`, not `mod.rs`/`test_support.rs`) or a documented group: `upstream` (tool
recordings ahead of a pack) or `licenses`.
Exemption `non-engine-families`: the children of `fixtures/host/` and `fixtures/packs/` are areas.

### 11. No orphan fixture families or packs — target 0

Every `fixtures/<family>/` has `languages/<family>.json` or `packs/<family>/`, or is a declared shared
family: `ecma` (ECMAScript recordings shared by the javascript and typescript languages and their packs),
`host` and `packs` (not tied to a language). Every pack holds at least one rule document in `rules/`, or in
`candidates/rules/` while it ships none because all its ports are candidates.

## Layering

### 12. Max files in one flat code or test directory — target ≤25

A directory holding a `.ts .tsx .mts .cts .js .mjs .cjs .rs .py .php .sh` file directly holds at most 25
tracked files directly.
Exemptions: `pack-data` (`packs/<id>/{rules,candidates,upstream,licenses}/**`, one file per rule or
recording), `fixture-data` (`fixtures/**`), `vendored` (`native/grammars/**`).

### 13. Import-direction violations — target 0

This metric is the result of the layer boundary lint, `scripts/check/check-boundaries.mjs`
(`checkBoundaries(root, { files })`; run alone as `npm run check:boundaries`). Its rules, stated in its header
and tested in `test/structure/boundaries.test.mjs`: `engine-isolation` (`src/engine/` imports no other
layer), `host-isolation` (no host imports another), `layer-order` (each layer imports only the layers
listed for it, the order in `src/README.md`), `src-self-contained` (`src/` imports nothing outside `src/`),
`entry-points` (code outside `src/` imports a layer's `index.ts` or `src/index.ts`; unit tests under
`test/<layer>/` may import their own layer's internals, and a vitest mock seam names the module it
replaces) and `playground-public-api` (`playground/` imports only the package's `exports`, in code and in
tsconfig `paths`). The metric fails when the lint is not tracked.

### 14. Production reads `fixtures/`; one root resolver per language — target 0; ≤1 per language

A file resolves a fixtures root when it writes, on a path-building line (`join(`, `resolve(`, `URL(`,
`Path::new(`, `PathBuf::from(`, `||`, `??`, pathlib ` / `), a relative path literal whose first component
is `fixtures`; or the bare segment `"fixtures"` as a direct argument of a `join(`/`resolve(` call whose
earlier arguments are one root (any balanced expression) and then only `"."`/`".."` segments, the call
possibly spanning lines (`join(here, "..", "fixtures")`, `resolve(process.cwd(), "fixtures")`,
`path.join(repoRoot(), "fixtures")`); or the bare segment anywhere on a path-building line of a file that
reads the `RERAIL_FIXTURES_DIR` override. Compared literals (`startsWith(`, `===`, …), name lists and
deeper segments (`join(root, "labels", "fixtures")`) do not count. No file under `src/` resolves one; per
language (Rust, TypeScript/JavaScript, Python, PHP, shell) at most one file does.
Exemptions: `data-roots` (generators write beside themselves) and `structure-gate` (this gate and its tests).

### 15. Duplicate concepts — target 0 unexplained

- `rule-id`: no rule `id` is defined by two rule documents across `packs/*/{rules,candidates}/**`.
  Exemption `option-variant`: documents under `packs/<p>/candidates/<rule>/` (any depth) whose id is
  `<rule>` or ends `:<rule>` may share it with each other and with `packs/<p>/rules/<rule>.json`.
- `pack-format`, `conformance-format`: all `pack.json` files share one `format`, and all
  `conformance.json` files share one (absent = `untagged`); files differing from the most common are
  reported. Exemption `validator-inputs`: `fixtures/**` (deliberately malformed inputs included).
- `harness-import`: code in a pack-specific directory (`packs/<id>/`, `conformance/<id>/`,
  `test/conformance/<id>/` for a pack `<id>`) is imported only by code for the same pack.

## Naming

### 16. `rules/` directories holding non-rules — target 0

Every file below a `rules/` directory is a rule document.
Exemptions: `data-roots` (upstream trees) and `vendored`.

### 17. Test helpers in a support directory — target 100%

Every non-test script under `test/` or `integration/` has a `support/` directory in its path.
Exemptions: `ambient-declarations` (`*.d.ts`) and `fixture-trees` (below a `fixtures/` directory: data, #3).

### 18. One case convention per language — target 100%

TS/JS file basenames (first dot part; later parts lowercase) and their directories are kebab-case
(dot-directories skipped); Rust file names and module directories below `src/`/`tests/` are snake_case;
Python file names are snake_case. The `.generated` infix (#20) is not part of a stem: `builtins.generated.rs`
is checked as `builtins`.
Exemptions: `data-roots` (named after upstream tools and engine modules), `vendored`, `fixture-trees`
(framework route files such as `pages/_app.tsx`), `react-components` (under `playground/ui/src/`, a
component's `.tsx` file is PascalCase and a hook's file is `useThing`).

## Hygiene

### 19. Large binaries committed without LFS — target 0 (>100 KB)

Every tracked binary (NUL byte in the first 8000 bytes, or a known binary extension) over 100 KB matches a
`.gitattributes` line setting `filter=lfs`, the last matching line winning (`-filter`, `!filter` or another
`filter=` unsets). Patterns follow gitattributes: no `/` matches the basename anywhere, otherwise anchored to
the declaring `.gitattributes`. A checkout without LFS content holds text pointers, which pass.

### 20. Generated files are marked and grouped — target 100%

A file is generated when named `*.generated.*` or when its first five lines say "generated by", carry the
at-sign generated tag, or say "do not edit". Each is named `*.generated.*` or listed in
`scripts/generate/README.md`, and when commentable (not JSON, not binary) carries
`Generated by <generator path>; do not edit.` within its first three lines, naming a tracked file.
Exemptions: `data-roots` (recordings follow the fixtures convention), `tool-owned` (`*.lock`,
`package-lock.json`, `.changeset/README.md`); the manifest and this gate are not scanned.

### 21. Top-level directories have a README — target 100%

Every top-level directory has `README.md`.
Exemptions: `dot-directories` (tool configuration) and `reuse-licenses` (`LICENSES/` holds only licence texts).

### 25. Generated-file manifest is consistent — target 0

`scripts/generate/README.md` is a markdown table: first column the generated path (a path ending `/` is a
directory of generated files), second the generator, both in backticks. It exists whenever a
`*.generated.*` file does; every row's generated path and generator are tracked (the generator under
`scripts/generate/`, #6); every `*.generated.*` file is covered by a row; a header naming a generator
names the row's.

## Found during the review

### 22. Stale paths left from the import — target 0

No tracked text file mentions `rerail-fixtures/`, `tempo-oracles`, `code-rule-dsl/rules/opt-in` or
`policies/rule-packs`.
Exemptions: `history` (`CHANGELOG.md`, `.changeset/*.md`) and `structure-gate` (this page, the gate, its tests).

### 23. Shipped rules come only from `packs/*` — target `packs/*` only

Each root `package.json` `files` entry that ships rules (it covers a rule document or a `pack.json`, or names
a `rules` directory, empty or not) starts with `packs/<id>/`.

### 24. Every test root and harness runs in CI — target 100%

From every `run:` script in `.github/workflows/ci.yml`, expanded through npm scripts transitively
(`npm run`, `npm test`, `npm --prefix <dir>`, `cd`), the gate collects what runs: `vitest` (by `--dir`,
`--config` directory, filters, minus the package's `vitest.config.*` `exclude` globs), `node --test`,
`pytest`, and entry points (`node`/`tsx`/`python`/`bash` files). Every test file outside the data roots
runs, and every entry point that an npm script named `test*` or `conformance*` runs also runs in CI.
Rust tests run through `cargo test`, outside this metric.

## On `main` (`fbb6104f`)

The gate's first run, before the structure work, with the predicates of that time (#4, #6, #13 and #14
have since been tightened):

| # | Metric | Target | Actual | Result |
|---|---|---|---|---|
| 1 | `pack.json` outside `packs/<id>/` | 0 | 6 | FAIL |
| 2 | Rule docs outside `packs/*/{rules,candidates}` | 0 | 29 in 8 dirs | FAIL |
| 3 | Fixture data outside `fixtures/` and `packs/*/upstream` | 0 | 103 | FAIL |
| 4 | Engine source roots | 1 (native/) | 2 | FAIL |
| 5 | Test roots | test/, integration/ (+ playground/) | 5 (conformance, integration, playground, scripts, test) | FAIL |
| 6 | Generators follow one known pattern | 0 misplaced | 12 | FAIL |
| 7 | Pack rules carry the pack's licence (REUSE) | 100% | 100% (493/493) | PASS |
| 8 | Packs pass the pack-format layout | 100% | 100% (9/9) | PASS |
| 9 | Pack dirs that import the engine | 0 | 0 | PASS |
| 10 | `fixtures/<family>/<feature>` maps to an engine module | 100% | 72% (29/40) | FAIL |
| 11 | No orphan fixture families or packs | 0 | 0 | PASS |
| 12 | Max files in one flat code/test dir | ≤25 | 108 (src/) | FAIL |
| 13 | Import-direction violations | 0 | 106 in 41 files (fallback scan) | FAIL |
| 14 | Production reads `fixtures/`; one root resolver per language | 0; ≤1 per language | 0; Rust 1, TypeScript 9 | FAIL |
| 15 | Duplicate concepts | 0 unexplained | rule ids 141, pack.json formats 4, conformance.json formats 1, harness imports 5 | FAIL |
| 16 | `rules/` dirs holding non-rules | 0 | 0 | PASS |
| 17 | Test helpers in a support dir | 100% | 15% (2/13) | FAIL |
| 18 | One case convention per language | 100% | 99% (595/601) | FAIL |
| 19 | Large binaries committed without LFS | 0 (>100 KB) | 2 | FAIL |
| 20 | Generated files are marked and grouped | 100% | 0% (0/8) | FAIL |
| 21 | Top-level dirs have a README | 100% | 18% (3/16) | FAIL |
| 22 | Stale paths left from the import | 0 | 4 in 1 files | FAIL |
| 23 | Shipped rules come only from `packs/*` | packs/* only | code-rule-dsl/rules, packs/complexity, packs/tslint | FAIL |
| 24 | Every test root and harness runs in CI | 100% | 97% (150/154) | FAIL |
| 25 | Generated-file manifest is consistent | 0 | 4 | FAIL |

## Adding or changing a rule

Change the metric module (predicate and header comment together), its test in `test/structure/`, and the
section above. A new exemption is a named rule in the module with its reason, never a path list
maintained elsewhere. A new metric is a new `scripts/check/check-structure/metrics/<NN>-<slug>.mjs`; the gate
loads every module in that directory.
