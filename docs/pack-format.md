# Rule pack structure and validation contract

A rule pack is a directory under `packs/`: rule documents plus the evidence that they behave.
`test/engine/catalog/packs.test.ts` validates each one against the [validation contract](#the-validation-contract), and
the [release workflow](#release) publishes each on its own, except a [private](#packagejson) one, which is never
published and is installed from a checkout with `rerail packs add packs/<id>`.

Core ships no packs and has no rules of its own. Set-up and `rerail packs add` install the published packs from npm
into `$RERAIL_HOME/packs`, and core reads them from there. To start a pack, see
[language packs](language-packs.md#start-a-pack).

<details>
<summary>How core lists packs</summary>

Core lists published packs by name only, in `src/engine/catalog/pack-list.generated.ts`
([rule packs](rule-packs.md#installing-packs)). Like every By Rerail rule, a pack's rules run only in repositories that
add them or the pack.

A pack adds no file format: it combines the two schemas the repository already has. Code rules compile against the
engine's `capabilities.json`, `native/src/code_rule_dsl/capabilities.json`.

</details>

## Principles

Four principles decide what a pack holds and how it is checked.

1. **A pack is data.** It holds no code and never imports the engine. Core runs its rules; the repository's tests
   validate it.
2. **The directory is the contract.** The files a pack has decide what it must prove.
3. **Every rule is proven,** by examples it must catch and let pass, or, for a port, by the tool's own tests.
4. **Packs version on their own.** Each is its own npm package, released separately from core.

<details>
<summary>More on each principle</summary>

- A pack holds no code even to decide whether it fits a repository: its `match` is globs and dependency names.
- There is no separate validation manifest.
- A port of another tool's rule is proven by that tool's expected findings on its own tests, with every disagreement
  declared.

</details>

## Layout

A pack's files, and which of them ship to npm.

```text
packs/<id>/
├── package.json        npm package @neralabs/rerail-pack-<id>: licence, files, core range
├── pack.json           the pack: rerail.rule-pack/v1
├── README.md           what the pack covers, for people: the rules it ships, never a candidate's details
├── LICENSE, NOTICE     the pack's licence and notice
├── rules/
│   └── <rule id>.json  rule documents, each named for its id: code rules (rerail.policy/v1) or conversation rules
├── licenses/
│   └── <project>/      upstream LICENSE (verbatim) and UPSTREAM.md (per repository: source, path patterns taken, not copied)
│
│   only for packs that port rules from another tool:
├── conformance.json    the conformance pack schema of conformance/languages/pack.ts
├── divergences.json    declared disagreements with the tools
├── upstream/
│   └── <tool>/         gates.yaml, the generator, test-data.lock, samples.lock and SAMPLES.md (optional), and
│                       UPSTREAM.md when Rerail wrote files for the tool to score; the tool's copied tests
│                       (fixtures/, rename variants under fixtures/renamed/) and expected findings (*.expected.json)
│                       are pack test data, not in git (see Pack data); publish/ while a pull request publishes data
└── candidates/         ports still being evaluated: scored and reported, never gated, never shipped; README.md
                        says what each is waiting on and holds their rule details
```

`<id>` is lowercase letters, digits and dashes. It is the directory name, the `id` in `pack.json` and the npm name
`@neralabs/rerail-pack-<id>`. That scope is Rerail's. A pack published elsewhere is named `rerail-pack-<id>` or
`@<scope>/rerail-pack-<id>`, and core loads it if it passes the catalog's checks
([third parties' packs](rule-packs.md#third-parties-packs)).

### package.json

```json
{
  "name": "@neralabs/rerail-pack-typescript",
  "version": "0.1.0",
  "license": "Apache-2.0 AND MIT",
  "repository": { "type": "git", "url": "git+https://github.com/neralabs/rerail.git", "directory": "packs/typescript" },
  "files": ["pack.json", "rules", "licenses", "LICENSE", "NOTICE", "README.md"],
  "peerDependencies": { "@neralabs/rerail": ">=0.1.0-0 <0.2.0" },
  "peerDependenciesMeta": { "@neralabs/rerail": { "optional": true } },
  "publishConfig": { "access": "public" }
}
```

- **`peerDependencies`**: the core range. A pack has no `dependencies`.
- **`files`**: the pack and its licence records. Validation material never reaches npm.
- **`repository`**: this repository and the pack's directory. `publishConfig.access` is `public`, like core.
- **`private`**: `true` makes a pack that is never published. It needs no `publishConfig`; `false` is refused (a
  published pack omits `private`).

<details>
<summary>Why these fields</summary>

- The peer range is a declaration, not an import. The `-0` admits core's prereleases (`0.1.0-next.0`).
- Rerail's catalog enforces the range: an installed pack whose range excludes the active core is left out
  ([rule packs](rule-packs.md#installing-packs)). npm doesn't, since core is never installed beside the packs in
  `$RERAIL_HOME/packs`.
- The peer is `optional` because `packs/*` are workspaces of this repository, whose root package is core. Otherwise
  npm would try to install a published core into them.
- `conformance.json`, `divergences.json`, `upstream/` and `candidates/` are validation material: they stay in the
  repository, which keeps copied upstream tests out of the package.
- npm trusted publishing requires the repository it publishes from, hence `repository`.
- A private pack: npm refuses to publish it, Changesets never versions it, the release scripts leave it out, and the
  pack list does not name it, so set-up never fetches it. It still passes every check, and is installed from a
  checkout with `rerail packs add packs/<id>`.
  The conformance gate's not-gated list leaves it out ([conformance/README.md](../conformance/README.md#packs-not-gated)):
  its rules are proven by their examples and tests, not by an upstream tool.
- The schema is `rulePackPackageSchema` in `src/engine/catalog/rule-packs.ts`.

</details>

### pack.json

`pack.json` is the pack, in the `rerail.rule-pack/v1` format (`rulePackFileSchema` in
`src/engine/catalog/rule-packs.ts`). The TypeScript pack, with one of its rules:

```json
{
  "format": "rerail.rule-pack/v1",
  "id": "typescript",
  "name": "TypeScript",
  "category": "language",
  "covers": "typescript-eslint and @vitest/eslint-plugin rules for safe, idiomatic TypeScript and Vitest tests.",
  "reason": "matches TypeScript",
  "match": {
    "any": [
      { "dependencies": { "ecosystem": "npm", "names": ["typescript"] } },
      { "files": ["**/*.ts", "**/*.mts", "**/*.cts"], "exclude": ["**/*.d.ts"], "label": "*.ts files" },
      { "files": ["**/*.tsx"], "label": "*.tsx files" }
    ]
  },
  "rules": [
    { "id": "typescript-eslint-ban-tslint-comment", "document": "rules/typescript-eslint-ban-tslint-comment.json", "title": "typescript-eslint ban-tslint-comment: Disallow `// tslint:<rule-flag>` comments", "group": "typescript-eslint", "on": false }
  ]
}
```

- **`name`**: the title on the dashboard, in set-up and in a steer's note.
- **`covers`**: the dashboard's one-sentence line under the pack's name.
- **`category`**: `language`, `framework` or `repository`.
- **`match`** (required): `{ "any": [signal, …] }`, what set-up recommends the pack for. A signal is file globs or
  package dependencies.
- **`reason`**: the pack's own summary.
- **`honourSuppressionComments`** (optional, default `true`): whether its rules honour their linters' suppression
  comments.
- **`rules`**: the pack's rules. Each entry has an `id` ([Rule ids](#rule-ids)), a `document` (required,
  `rules/<id>.json`), a `title` and `group`, and optional `mode`, `on` and `examples`.

<details>
<summary>Field details</summary>

- **`name`** says what the pack is about, and nothing else: `Python`, not `Python (Ruff and Bandit)`. Tools, versions
  and limits go in `covers`, since they change and a title should not. The repository structure gate
  ([metric 8](repository-structure.md#8-packs-pass-the-pack-format-layout--target-100)) holds Rerail's own packs to
  this. A pack's README opens with `# Rerail rule pack: <name>`, and a published pack has a row in
  [the packs table](rule-packs.md#the-packs).
- **`covers`** says what the rules are for and where they come from. It gives no rule counts and nothing on which
  start on; the dashboard shows both beside it.
- **`category`** drives the kind chips of the dashboard's rule pack catalog, which lists languages, then frameworks,
  then repository packs.
- **`match` signals:**
  - `{ "files", "exclude"?, "label" }`: repository-relative globs. `**` crosses folders, `*` and `?` stay within one,
    `{a,b}` gives alternatives.
  - `{ "dependencies": { "ecosystem", "names" } }`: any manifest in the repository depending on one of the packages.
    - `npm` reads `package.json` (`dependencies`, `devDependencies`, `peerDependencies`, `optionalDependencies`).
    - `pypi` reads `requirements*.txt` and `pyproject.toml`: PEP 621 `dependencies` and `optional-dependencies`,
      Poetry's `dependencies` less `python`, its `group.<name>.dependencies`, and the `dev-dependencies` of Poetry
      before 1.2. Names are normalised as PEP 503 does.
    - `composer` reads `composer.json` (`require`, `require-dev`), case-insensitively.
- **`reason`** shown for a repository is built from what matched, naming at most two signals:
  `matches the typescript dependency in package.json and *.ts files in src`. A file signal names the folder its files
  share, so a person can spot a pack picked only for a stray folder.
- **`honourSuppressionComments`** is the pack's start for the `suppressions` rule setting. With `false`, no linter's
  comment silences its rules until a repository or checkout sets `suppressions: "honour"`. Rerail's own
  `rerail-ignore` always does.
- **`rules`:** each rule belongs to exactly one pack. `"rules": []` is allowed while all ports are candidates. Set-up
  recommends only a pack with at least one rule that starts on ([rule packs](rule-packs.md#the-packs)).
  - `document` is relative to the pack and carries the rule's id. Core reads each document from the pack, so a pack
    is self-contained.
  - `title` and `group` are what the dashboard shows.
  - `mode` is the rule mode the pack starts the rule in; otherwise the document's. Ports use it for upstream's
    warning level (below).
  - `on` (default `true`) is whether the pack starts the rule on.
  - `examples` is `{ "caught": { "path"?, "text" }, "passes": { … } }`. A code rule's examples need a `path` inside
    the rule's scope, which decides the file's language; a conversation rule's have none. The complexity pack has one
    pair per rule.

**Conversation rules.** A pack may also ship a conversation rule: the rule object of
[rule-schema.ts](../src/engine/catalog/rule-schema.ts) (`id`, `mode`, `match`, `sample`, `action`, optional `repeat`)
as its document, with no `format`. It steers on the coding agent's session rather than on code. Its examples take no
`path`: `text` is the command the coding agent runs for a rule that samples tool calls (`tool:*` is tried as
`tool:Bash`, the command as the call's `input.command`), and the sampled text otherwise. Its schema has no
`description`, so check 8 leaves it out; say what it does in the pack's README.

Internal testing: conversation rules in a pack, as every conversation rule; Rerail 0.1.0 releases code rules only. See
[Internal testing](../README.md#internal-testing).

</details>

A ported rule starts at the level upstream's recommended configuration gives it: upstream `warn` becomes
`"mode": "log"` in `pack.json`.

<details>
<summary>Upstream levels and modes</summary>

The document stays a faithful port with no level.

- **Upstream `warn`** (an ESLint-family `"warn"`, a Clippy lint whose default level is `warn`, a PHP_CodeSniffer
  warning): `"mode": "log"` in `pack.json`. It is recorded and steers no one until a repository switches it to `steer`.
- **Upstream `error` or `deny`:** keeps its document's `steer`.
- **No warning level** (Ruff, Bandit, SQLFluff, PHPStan, Psalm, tlint, blade-parser): keeps the document's mode.

`test/engine/catalog/rule-packs.test.ts` holds what match and examples promise: every rule starts in the mode its pack
or document gives it, and every example is caught or passes as it says.

</details>

#### Rule ids

An id must not change once its pack is published, because people turn rules on and off by id.

- **Characters:** lowercase letters, digits and dashes (`RULE_PACK_RULE_ID` in `src/engine/catalog/rule-packs.ts`).
- **Port:** the upstream tool's name, then its rule (`eslint-no-empty`, `typescript-eslint-no-explicit-any`,
  `ruff-b011-assert-false`, `n-no-sync`).
- **Rerail's own rule:** the pack's name, then the rule (`complexity-cyclomatic-python`).

<details>
<summary>Where the id must match</summary>

The id is the same string in `pack.json`, in the document's `id`, in its file name and in its port's `id` in
`conformance.json`. The file is `rules/<id>.json`, or `candidates/rules/<id>.json` while a candidate, so the id
survives shipping. Check 1 enforces this; the pack tests reject an id two documents in any packs share.

</details>

### conformance.json, divergences.json, upstream/

Packs that port rules keep their proof beside the rules, in the schemas of `conformance/languages/pack.ts`.

- **`conformance.json`**: the ports and what they are compared with.
- **`divergences.json`**: each place Rerail and the tool disagree, with its kind and evidence.
- **`upstream/<tool>/`**: the tool's expected findings, generator, `gates.yaml` and the locks of its
  [pack data](#pack-data).

<details>
<summary>Divergences, locks and oracles</summary>

- The runner reads expected findings from `upstream/` unless `conformance.json` sets `oracles`.
- A divergence's kind is `upstream-wrong`, `not-comparable` or `rerail-unknown`, with the evidence checked
  ([conformance/README.md](../conformance/README.md#divergences)).
- `upstream/<tool>/test-data.lock`, `samples.lock` and `SAMPLES.md` are the pins of the tool-pack's
  [pack data](#pack-data).

</details>

#### conformance.json

It records the language families, the engine capabilities, the pinned tools and, per port, the upstream tool and rule.

<details>
<summary>Full contents and per-port options</summary>

It records:

- the language family, or the families for a pack whose ports span languages (complexity's
  `["python", "ecma", "php", "blade", "rust", "vue"]`);
- the engine capabilities the pack declares, with exemptions and their evidence;
- the pinned tools;
- per ported rule: the upstream tool and rule it reproduces, how findings compare (`line` or `line+column`), and the
  capabilities it exercises;
- `suppressionExemptions`: ports, by `id`, whose documents need not carry their tool's suppression comment yet, each
  with a `reason` (check 4);
- `own`: the ids of the pack's own rules, written for Rerail with no upstream tool. The conformance gate judges each
  once, on `loads` and `compact` alone, the gates that read the document by itself; `upstream-status` and
  `conformance` need an upstream tool to compare with. Lacking a tool-pack's file types, `loads`'s in-scope check asks
  the rule to apply to its `pack.json` examples' paths. The tool-pack run does not report them as unmapped and the
  not-gated check does not ask for them on the frozen list. Each must ship in `pack.json` with examples, which are
  their proof, must set no `violation.suppress` (check 2), and must have no port entry.

A port whose document is under `candidates/` is scored on the fixtures, whose expected findings must cover it, and
reported; its scores never fail a check. A port may also set:

- **`examples: { caught, passes }`**, each with `path` and `text` as in `pack.json`: the rule's pair while `pack.json`
  does not list it. Check 3 runs it; it moves to the `pack.json` entry when the rule ships. A shipped port with
  `examples` fails the schema.
- **`fixtures`**: `path.matchesGlob` patterns over the tool's fixture paths. The port is evaluated and scored only on
  matching fixtures, the tests its upstream configuration ran on. So a tool whose tests run each rule with several
  option sets is scored with one document per option set, each on its own tests.
- **`lints`**: the file types the port covers, when fewer than its tool-pack's (`gates.yaml`'s `upstream.lints`).
  `loads`'s in-scope check then asks for those alone, as for complexity's one port per language.

</details>

#### Expected findings

`upstream/<tool>/fixtures.expected.json` is what the tool reported on its own tests. The generator beside it writes it,
run through `npm run fixtures:derive` ([docs/upstream.md](upstream.md#take-a-new-snapshot)).

<details>
<summary>Generators and the source block</summary>

- Recipes are in `scripts/pack-data/recipes/`, runtimes in `scripts/pack-data/runtimes/`; tools live under
  `.rerail/`, gitignored.
- Where a tool's tests state their expected findings as markup, the generator reads them instead of running the tool,
  since the tool's own test suite requires exactly those.
- The ESLint-based generators share `scripts/pack-data/eslint-expected.mjs`.

Its `source` block names `origin` (upstream repository `@` commit), the `rules` the run reported, the `files` (units: a
path, or a list of paths for a multi-file unit), and `units`. A `units` entry, keyed by the unit's path (or first
path), gives its upstream origin: `rules`, the upstream rules whose tests it was written from. A unit without an entry
(Rerail's probes, or of unknown origin) is scored but is no rule's own evidence for `stays-silent`.

</details>

<details>
<summary>ECMA language options</summary>

An ECMA unit's entry may carry `languageOptions`, where its test case set a language beyond ESLint's flat-config
defaults for its extension (`unitLanguage` in `scripts/pack-data/eslint-expected.mjs`):

- `ecmaVersion`, left out when `"latest"`;
- `sourceType`, left out when it is the extension's (`commonjs` for `.cjs` and `.cts`, `module` otherwise);
- `parserOptions`, without `ecmaFeatures.jsx`, which the extension gives;
- never `globals`.

A rename variant carries its original's. The conformance runner merges them into each AST clause's `languageOptions`,
so the port judges the file as the linter did. It merges `ecmaVersion`, `sourceType`, `parserOptions.ecmaFeatures`'
`globalReturn` and `impliedStrict`, and typescript-eslint's `jsxPragma`, `jsxFragmentName` and `lib`, which a code
rule spells at the top of `languageOptions`. A query that had none keeps reading its listed `global` names as unbound
(`declared: false`). A unit without them gets the port's own `languageOptions` (ESLint's defaults unless set).

The javascript pack's two tools and eslint-plugin-vitest record them. The other ECMA generators lint every case under
one configuration of ESLint's defaults (vue's, next's, react's, typescript-eslint's) or convey the source type through
the unit's `package.json` (eslint-plugin-n's), so their units carry none.

</details>

#### Rename variants

For the ECMA tool-packs, the generator also lints a renamed copy of every upstream unit, under
`fixtures/renamed/<the original's path>`. A port that matches the tests' names rather than the construct then fails.

<details>
<summary>How renaming works</summary>

- **Renaming** (`scripts/pack-data/rename/ecmascript.ts`): same length and shape, one order-preserving bijective map
  per file.
- **Kept:** literals, comments other than `/* global */`-style name lists, keywords, the standard library, common host,
  framework and test-runner API names, hook-shaped and `…Sync` names, and what a package exports.
- **Record:** the `source.units` entry adds `renamedFrom`, the key of an original unit with an origin.

The conformance check scores variants apart from the originals (`holds-under-rename`). A family without variants takes
its overfit evidence from its samples alone.

</details>

#### gates.yaml

`upstream/<tool>/gates.yaml` is a gated tool-pack's upstream manifest: pins, linted file types, and each upstream rule's
deprecation and recommended configuration ([conformance/README.md](../conformance/README.md#declared-upstream-facts-gatesyaml)).
Its `watch:` block says what the nightly upstream report ([docs/upstream.md](upstream.md)) compares between the pinned
commit and upstream's latest release:

  ```yaml
  watch:
    rule-prefix: "unicorn/"          # optional: removed from upstream.rule to give {rule}
    extra-rules: [NoArgumentsValidator]  # optional: rules the generator reads that conformance.json does not port
    tests: ["test/{rule}.js"]        # the tests the generator reads; or { file: fixtures.json [, with: [".stderr"]] } or { named-by: fixtures.json }
    code: ["rules/{rule}.js"]        # each ported rule's code; or { index: clippy-declared-lints | ruff-codes }
    shared: ["rules/utils/**"]       # code every ported rule depends on; no {rule}
    new-tests: ["tests/{rule}/**"]   # optional: where upstream adds tests for a ported rule that the generator does not read
    rules:                           # optional: a rule's own tests or code, in place of the {rule} patterns
      DebugDirectiveValidator: { tests: ["tests/Validation/DebugDirectiveTest.php", "src/Compiler/**"] }
  ```

<details>
<summary>Paths, special forms and coverage</summary>

The conformance check reads the manifest. Paths are relative to upstream's repository root; `*` matches within a path
segment and `**` across segments. `{rule}` is each ported rule: conformance.json's `upstream.rule` for this tool
without `rule-prefix`, plus `extra-rules`.

- **`{ file: fixtures.json }`** watches the paths a data file beside the generator lists per rule
  (`{ "rules": { "<rule>": ["<upstream path>", …] } }`), for a generator that lists its tests rather than finding them
  by convention.
- **`with: [".stderr"]`** also watches each listed path with its extension swapped for each one given, so a change to a
  Clippy test's expected output (`tests/ui/x.rs` → `tests/ui/x.stderr`) is a test change.
- **`{ named-by: fixtures.json }`** treats the data file's paths as PHPStan-style tests. The report reads the tests at
  both commits and watches exactly the files either names as `__DIR__ . '<relative path>'` (its
  `$this->analyse([…])` files), for the test's rules. The tests themselves are watched only where another entry lists
  them.
- **`{ index: … }`** names the upstream file the report reads to find each rule's code
  (`clippy_lints/src/declared_lints.rs`, `crates/ruff_linter/src/codes.rs`).

`expandWatch` in `conformance/languages/watch.ts` resolves the block. A `new-tests` glob may overlap the generator's
tests; a file counts as a new test only when `tests` does not cover it.

`test/scripts/upstream/watch-coverage.test.ts` checks that `tests` covers every upstream file the tool-pack's pulled
`provenance.json` records as read. Which files a `named-by` test names is upstream's text, which it does not read, so it
counts any file under that test's top directory (`tests/`) as covered.

</details>

## Pack data

A tool-pack's test data and real-code samples live in S3 archives, never in git or npm.

- **Test data:** the tool's copied tests, its expected findings, `authored/` files and the archive's provenance files.
- **Samples:** the tool-pack's labelled real files ([docs/upstream.md, "Samples"](upstream.md#samples)).
- **Locks:** each kind is one archive, named by a tracked lock (`test-data.lock`, `samples.lock`) in the tool
  directory.

Fetch the archives your checkout's locks name, and pull again after a checkout or merge that changes a lock:

```sh
npm run fixtures:pull
```

[docs/upstream.md](upstream.md) covers how the data is made, published and judged, and who does what
([Publishing pack data](upstream.md#publishing-pack-data)).

<details>
<summary>What test data holds</summary>

Test data is the files under `packs/<id>/upstream/<tool>/` matched by `PACK_FIXTURE_PATTERNS`
(`scripts/s3/pack-fixtures.mjs`), which `.gitignore` also covers: the tool's test cases (`fixtures/`, `tests/`, case
lists such as `cases.json` and `*.cases.json`, `fixtures.upstream-check.json`, `lizard-*.tsv`), its expected findings
(`*.expected.json`), `authored/`, and the archive's `provenance.json` and `LICENSES.md`
([below](#provenancejson-and-licensesmd)). It is of two kinds:

- **Authored:** what Rerail wrote for the tool to score, which nothing can regenerate. That is the files listed in the
  tool's own `upstream/<tool>/UPSTREAM.md` ([UPSTREAM.md](#upstreammd)), and `upstream/<tool>/authored/`: a
  contributor's code corpus with ground truth, where no public repository a pin can name holds it.
- **Derived:** everything else, which the generators make from the pins: upstream's tests, copied or derived, and the
  expected findings. Their `source.units` record each unit's origin, `renamedFrom` and `languageOptions`
  ([above](#conformancejson-divergencesjson-upstream)).

</details>

<details>
<summary>Samples archive layout</summary>

A tool-pack without a `samples.lock` has no samples. In the archive (`scripts/samples/format.ts`), each sample is
`<sample-id>/<source path>` beside `<source path>.expected.json`: the source repository, commit, path, sha256, SPDX
licence, and per rule the labelled lines. Context files go under the same `<sample-id>/`; `LICENSES.md` is at the root.

The tracked `SAMPLES.md` beside `samples.lock` lists the repositories with licences, commits and file counts, which
licence texts `LICENSES.md` holds, and how the set was trimmed, if it was.

</details>

<details>
<summary>Archive format, S3 locations and locks</summary>

An archive is the canonical tar of its files, zstd-compressed (`scripts/s3/test-data-archive.mjs`): one regular-file
entry per file, sorted by path's UTF-8 bytes, path relative to the tool directory, mode 0644, owner 0, time 0, no
directory entries. Archives live in us-east-2:

- test data: `s3://nera-test-fixtures/rerail/archives/<pack>/<tool>/<sha256>.tar.zst`;
- samples: `s3://nera-test-fixtures/rerail/archives/samples/<pack>/<tool>/<sha256>.tar.zst`.

Writes are create-only and the bucket is versioned, so changed data is a new archive and a new lock. `test-data.lock`
and `samples.lock` share one format, read by `parseLock`: `{ "format": 1, "sha256", "files", "bytes" }`. These are the
tar's sha256 (not the compressed file's, so a different zstd build changes nothing), file count and bytes. Every tool
directory with a tracked file outside `publish/` has a `test-data.lock`; `samples.lock` is optional.

</details>

<details>
<summary>How pulling works</summary>

`npm run fixtures:pull` fetches from S3, with the AWS CLI and read access to the bucket, only archives it lacks,
keeping them in `.rerail/pack-data/archives/`.

- `-- --pack <id> [--tool <tool>]` narrows it; `-- --dry-run` shows what it would fetch.
- It verifies every archive against its lock before extracting any. It refuses an archive with a path that has a
  control character, is not NFC, or equals another's ignoring case.
- Test data goes into the tool directory, with the tool-pack's `publish/` laid over it. Derived files on disk that
  neither holds (stale) are deleted, authored ones kept, and it says so.
- Samples go under `.rerail/pack-data/samples/<pack>/<tool>/<sha256>/`.
- It never writes or reads through a symbolic link: if `packs/`, a tool directory, its `publish/` or a directory a file
  goes into is one, it stops before writing anything.

Every consumer (checks 4 and 6, the conformance runner, the generators, the Rust tests) says to pull when data is
absent.

</details>

**In git** under `upstream/<tool>/`: `gates.yaml`, the generators, `UPSTREAM.md`, `test-data.lock`, `samples.lock`,
`SAMPLES.md`, and `publish/` while a pull request publishes data
([docs/upstream.md](upstream.md#publishing-pack-data)). Structure metric #3 fails while pack test data is tracked in
the pulled layout, a tool directory lacks its `test-data.lock`, a lock does not parse, or a `publish/` is tracked.

## Rule text

A code-rule document has text for two readers: people, and the coding agent.

- **`description`** is for people browsing the dashboard and rule catalog. The agent never reads it.
- **`violation.message`**, one per clause, is for the agent, which reads it inside the steer block Rerail writes:

```text
Rerail found 2 problems in your last edit:
- src/grade.js:1:8 — <message> [complexity-cyclomatic-javascript]
  Suggested fix: delete 14:3–14:12
- src/api.py:42:5 — <message> [bandit-b113-request-without-timeout]
Fix these, or tell the user in one line why you won't and run `rerail feedback <rule-id> '<that line>'`.
```

The block already gives the location, rule id, lead and latitude ("fix or explain"), so the message only advises what
to do about this code. Check 8 enforces the machine-checkable description criteria; review covers the rest and all
message criteria. `just rerail-guidance-review` shows every rule's description, messages and block for each case.

### Description

One plain-English imperative sentence of at most 200 characters, saying what to do or avoid, with no code and no
origin.

Example: `Avoid dd() and dump() calls; they leak debug output to users.`

<details>
<summary>All twelve criteria</summary>

1. Readable at a glance by a developer, in plain English.
2. States the intent: what the rule wants the developer to do or avoid.
3. No code or source material: no snippets, regexes, AST node kinds, globs, JSON, config keys, token lists or
   backticks. Naming the API or construct the rule is about is fine when the intent needs it ("dd() and dump() calls",
   "mutable default arguments", "useEffect").
4. No origin or source: no upstream tool or rule names, no versions, no "port of", no conformance or divergence notes.
5. Clear and concise.
6. Accurate: it matches what the clauses and scope catch. It doesn't overpromise (if tests are excluded and that
   matters, don't claim "everywhere") or underdescribe.
7. One voice for the whole repository: one imperative sentence starting with a verb (Avoid, Use, Prefer, Keep, Remove,
   Add, Declare, Close, Handle, Don't…) and ending with a period, optionally followed by `; <short reason>`.
8. At most 200 characters, a hard cap.
9. A reason after the semicolon only when it isn't obvious and still fits.
10. Siblings are told apart: when similar rules exist for several languages or file types, name the language or file
    type (`complexity-cyclomatic-python`, `-typescript`, `-javascript`).
11. Meaningful thresholds stay ("above 6", "more than 5 parameters"); implementation trivia goes.
12. It doesn't just repeat the rule's title or its message.

</details>

Detail left out goes under "Rule details" in the pack's README.

<details>
<summary>Rule details and candidates</summary>

Rule details hold exact counting rules, edge cases, deviations from upstream and scope detail. The README is
published, so it covers only rules `pack.json` lists. A candidate's details go under "Rule details" in the unpublished
`candidates/README.md`, and move to the README when the rule ships (check 5).

</details>

### Message

The message tells the agent what to change and, when the change could be wrong, when not to.

| Instead of | Write |
|---|---|
| `Bandit B113 (request-without-timeout)` | `Pass a finite timeout to this HTTP call; without one a slow server can hang the program.` |
| `zip() without an explicit strict= parameter` | ``Pass `strict=` explicitly: `strict=True` when lengths must match, `strict=False` when truncation is intended.`` |
| `` Comparison to `None` should be `cond is None` ``, one clause for `==` and `!=` | One clause each: ``Use `is None` instead of `== None` for this comparison.`` and ``Use `is not None` instead of `!= None` for this comparison.`` |

These criteria are for reviewers and never block a merge: no check reads a message's text.

<details>
<summary>All seven criteria</summary>

1. It names the change: a replacement or what to remove. A fact or a ban alone isn't advice.
2. If following it would be wrong for some code it matches, it says when to leave the code alone or what the change
   trades off.
3. It states a risk only as far as the rule can confirm it, without softening ("consider") or shouting (NEVER, MUST).
4. It is right for every match it doesn't exempt, agrees with the clause's `fix`, and names captures (`$NAME`) where
   that shows which code it means.
5. It leaves out the rule id, location, "Rerail" and "fix it or explain", and names no tool, rule code, tool version or
   matcher term outside quoted code.
6. Each text it can show is at most 200 characters (placeholders as written), in any number of sentences.
7. Matches that need different advice get different messages (`x == None` and `x != None`).

</details>

## The validation contract

Eight checks every pack must pass; ported rules must also pass the conformance gate to ship.

```sh
npx vitest run test/engine/catalog/packs.test.ts                         # every pack: checks 1 to 8, each rule's status
npx vitest run test/engine/catalog/packs.test.ts -t "pack python"        # one pack
```

To see every false positive and false negative of one ported rule:
`node --import tsx conformance/languages/cli.ts --pack <id> --rule <rule> --fixtures`.

| # | Check | Fails when |
|---|---|---|
| 1 | Rules | A rule document or candidate is unlisted, missing, misnamed or doesn't compile (a conversation rule: doesn't parse, or has a pattern the engine refuses); an id breaks the [id rule](#packjson) |
| 2 | Proof | A rule has neither `examples` nor a port entry in `conformance.json`; an id in `own` isn't a rule `pack.json` ships, or its document sets `violation.suppress` |
| 3 | Examples | An example isn't caught, or doesn't pass, as it says (a conversation rule: doesn't fire, or fires) |
| 4 | Ports | A shipped port disagrees with its expected findings outside `divergences.json`; suppression data is missing or stale |
| 5 | Package | `package.json` (`public` or `"private": true`), its `license` or a README's rule details are wrong |
| 6 | Licences | Provenance records are malformed or disagree with the pulled test data |
| 7 | Manifest | `pack.json` has no valid `match`; core's generated pack list lacks a published pack, names a private one, or is stale |
| 8 | Text | A code rule's description fails a machine-checkable [description](#description) criterion |

<details>
<summary>Where the checks run</summary>

`validatePack(directory, { listed?, fixtures? })` in `conformance/pack-validation.ts` runs checks 1 to 8 over one pack.
It returns each check's failures and each rule's status (`ok`, `failed` or `not-supported-yet`).

`test/engine/catalog/packs.test.ts` calls it for every directory under `packs/`, with one test per check and per rule,
plus the validator's own tests over the fixture packs in `fixtures/packs/`. It runs with this repository's core in
`npm test`, and in `release-packs.yml` on a pull request from this repository touching `packs/` and before each publish
([release](#release)).

The contract is a floor. A ported rule must also pass the
[conformance check's gate](../conformance/README.md#the-gate): on upstream's cases and their rename variants (no
unexplained false positive or miss), on real code, for size, and for silence where it should be silent. The gate
blocks CI for shipped rules and only reports candidates.

</details>

<details>
<summary>How check 4 is scored in CI and at publish</summary>

Check 4 scores ported rules on the tools' fixtures. Rename variants and real-code samples are scored by the
conformance check (`npm run test:conformance`), not here.

- **CI:** unit tests set `RERAIL_PACK_FIXTURES_IN_CONFORMANCE=1`, which passes `fixtures: false`, so check 4 skips the
  scoring. The conformance job scores the same fixtures with the same runner and fails on the same failures, so CI
  scores them once.
- **Publish:** the pack publish gate scores them in check 4, as `npm test` does.

</details>

<details>
<summary>Exact conditions: checks 1 to 3</summary>

1. **Rules.** `pack.json` must parse (`rulePackFileSchema`) and its `id` must be the directory name.
   - Every file in `rules/` is listed in `pack.json`, and every listed `document` exists. Each rule's `id` is lowercase
     letters, digits and dashes; its `document` is `rules/<id>.json` and carries the same `id`.
   - Each document is a valid code rule (`rerail.policy/v1`, the host's schema) and compiles in the engine with core's
     `native/src/code_rule_dsl/capabilities.json`, as the catalog compiles it; or a valid conversation rule (a JSON
     object with a `match` and no `format`, `parseConversationRule`) whose every pattern the engine accepts
     (`engineRegexIssues`).
   - Candidates need not compile. Every rule document under `candidates/` (each JSON file in a `rules/` directory
     there, and any file there whose `format` names a policy, an older `<name>.policy/v<n>` included) must be
     `rerail.policy/v1` or a conversation rule, sit in a `rules/` directory and be named `<id>.json`
     (`candidates/rules/<id>.json`, `candidates/probes/rules/<id>.json`).
   - Every port in the pack's `conformance.json`, and in each `conformance.json` under `candidates/` (probes), has its
     document's `id`. Check 4 loads only the root `conformance.json`, so check 1 also fails a `conformance.json` under
     `candidates/` that does not parse (`packSchema`) or has a port whose document does not exist.
2. **Proof.** A port entry is a `conformance.json` rule whose `policy` is the rule's `document`. Every id in
   `conformance.json`'s `own` must be a rule `pack.json` ships, whose document sets `violation.suppress` in no clause:
   every port carries `suppress`, for its upstream tool's comments, and an own rule has no upstream tool, so one that
   sets it is a port moved into `own`, out of the gates that compare it with its upstream.
3. **Examples.** A `caught` example must produce a violation of its rule and a `passes` example none, for `pack.json`'s
   examples and a candidate's in `conformance.json`. Each is evaluated alone through the engine, as a one-file
   repository at its `path` (`evaluateExample` in `conformance/pack-validation.ts`). The path must be in the rule's
   scope, and evaluation must produce no engine failure. A violation counts when its code is one of the rule's
   clauses' codes. A candidate's pair is held to the same against its document under `candidates/`. A conversation
   rule's examples are matched alone through the conversation engine (`ConversationRuleRegistry`, repeat ignored),
   built as [Conversation rules](#packjson) says: the caught one must fire the rule, the passes one must not.

</details>

<details>
<summary>Exact conditions: check 4, ports and suppression comments</summary>

4. **Ports** (with `conformance.json`). The fixture tier of the conformance runner (`conformance/languages/runner.ts`,
   the `language-conformance` test's), over the production workspace path.
   - An unexplained false positive fails on every source. An unexplained false negative fails on the fixtures only, as
     do engine failures, expected-findings errors and stale divergences (matching nothing) or exemptions.
   - A declared capability needs a ported rule or an exemption.
   - Candidates (ports `pack.json` does not list) are scored too; their false positives, false negatives and engine
     failures are reported and fail nothing.

   **Suppression comments.** Every port, candidates included, must honour its upstream tool's suppression comments as
   the tool does: each violation of its document carries the `suppress` key below, naming the port's `upstream.rule`.
   This lives in the document because `conformance.json` never ships. Ports of other tools (Bladestan, tlint,
   blade-parser, lizard) need none.

   | `upstream.tool` | The comment it honours | `violation.suppress` |
   |---|---|---|
   | `eslint`, `eslint-plugin-*`, `typescript-eslint` | `// eslint-disable-line <rule>` and the other `eslint-disable` forms | `eslint.rules` |
   | `ruff` | `# noqa: <code>` | `noqa.codes` |
   | `bandit` | `# nosec <id>` | `nosec.codes` |
   | `phpstan` | `@phpstan-ignore <identifier>` | `phpstan.identifiers` |
   | `psalm` | `@psalm-suppress <issue>` | `psalm.issues` |
   | `phpcs` | `phpcs:ignore <sniff>` | `phpcs.sniffs`: the sniff or one of its message codes, `<sniff>.<code>` |

   A port whose document does not model the comment is listed in `suppressionExemptions` as
   `{ "rule": <port id>, "reason": … }`: its expected findings ignored the comments (Bandit's `--ignore-nosec`), or it
   has a clause the engine cannot suppress (`suppress` is refused on a `regex` query). An exemption is stale when the
   port carries the suppression on every violation, or its tool honours no comment.

   **Oxlint.** A port of a rule Oxlint 1.88.0 also implements carries `oxlint.rules` too, with Oxlint's rule name
   without its plugin (`rules-of-hooks`, `no-console`), so `oxlint-disable` comments silence it as they silence
   Oxlint's. Oxlint also reads `eslint-disable` comments, so once any violation of a document carries `suppress.oxlint`,
   each violation carrying `suppress.eslint` must carry it, and none may carry it alone.

   **Biome.** A port of a rule Biome 2.5.15 has a rule the same as (its `sources` mark the ESLint rule `.same()`, not
   `.inspired()`: `useHookAtTopLevel` for `react-hooks/rules-of-hooks`) carries `biome.rules` too, with Biome's full
   name (`lint/correctness/useHookAtTopLevel`). Then `biome-ignore` comments naming it, its group or `lint` silence the
   port as they silence Biome's rule. Each violation carrying `suppress.eslint` must carry the Biome rules the same as
   its ESLint rules, and none may carry `suppress.biome` without `suppress.eslint`.
   `scripts/generate/packs/biome-suppression.mjs` reads the mapping, `conformance/biome-same-sources.generated.ts`,
   from Biome's source and writes `suppress.biome` into the pack documents.

</details>

<details>
<summary>Exact conditions: checks 5 to 8</summary>

5. **Package.** The npm name is `@neralabs/rerail-pack-<id>`, `publishConfig.access` is `public` unless `private` is
   `true`, `private` is never `false`, there are no `dependencies`, and `repository` is
   `{ "type": "git", "url": <the root package.json's repository url>, "directory": "packs/<id>" }`.
   - `files` publishes `pack.json`, `rules`, `LICENSE` and, when present, `licenses`. No entry may cover
     `conformance.json`, `divergences.json`, `upstream` or `candidates`.
   - Some way to satisfy `license` names exactly Apache-2.0 plus, per `licenses/<project>/UPSTREAM.md`, one way to
     satisfy the SPDX expression its `License:` line records. An `OR` there lets the package elect either side
     (`MIT OR Apache-2.0` with `Apache-2.0 AND MIT`); an `AND` must be named whole.
   - Under `## Rule details` in `README.md`, every `### ` heading names at least one rule in backticks, and every
     backticked name is a rule `pack.json` lists. Under `## Rule details` in `candidates/README.md`, none is. A heading
     inside a fenced code block doesn't count.
6. **Licences.** The pack's provenance records ([UPSTREAM.md](#upstreammd)) parse and agree with the pulled test data.
   - Each `licenses/<project>/UPSTREAM.md` has a well-formed `License:` expression that some way to satisfy uses only
     licences on the repository allowlist (`scripts/check/licence-allowlist.json`), a complete `Source` row, and only
     well-formed `Not copied` lines. The pack has a `LICENSE`.
   - Every pulled file in a subdirectory of `upstream/<tool>/` (the tool's copied tests: `fixtures/`, `tests/`) gets a
     licence by the rules in [Licence of each file](#licence-of-each-file), which the publish also uses. Files directly
     in `upstream/` or `upstream/<tool>/` (expected findings, generators) are Rerail's.
   - Each declared glob matches at least one pulled file. A checkout without the test data fails once per tool,
     saying to run `npm run fixtures:pull`.
   - The tool's own `UPSTREAM.md` lists only existing files under its own `upstream/<tool>/`.

   `reuse lint` and `node scripts/check/check-licenses.mjs` are repository-wide CI gates, not part of check 6.
   Structure metric 7 (`npm run check:structure`) checks the rule documents' REUSE licence.
7. **Manifest.** `match` must be present and pass the schema: without it set-up never recommends the pack. For a
   directory under `packs/`, `src/engine/catalog/pack-list.generated.ts` equals what `npm run generate:packs` writes
   and lists the pack's id, or, for a private pack, doesn't. Fixture packs elsewhere are unpublished, so no list names
   them; `listed: true` checks the list for them too.
8. **Text.** Every code-rule description, shipped and candidate (not probes), meets the machine-checkable
   [description](#description) criteria (`textFailures`): present, one sentence ending in a period, at most 200
   characters, no backticks, no upstream tool named outside quoted text (`'…'` or `` `…` ``). Messages are not
   checked; the [message](#message) criteria are for review. Check 1 covers a message's `$` placeholders: a message
   naming a capture its clause doesn't bind does not compile. A conversation rule has no description and is left out.

</details>

**Languages not supported yet.** A pack for a language core's [language registry](language-packs.md) lists as
`planned` can land before the engine supports it.

<details>
<summary>How unsupported languages are tested</summary>

A pack may hold code rules for a `planned` language, or rules reading a capability their language has
`not-supported-yet`. Such a rule fails to compile with code `language-not-supported`. The tests report it as
`not supported yet` (its rule test is skipped with the engine's reason) and don't fail: check 1 passes it, and checks 3
and 4 skip it until core reads the language. Any other compile error still fails check 1.

</details>

## Versioning and release

Packs version with changesets and release from `main` through a reviewed Release pull request.

Packs are pre-release until 1.0.0: below it, every change is a `patch`. From 1.0.0:

| Change | Version bump from 1.0.0 |
|---|---|
| A false positive or false negative fixed; wording, examples or expected findings changed | patch |
| A rule added; the core range widened | minor |
| A rule removed or renamed; a default mode made stricter; the core range narrowed | major |

Versions are managed with [changesets](https://github.com/changesets/changesets). A pull request that changes what a
pack publishes adds a changeset naming the pack's package, its bump and a one-line summary for users, then renames it
so it starts with its change type (`fixed-vue-no-unused-vars-filters.md`, [.changeset/README.md](../.changeset/README.md)):

```sh
npx changeset
```

Nobody edits a pack's `version` or `CHANGELOG.md` by hand: at release, `npx changeset version` applies pending
changesets and bumps each pack, and the Release pull request writes its `CHANGELOG.md`.

<details>
<summary>What needs a changeset, and how it is enforced</summary>

- **What counts:** the pack's `package.json` (except `publishConfig` alone, which only steers `npm publish`) or a path
  its `files` lists (`pack.json`, `rules/`, `licenses/`, `LICENSE`, `NOTICE`, `README.md`). Expected findings,
  candidates and `CHANGELOG.md` don't.
- **Enforced:** the **Changesets** job of `release-packs.yml` fails such a pull request with no changeset of its own
  naming that pack (`missingChangesets` in `scripts/release/packs.mjs`); the Release pull request is exempt. `npm test`
  refuses a pending `minor` or `major` changeset for a pack below 1.0.0 (`changesetBumpFailures`, same file), which
  `changeset version` would turn into 0.2.0 or 1.0.0.
- **Packs only:** a changeset naming `@neralabs/rerail` is refused. Core, at the repository root, is bumped by the same
  Release pull request when what it ships changes, with notes pending in `.changelog/` ([release](release.md#release)).
- **New pack:** its first pull request adds it at `0.1.0` with a `patch` changeset for its initial rules, so its first
  release is `0.1.1`.
- **Private pack:** never versioned (`privatePackages` in `.changeset/config.json` is pinned to
  `{ "version": false, "tag": false }`), so it needs no changeset and none may name it: one naming only private packs
  stays pending for ever, and one naming a private pack beside a published one makes `changeset version` fail. The
  Changesets job and `npm test` refuse such a changeset (`privateChangesetFailures` in `scripts/release/packs.mjs`).

</details>

**Release notes.** The summary goes out as written on the pack's GitHub Release: what changed for the person
upgrading, in at most 60 words. Which rules a pack ships and how each starts is not the summary's to say: the Release
pull request writes that from `pack.json` (step 1 below). A summary says what a rule now does differently, or why a
rule no longer ships; a change with nothing else to say, a rule shipped or a README reworded, leaves it empty, and the
changeset still bumps the pack.

<details>
<summary>Release note rules</summary>

The summary says what changed plainly, in a sentence or two, without commit hashes, ticket ids, pull request numbers or
repository-internal maintenance. The **Changesets** job fails a changeset that breaks this
([release notes](release.md#release)), one whose file name gives no change type, and one under Added, Removed or Fixed
that doesn't open with the id of the rule it is about (`changesetRuleRefusal` in `scripts/release/packs.mjs`). The
Release pull request writes each summary as one entry, as written, under its type, and an empty summary as no entry; changesets itself writes no changelog
(`"changelog": false` in `.changeset/config.json`). Rule counts are left to the Release pull request
because changesets pile up between releases and each counts against the `main` it was written on.

</details>

### Release

`.github/workflows/release-pr.yml` versions the packs, with core; `.github/workflows/release-packs.yml` releases them.
Core's `v*` release (`release.yml`) is separate.

1. **Version.** Each push to `main` with anything releasable opens or updates the **Release** pull request. Merging it
   is the release.
2. **Publish.** On each push to `main`, every published pack (not private) whose `package.json` version has no git tag
   `@neralabs/rerail-pack-<id>@<version>` passes the publish gate and is published.
3. **Put back.** Run the workflow from `main` (Actions → Release packs → Run workflow) with the pack's id and an older
   released version, to point `latest` back at it.

<details>
<summary>Step 1: the Release pull request</summary>

- `release-pr.yml` opens it via the `neralabs-ci` app so CI runs on it.
- With changesets pending, it runs `npm run version:packs`: `changeset version`, then
  `scripts/release/sync-pack-lockfile.mjs` to record the new versions in `package-lock.json`. It bumps core too when
  what core ships has changed.
- It then writes each released pack's dated `CHANGELOG.md` section (`scripts/release/pack-changelog.mjs`), in the format
  of core's: [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), with the two differences the changelog's
  header states. The lead says how many rules it ships and how many start on. Under Added, Changed, Deprecated, Removed,
  Fixed and Security, in that order and leaving out types with no entry, come what changed in its rules since the commit
  that released its previous version, one entry per rule (each new rule's id, how it starts and its description under
  Added, those that start on first; those it no longer ships under Removed; those that start differently under
  Changed), then the changesets' entries in file-name order. A `removed-` changeset opening with a rule replaces that
  rule's generated entry, so the notes say why it left. Sections already released keep the format they went out in. A pending changeset with no
  type fails the run, naming the file: one `changeset version` bumps every pack, so no pack can wait for its rename.
- Its description lists every package and version it releases, with changelog entries, then the checklist to tick
  before merging ([release.md](release.md#before-merging)). Review it like any pull request.
- A newer push cancels an older run; with nothing releasable, an open Release pull request is closed.
- CI runs only licence and lint jobs on it: `release-packs.yml` validates and packs every published pack on it, and
  `main` runs everything after merge.

</details>

<details>
<summary>Step 2: the publish gate and dist-tags</summary>

The publish gate is:

- the pack's whole validation (`test/engine/catalog/packs.test.ts`, every check and rule), so a check added to the
  contract gates publishing with no workflow change;
- `npm pack`, and a check that the tarball holds only `package.json` and what `files` publishes, never validation
  material whatever `files` says (`packedFileFailures` in `conformance/pack-validation.ts`);
- the tag, made as soon as the two checks above pass and before anything is published, so a pack's release never
  waits on npm; then, side by side, its GitHub Release and `npm publish --access public --tag latest` with npm trusted
  publishing (the `release` environment, so a reviewer approves the run), with `next` moved up to it.

Set-up and `rerail packs add <id>` fetch under `latest` (on the default update channel), so the version is used from
then on. `next` never trails `latest`: it moves to the version too unless already at a newer one, so the `next`
channel gets the release. A pack held at an older `latest` (step 3) is published under `next` instead and stays held.

Packs publish one by one, and a failed publish leaves its tags and Releases standing. A version is released once it is
tagged and on npm: every run's plan reads npm for each tagged version and publishes again one npm doesn't have, with a
warning naming it. So re-running the run (all jobs or only the failed ones) or the next push to `main` finishes it,
with no time limit, and a run whose plan can't read npm fails rather than call the version released. One the registry
already has counts as published, and has `next` moved up. A version older than the pack's `latest` (an old run re-run
after a newer one shipped) is never published, since that would move `latest` back; the run fails naming it. The
**Published dist-tags** job lists each published pack's dist-tags in the run summary, read with no npm login. It fails
if a pack can't be read that way (it isn't public), if `latest` (`next` for a held pack) is not the version just
published, or if `next` is older than `latest`. It gates nothing. A pull request touching packs runs the gate and
packing as a dry run.

</details>

<details>
<summary>Step 3: holds and promotion</summary>

The first job lists in the run summary what will move. After a reviewer approves the `release` environment, the
promote job points `latest` at that version and tags it `held` there.

- Publishes and runs with no inputs leave a held pack's `latest` alone (the summary names it). Promoting the pack by
  id again, at its `package.json` version, releases the hold.
- With no inputs, the workflow points `latest` at every published pack's released version not yet `latest`, less any
  held.
- Each promotion moves `next` up to the version unless it is already newer, so a put-back leaves it. The promote job
  then lists each promoted pack's dist-tags, and fails if `latest` is not the version or `next` is older than
  `latest`.
- A pack that fails to move, or whose dist-tags can't be read, doesn't stop the others; the run fails naming it and
  can be rerun. A never-released version fails the run. These runs go one at a time.

</details>

**GitHub Releases.** Every released pack version gets one on its tag `@neralabs/rerail-pack-<id>@<version>`. The notes
are that version's `CHANGELOG.md` section, so they are written once, by the Release pull request and in the changeset
summaries.

<details>
<summary>How GitHub Releases are made</summary>

- Sections from before the Release pull request wrote them lose their one `### Patch Changes` heading and the commit
  hash their entries carry.
- Notes past GitHub's 125,000 characters go out as the section's lead and a link to the changelog at the tag
  (`fittedNotes`).
- Once the **Tag each release** job has tagged them, the **GitHub Releases** job makes one per tagged version
  (`scripts/release/github-release.mjs`), beside the npm publish rather than after it, so npm can neither hold back
  nor skip a Release. Rerun, it rewrites an existing Release's notes and flags.
- A pack's Release is never the repository's latest release, which is core's. It is a pre-release only for a held pack
  published under `next`.
- Releases missing for older tags, or from a run that failed before its Release, are made by hand with the script's
  backfill, which skips a version whose changelog has no section ([release.md](release.md#release)).

</details>

The scripts are in `scripts/release/`. To check a pack's tarball locally:
`node --import tsx scripts/release/packs-pack.ts --out /tmp/rule-packs <id>`.

### Publishing a new pack

A pack's first version is published by hand; the workflow publishes later ones. Steps:
[publishing-a-new-pack.md](publishing-a-new-pack.md). To publish a private pack, remove `"private": true` in a pull
request that adds its changeset and `publishConfig`, then follow those steps.

<details>
<summary>Why the first publish is manual</summary>

npm configures a trusted publisher only for an existing package, and a new configuration expires unless a publish uses
it within 2 days. Until the first publish, each release run reports the pack as needing it and fails after publishing
the others.

</details>

## Licensing

Rerail's own material in a pack is Apache-2.0; material copied or adapted from upstream keeps its licence.

- **Records:** upstream licences are recorded per repository in `licenses/<project>/UPSTREAM.md`.
- **`license`** names every licence the pack's published files carry, e.g. `Apache-2.0 AND MIT` for rules adapted from
  an MIT tool (check 5).
- Only licences on the repository allowlist (`scripts/check/licence-allowlist.json`) are accepted.

<details>
<summary>Rule documents, test data and REUSE</summary>

- **Rule documents** (`rules/`, and those under `candidates/`) carry the same expression, `Apache-2.0 AND <upstream>`,
  in `REUSE.toml`, so `reuse lint` agrees with what the package publishes. Whether adapted rule documents need the
  upstream licence is an open legal question.
- **Test data** is not in git; its licences are the `UPSTREAM.md` records and each archive's `LICENSES.md`. For some
  tools (the rust and vue packs' entries), `REUSE.toml` annotates copied paths, pulled and under `publish/`, so
  `reuse lint` passes while a copy is staged.

</details>

### UPSTREAM.md

A pack keeps two kinds of provenance record in git.

- **`licenses/<project>/UPSTREAM.md`**, one per upstream repository, beside its licence text (verbatim): the source,
  the path patterns taken, and what was not copied.
- **`upstream/<tool>/UPSTREAM.md`**, when Rerail wrote files for the tool to score: one row per such file.

<details>
<summary>Format of licenses/&lt;project&gt;/UPSTREAM.md</summary>

It names patterns, never single files; the per-file record is in each archive ([below](#provenancejson-and-licensesmd)).

- **`License:` line:** an SPDX expression (ids, `AND`, `OR`, `WITH` and parentheses), up to the note after it.
- **`Source` section:** a table row with the repository, a tag, the 40-hex commit and the licence.
- **`Taken from it` table:** per row, backticked pack-path globs starting with `upstream/` (first column), the upstream
  paths, and how that kind of file was made (last column). A How cell's copy kind is the most modified of `verbatim`,
  `extracted`, `renamed` and `adapted` it names (`verbatim …; or adapted …` is `adapted`). A glob holds at most two
  `**`; a `**` segment matches zero or more segments (`a/**/x` matches `a/x`).
- **`Not copied` lines:** one per upstream path nothing is copied from (upstream material derived from a copyleft
  project, such as Ruff's fixtures for its pylint ports). Each is exactly
  ``- Not copied: `<upstream path prefix>` as `<pack path glob>` — <project>, <SPDX id>``, pack-path glob required. A
  list item meant as one but spelled otherwise fails: `* Not copied:`, `- Not Copied:`, indented, or without the
  backticks.

</details>

<details>
<summary>Format of upstream/&lt;tool&gt;/UPSTREAM.md</summary>

A table with one row per file, backticked pack path under `upstream/<tool>/` first
(`` | `upstream/ruff/fixtures/rerail/noqa/B002.py` | written by Rerail | ``).

- Those files are authored, Rerail's (Apache-2.0), and add no licence to `licenses/` or to the package's `license`.
- Removing a row removes the file from the archive at the next publish, so such a pull request is a publish request
  ([docs/upstream.md](upstream.md#removing-data)).
- An `authored/` corpus needs no rows.

</details>

### provenance.json and LICENSES.md

Each test data archive holds `upstream/<tool>/provenance.json` (every file with its source, upstream path and how it
was made) and `upstream/<tool>/LICENSES.md` (each source's licence and licence text).

<details>
<summary>How they are written and read</summary>

Both are derive output. The generator writes both last, through `scripts/pack-data/provenance.mjs`, which refuses to
write either when the files disagree with the pack's `UPSTREAM.md` records
([docs/upstream.md](upstream.md#what-the-generators-write)). They are for audit: no licence check reads them. The
publish workflow's prep reads only the key set of provenance.json's `files`, to refuse an archive whose files it does
not describe.

A `snapshot` key appears only in archives bootstrapped by the one-off snapshot taken when the records moved from
per-file rows to patterns, until the tool-pack is derived again. A samples archive's `LICENSES.md` lists each source
repository, its licence and the files taken from it ([docs/upstream.md](upstream.md#sample-licences)).

</details>

### Licence of each file

Pack check 6 and the publish workflow's prep decide each test data file's licence by the same rules: `Not copied`
first, then Rerail's own files, then the longest matching `Taken from it` glob.

<details>
<summary>The exact rules</summary>

They share parsing and pattern choice (`readUpstreamSources`, `decidingPattern` and `inToolSubdirectory` in
`scripts/s3/pack-fixtures.mjs`):

1. A file under a `Not copied` glob is refused, even one the tool's `UPSTREAM.md` lists or one under `authored/`.
2. Otherwise a listed file, or one under `authored/`, is Rerail's Apache-2.0.
3. Any other file takes the licence of the longest `Taken from it` glob covering it. Equal-length longest globs that
   disagree on licence or copy kind are refused.
4. A file in a subdirectory of `upstream/<tool>/` that none of these takes is refused. One directly in
   `upstream/<tool>/` (expected findings, `cases.json`, `provenance.json`) is Rerail's.

The publish adds one rule: a file from an Apache-2.0-only source is refused unless that glob's kind is `verbatim`,
since a modified Apache-2.0 file must carry its change notices.

A licence passes when some way to satisfy its SPDX expression uses only allowlisted licences (`AND` binds tighter than
`OR`); a malformed expression is refused. A sample's licence is its repository's recorded one
([docs/upstream.md](upstream.md#sample-licences)).

</details>
