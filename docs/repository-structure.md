# Repository structure

One place for rules, one place for fixtures, one place for the engine. The structure gate,
`npm run check:structure`, checks the layout below. It runs in `npm run check`, so CI's lint job fails when the
layout regresses. The metrics are the 24 of the structure review on PR #1 plus #25, the generated-file manifest.

## Layout

Each top-level path has one job.

| Path | Holds |
|---|---|
| `src/engine/`, `src/hosts/<harness>/`, `src/<layer>/` | The TypeScript package, one directory per layer, each with an `index.ts` entry point |
| `native/` | The Rust engine; it compiles only from here (#4) |
| `packs/<id>/` | A rule pack: manifests, `rules/` (shipped), `candidates/` (scored, never shipped) and `upstream/<tool>/` |
| `fixtures/<family>/<feature>/` | Engine and host test data; `<feature>` names an engine module (#10) |
| `fixtures/packs/` | Pack-validator inputs |
| `test/`, `integration/` | Unit and integration tests; helpers in `support/` |
| `e2e/` | The dashboard's Playwright browser specs over seeded scenarios (`e2e/README.md`); helpers in `support/` |
| `plugins/<agent>/` | Installer plugins for agent plugin directories (`plugins/README.md`) |
| `scripts/generate/` | Generators of engine, host and pack data, listed in `scripts/generate/README.md` |

<details>
<summary>What a pack and a plugin directory hold</summary>

- **`packs/<id>/`:** `pack.json`, `package.json`, `README.md`, `LICENSE`, `NOTICE`, `licenses/`, `rules/`
  (shipped), `candidates/` (scored, never shipped), `upstream/<tool>/` (generators and locks; test data, expected
  findings included, is one S3 archive per tool named by its `test-data.lock`, not in git, #3).
- **`plugins/<agent>/`:** the installer plugins listed in the Claude, OpenAI and Cursor plugin directories
  (`claude/`, `codex/`, `cursor/`): one skill each, `install`, running
  `npx @neralabs/rerail@<version> setup --relay`; no hooks or binaries.

</details>

## Running the gate

Run the gate to see which metrics fail and every finding behind them.

```sh
npm run check:structure                       # table, then every finding of each failing metric
node scripts/check/check-structure.mjs --only 4,12  # selected metrics
node scripts/check/check-structure.mjs --json       # machine output: [{ id, title, target, actual, pass, findings }]
```

The gate exits 1 when any metric fails and prints each finding as `path[:line]: message`. New files count once
tracked (`git add`).

<details>
<summary>Inputs and how metrics are written</summary>

The only inputs are `git ls-files` (tracked files present in the working tree) and file contents. No build,
network or clock: two runs print the same bytes.

Each metric is a module, `scripts/check/check-structure/metrics/<NN>-<slug>.mjs`, exporting
`{ id, title, target, check(repo) }`. `check` returns `{ actual, findings: [{ path, line?, message }] }`; no
findings means pass. The module's header comment is the authoritative predicate. Exemptions exist only as named
rules inside a module, each with its reason.

Shared code sits beside the metrics: `repo.mjs` (the repository view; `memoryRepo` builds one in memory for tests),
`lib.mjs`, `generated.mjs` (#6, #20, #25) and `fixtures.mjs` (#3, #10). Tests are in `test/structure/`;
`boundaries.test.mjs` covers the layer boundary lint #13 runs.

</details>

### Terms

- A **rule document** is a JSON file whose `format` is `rerail.policy/v1` (a code rule), or a JSON object with no
  `format` holding a conversation rule's required fields: `id`, `mode`, `match`, `sample` and `action`
  (`isRuleDoc` in `lib.mjs`). Metrics 2, 7, 11, 15, 16 and 23 count both.
- A **test file** is a `*.test.*`/`*.spec.*` script, a pytest `test_*.py`/`*_test.py` or a PHPUnit `*Test.php`.
- The **data roots** are `fixtures/**` and `packs/<id>/upstream/**`: recorded or copied material, not the
  repository's own code.

## Location

Metrics 1–7 say where each kind of file may live.

### 1. `pack.json` outside `packs/<id>/` — target 0

Every `pack.json` is `packs/<id>/pack.json`. Exemption `validator-inputs`: `fixtures/packs/**/pack.json` are
pack-validator test inputs.

### 2. Rule documents outside `packs/*/{rules,candidates}` — target 0

Every rule document, code or conversation rule, is under `packs/<id>/rules/` or `packs/<id>/candidates/`. A
benchmark or test that needs a pack's rule references the pack's document.

<details>
<summary>Exemptions</summary>

- **`engine-test-inputs`:** a rule document may be an engine or host test input under `fixtures/`
  (`fixtures/<family>/compile/…`, `fixtures/host/…`), never shipped (#23). No directory on its path below
  `fixtures/` may be named `rules`, `candidates`, `probes` or `policies`: those names mark rule sets, which belong
  in a pack.
- **`validator-inputs`:** `fixtures/packs/**`, whose pack format puts rule documents in `rules/`.

</details>

### 3. Fixture data outside `fixtures/` and `packs/*/upstream`, pack test data in git or unpinned, or pack data waiting to publish — target 0

Fixture data lives only under `fixtures/` or `packs/<id>/upstream/`. Pack test data lives in S3, not git, pinned by
each tool directory's `test-data.lock`. To add or change it, stage it under the tool-pack's tracked `publish/`:

```sh
npm run fixtures:stage -- --pack <id> --tool <tool>
```

**Publish pack data** uploads it after a code owner approves.

<details>
<summary>What counts as fixture data</summary>

Fixture data is:

- any file with a `fixtures` or `__fixtures__` directory in its path;
- `*.expected.json` (expected findings), `*.cases.json` (case lists) and code-rule documents;
- non-TS/JS, non-Markdown files in a test root (`test/`, `integration/`, `conformance/`, `benchmark/`);
- files in `test/` or `integration/` with a fixture name.

Exemptions:

- `packs/<id>/candidates/**` (pack data).
- **`gate-pins`:** the conformance gate's lists (`conformance/languages/ungated-rules.json`,
  `conformance/languages/pending.json`) and `scripts/pack-data/runtimes/**`, the manifests that pin each runtime
  the derivation runs upstream on.
- For code-rule documents, `packs/<id>/rules/**`.

</details>

<details>
<summary>Pack test data patterns and publishing</summary>

No tracked file matches `PACK_FIXTURE_PATTERNS` in `scripts/s3/pack-fixtures.mjs`, the pulled layout: under
`packs/*/upstream/*/`, the patterns `fixtures/**`, `tests/**`, `*.expected.json`, `cases.json`, `*.cases.json`,
`fixtures.upstream-check.json`, `lizard-*.tsv`, `authored/**`, `provenance.json`, `LICENSES.md`
(CONTRIBUTING.md, "Pack test data"). Each tool-pack's test data, derived and authored alike, is one archive, pinned
by sha256.

While any `publish/` is tracked, the metric fails once per tool-pack ("Pack data waiting to publish"), naming the
workflow to run. **Publish pack data** (`.github/workflows/pack-data-publish.yml`) then uploads the data, commits
the new locks and deletes `publish/` on the pull request's branch. The pull request cannot merge before its data is
in S3 and its locks name it.

</details>

<details>
<summary>Lock rules</summary>

Every tool directory has a tracked `test-data.lock` that `parseLock` in `scripts/s3/test-data-archive.mjs`
accepts. A tool directory is a `packs/<id>/upstream/<tool>/` with a tracked file outside `publish/`; files directly
under `packs/<id>/upstream/`, such as `common.py`, are in none. A tool-pack with a tracked `publish/` needs no
lock: the publish writes it.

- **Format:** `{ "format": 1, "sha256", "files", "bytes" }`; sha256 is 64-digit lowercase hex, counts are
  non-negative integers.
- **`samples.lock`** (optional) pins the tool-pack's real-code samples archive the same way; `parseLock` must
  accept it too.
- **Findings:** a missing lock is reported at the path it would have; a malformed one carries `parseLock`'s
  message.

These checks have no exemptions.

</details>

### 4. Engine source roots — target 1 (`native/`)

No `include_str!`/`include_bytes!`/`include!` in a `native/**/*.rs` file resolves outside `native/`. Nothing is
tracked under the retired `code-rule-dsl/`.

<details>
<summary>How includes are resolved and exempted</summary>

String-literal paths resolve against the including file and `concat!(env!("CARGO_MANIFEST_DIR"), "…")` against
the crate. Other arguments, such as `fixtures_dir!()` or `OUT_DIR`, are skipped.

Exemption `test-inputs`: a test-only include may read `fixtures/**`, `packs/**` or a `*.generated.*` file. An
include is test-only when it is:

- in `tests.rs`, `*_tests.rs`, `test_support.rs` or under `native/tests/`; or
- inside the item a `#[cfg(test)]` attribute applies to: its brace-matched body (comments and string and
  character literals skipped), or up to its `;` when it has none.

`#[cfg(test)] mod tests;` declares a module in another file and exempts nothing after it.

</details>

### 5. Test roots — target `test/`, `integration/`, `e2e/`

Every test file is under `test/`, `integration/` or `e2e/`. `e2e/` holds the dashboard's Playwright browser specs,
kept out of `test/` because vitest runs every spec file under it. Exemption: `data-roots` (upstream projects'
copied tests). Rust `#[cfg(test)]` modules are part of the crate and not counted.

### 6. Generators follow one known pattern — target 0 misplaced

Every generator is under `scripts/generate/`, never beside its output in `native/src/` or `src/`, whatever its
name: `scripts/generate/schema/schema-sql.ts`, not `src/engine/storage/schema-sql.ts`.

<details>
<summary>What counts as a generator</summary>

A generator is any of:

- every manifest row's generator (`scripts/generate/README.md`, #25);
- every path a generated file's `Generated by <path>; do not edit.` header names;
- any script named `generate`, `generate_*` or `generate-*` (extension `.mjs .cjs .js .ts .mts .py .php`).

Exemption `data-roots`: pack generators sit beside the expected findings they write, under `fixtures/` or
`packs/<id>/upstream/`; generated files there are not scanned for headers.

</details>

### 7. One location pattern for licences — target 100%

Each pack rule document's licence must match the pack's `package.json` `license`.

<details>
<summary>How licences are compared</summary>

The document's REUSE-resolved licence, as a set of `AND` terms, is compared with the pack's `license`. In
REUSE.toml the last matching `[[annotations]]` applies: `override` wins outright, otherwise an in-file
`SPDX-License-Identifier` or a `.license` sidecar wins; `aggregate` combines.

- `packs/<id>/rules/**`: equal (the package's licence describes exactly what it ships).
- `packs/<id>/candidates/**`: only terms the pack declares (Rerail's own probe rules are Apache-2.0 alone).

Copied pack test data is not in git, so this metric checks rule documents only.
`packs/<id>/licenses/<project>/UPSTREAM.md` records copied files by pattern; pack check 6 checks the pulled data
against those patterns.

</details>

## Contract conformance

Metrics 8–11 check packs and fixtures against their formats and the engine.

### 8. Packs pass the pack-format layout — target 100%

Every pack has the files and manifests the pack format requires. The compile half of the
[validation contract](pack-format.md#the-validation-contract) runs in the pack tests.

<details>
<summary>Exact checks</summary>

- **Files:** every `packs/<id>/` has `pack.json`, `package.json`, `README.md`, `LICENSE`, `NOTICE`.
- **`<id>`:** lowercase letters, digits and dashes.
- **`pack.json`:** `rerail.rule-pack/v1` with `id` = `<id>`; lists only tracked documents, and every file in
  `rules/`.
- **`name`:** no brackets, digits or tool from the pack's `conformance.json`. `README.md` opens with
  `# Rerail rule pack: <name>` and `candidates/README.md` with `# Rerail rule pack: <name>, candidates`. A published
  pack has a row for it in the `docs/rule-packs.md` table.
- **`package.json`:** `@neralabs/rerail-pack-<id>`, with a `license`, no `dependencies`, and `files` publishing
  `pack.json`, `rules`, `LICENSE`.

</details>

### 9. Pack directories that import the engine — target 0

No JS/TS file under `packs/` imports a path resolving into `src/`, or `@neralabs/rerail[/…]`.

### 10. `fixtures/<family>/<feature>` maps to an engine module — target 100%

Every `fixtures/<family>/<feature>/` names a top-level module of `native/src/code_rule_dsl/` (`<feature>.rs` or
`<feature>/`, not `mod.rs`/`test_support.rs`), or a documented group: `upstream` (tool tests and expected
findings ahead of a pack) or `licenses`. Exemption `non-engine-families`: the children of `fixtures/host/` and
`fixtures/packs/` are areas.

### 11. No orphan fixture families or packs — target 0

Every fixture family maps to a language or pack, or is a declared shared family. Every pack holds a rule document.

<details>
<summary>Exact rules</summary>

Every `fixtures/<family>/` has `languages/<family>.json` or `packs/<family>/`, or is a declared shared family:
`ecma` (ECMAScript oracles shared by the javascript and typescript languages and their packs), `host` or `packs`
(not tied to a language). Every pack holds at least one rule document in `rules/`, or in `candidates/rules/` while
it ships none because all its ports are candidates.

</details>

## Layering

Metrics 12–15 keep directories small, imports one-way and each concept in one place.

### 12. Max files in one flat code or test directory — target ≤25

A directory directly holding a `.ts .tsx .mts .cts .js .mjs .cjs .rs .py .php .sh` file directly holds at most
25 tracked files. Exemptions: `pack-data` (`packs/<id>/{rules,candidates,upstream,licenses}/**`, one file per rule
or test case), `fixture-data` (`fixtures/**`) and `vendored` (`native/grammars/**`).

### 13. Import-direction violations — target 0

This metric is the result of the layer boundary lint, `scripts/check/check-boundaries.mjs`; run it alone as
`npm run check:boundaries`. The metric fails when the lint is not tracked.

<details>
<summary>Lint rules</summary>

The lint's entry is `checkBoundaries(root, { files })`. Its rules are stated in its header and tested in
`test/structure/boundaries.test.mjs`:

- **`engine-isolation`:** `src/engine/` imports no other layer.
- **`host-isolation`:** no host imports another.
- **`layer-order`:** each layer imports only the layers listed for it, the order in `src/README.md`.
- **`src-self-contained`:** `src/` imports nothing outside `src/`.
- **`entry-points`:** code outside `src/` imports a layer's `index.ts` or `src/index.ts`. Unit tests under
  `test/<layer>/` may import their own layer's internals, and a vitest mock seam names the module it replaces.

</details>

### 14. Production reads `fixtures/`; one root resolver per language — target 0; ≤1 per language

No file under `src/` resolves a fixtures root. Per language (Rust, TypeScript/JavaScript, Python, PHP, shell), at
most one file does.

<details>
<summary>What counts as resolving a fixtures root</summary>

A file resolves a fixtures root when it writes any of:

- on a path-building line (`join(`, `resolve(`, `URL(`, `Path::new(`, `PathBuf::from(`, `||`, `??`, pathlib
  ` / `), a relative path literal whose first component is `fixtures`;
- the bare segment `"fixtures"` as a direct argument of a `join(`/`resolve(` call whose earlier arguments are one
  root (any balanced expression) and then only `"."`/`".."` segments. The call may span lines:
  `join(here, "..", "fixtures")`, `resolve(process.cwd(), "fixtures")`, `path.join(repoRoot(), "fixtures")`;
- the bare segment anywhere on a path-building line of a file that reads the `RERAIL_ORACLES_DIR` override.

Compared literals (`startsWith(`, `===`, …), name lists and deeper segments (`join(root, "labels", "fixtures")`)
do not count.

Exemptions: `data-roots` (generators write beside themselves) and `structure-gate` (this gate and its tests).

</details>

### 15. Duplicate concepts — target 0 unexplained

Rule ids, the `pack.json` and `conformance.json` formats, and pack-specific harness code each have one owner.

<details>
<summary>Exact checks</summary>

- `rule-id`: no rule `id` is defined by two rule documents across `packs/*/{rules,candidates}/**`. Exemption
  `option-variant`: documents under `packs/<p>/candidates/<rule>/` (any depth) whose id is `<rule>` or ends
  `:<rule>` may share it with each other and with `packs/<p>/rules/<rule>.json`.
- `pack-format`, `conformance-format`: all `pack.json` files share one `format`, and all `conformance.json` files
  share one (absent = `untagged`). Files differing from the most common are reported. Exemption
  `validator-inputs`: `fixtures/**` (deliberately malformed inputs included).
- `harness-import`: code in a pack-specific directory (`packs/<id>/`, `conformance/<id>/`,
  `test/conformance/<id>/` for a pack `<id>`) is imported only by code for the same pack.

</details>

## Naming

Metrics 16–18 check what directories hold and how files are named.

### 16. `rules/` directories holding non-rules — target 0

Every file below a `rules/` directory is a rule document: a code rule or a conversation rule, which a pack may ship
in `rules/`. A JSON object sharing only some of a conversation rule's fields is not one. Exemptions: `data-roots`
(upstream trees) and `vendored`.

### 17. Test helpers in a support directory — target 100%

Every non-test script under `test/` or `integration/` has a `support/` directory in its path. Exemptions:
`ambient-declarations` (`*.d.ts`) and `fixture-trees` (below a `fixtures/` directory: data, #3).

### 18. One case convention per language — target 100%

TS/JS names are kebab-case; Rust and Python names are snake_case.

<details>
<summary>Exact scope and exemptions</summary>

| Language | Case | Applies to |
|---|---|---|
| TS/JS | kebab-case | File basenames (first dot part; later parts lowercase) and their directories; dot-directories skipped |
| Rust | snake_case | File names and module directories below `src/`/`tests/` |
| Python | snake_case | File names |

The `.generated` infix (#20) is not part of a stem: `builtins.generated.rs` is checked as `builtins`.
Exemptions: `data-roots` (named after upstream tools and engine modules), `vendored`, `fixture-trees` (framework
route files such as `pages/_app.tsx`).

</details>

## Hygiene

Metrics 19–21 and 25 cover large binaries, generated files and READMEs.

### 19. Large binaries committed without LFS — target 0 (>100 KB)

Every tracked binary over 100 KB matches a `.gitattributes` line setting `filter=lfs`.

<details>
<summary>How binaries and patterns are matched</summary>

A binary has a NUL byte in the first 8000 bytes, or a known binary extension. The last matching `.gitattributes`
line wins; `-filter`, `!filter` or another `filter=` unsets. Patterns follow gitattributes: a pattern without `/`
matches the basename anywhere; otherwise it is anchored to the declaring `.gitattributes`. A checkout without LFS
content holds text pointers, which pass.

</details>

### 20. Generated files are marked and grouped — target 100%

Each generated file is named `*.generated.*` or listed in `scripts/generate/README.md`. When commentable, it
carries `Generated by <generator path>; do not edit.` within its first three lines, naming a tracked file.

<details>
<summary>Detection and exemptions</summary>

A file is generated when it is named `*.generated.*`, or its first five lines say "generated by", carry the
at-sign generated tag, or say "do not edit". Commentable means not JSON and not binary.

Exemptions: `data-roots` (pack test data follows the fixtures convention) and `tool-owned` (`*.lock`,
`package-lock.json`, `.changeset/README.md`). The manifest and this gate are not scanned.

</details>

### 21. Top-level directories have a README — target 100%

Every top-level directory has `README.md`. Exemptions: `dot-directories` (tool configuration) and
`reuse-licenses` (`LICENSES/` holds only licence texts).

### 25. Generated-file manifest is consistent — target 0

`scripts/generate/README.md` is a markdown table: first column the generated path, second the generator, both in
backticks. A path ending `/` is a directory of generated files.

<details>
<summary>Consistency checks</summary>

- The manifest exists whenever a `*.generated.*` file does.
- Every row's generated path and generator are tracked, the generator under `scripts/generate/` (#6).
- Every `*.generated.*` file is covered by a row.
- A header naming a generator names the row's.

</details>

## Found during the review

Metrics 22–24 cover problems the structure review found.

### 22. Stale paths left from the import — target 0

No tracked text file mentions `rerail-fixtures/`, `code-rule-dsl/rules/opt-in` or
`policies/rule-packs`. Exemptions: `history` (`CHANGELOG.md`, `.changelog/*.md`, `.changeset/*.md`) and
`structure-gate` (this page, the gate, its tests).

### 23. Shipped rules come only from `packs/*` — target `packs/*` only

Each root `package.json` `files` entry that ships rules starts with `packs/<id>/`. An entry ships rules when it
covers a rule document or a `pack.json`, or names a `rules` directory, empty or not.

### 24. Every test root and harness runs in CI — target 100%

Every test file outside the data roots runs in CI. So does every entry point that an npm script named `test*` or
`conformance*` runs. Rust tests run through `cargo test`, outside this metric.

<details>
<summary>How the gate finds what CI runs</summary>

The gate expands every `run:` script in `.github/workflows/ci.yml` through npm scripts, transitively
(`npm run`, `npm test`, `npm --prefix <dir>`, `cd`). It collects what runs:

- `vitest`, by `--dir`, `--config` directory and filters, minus the package's `vitest.config.*` `exclude` globs;
- `node --test` and `pytest`;
- `playwright test`, the spec files under its config's `testDir`;
- entry points: `node`/`tsx`/`python`/`bash` files.

</details>

## On `main` (`fbb6104f`)

The gate's first run, before the structure work, shows where the layout started: 5 of 25 metrics passed. It used
the predicates of that time; #4, #6, #13 and #14 have since been tightened.

<details>
<summary>Results of the first run</summary>

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

</details>

## Adding or changing a rule

Change a metric's module (predicate and header comment), its test in `test/structure/` and its section here
together.

- A new exemption is a named rule in the module with its reason, never a path list maintained elsewhere.
- A new metric is a new `scripts/check/check-structure/metrics/<NN>-<slug>.mjs`; the gate loads every module in
  that directory.
