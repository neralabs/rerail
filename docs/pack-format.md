# Rule pack structure and validation contract

Status: the layout and the validation contract (checks 1 to 8) are implemented. `test/engine/catalog/packs.test.ts` validates every
`packs/*` directory: `python`, `php`, `blade`, `rust`, `typescript`, `javascript`, `react`, `vue`, `node`, `next`, `sql` and `complexity`. Core
ships none of them: set-up and `rerail packs add` install them from npm into `$RERAIL_HOME/packs`, and core reads them
from there; it lists them by name only, in `src/engine/catalog/pack-list.generated.ts` ([rule packs](rule-packs.md#installing-packs)).
Each pack is published on its own by the [release workflow](#release). A pack introduces no new file format: it
combines the two schemas the repository already has. To start one, see [language packs](language-packs.md#start-a-pack).

Core has no rules of its own: every rule document it ships is in a pack. Like every By Rerail rule, a pack's rules run
only in repositories that add them (or the pack). The engine's `capabilities.json`, which every code rule compiles
against, is `native/src/code_rule_dsl/capabilities.json`.

## Principles

1. **A pack is data.** Rule documents plus the evidence that they behave. It contains no code, not even to decide
   whether it fits a repository (its `match` is globs and dependency names), and never imports the engine: core runs
   its rules, and the repository's tests validate it.
2. **The directory is the contract.** What a pack must prove follows from which files it has. There is no separate
   validation manifest.
3. **Every rule is proven.** By examples it must catch and let pass, or, for a rule ported from another tool, by that
   tool's expected findings on its own tests, with every disagreement declared.
4. **Packs version on their own.** Each pack is its own npm package, released separately from core.

## Layout

```text
packs/<id>/
├── package.json        npm package @neralabs/rerail-pack-<id>: licence, files, core range
├── pack.json           the pack: rerail.rule-pack/v1, as today
├── README.md           what the pack covers, for people
├── LICENSE, NOTICE     the pack's licence and notice
├── rules/
│   └── <rule id>.json  code-rule documents (rerail.policy/v1), each named for its id
├── licenses/
│   └── <project>/      upstream LICENSE (verbatim) and UPSTREAM.md (per repository: source, path patterns taken, not copied)
│
│   only for packs that port rules from another tool:
├── conformance.json    the conformance pack schema of conformance/languages/pack.ts, as today
├── divergences.json    declared disagreements with the tools, as today
├── upstream/
│   └── <tool>/         gates.yaml, the generator, test-data.lock, samples.lock and SAMPLES.md (optional), and
│                       UPSTREAM.md when Rerail wrote files for the tool to score; the tool's copied tests
│                       (fixtures/, rename variants under fixtures/renamed/) and expected findings (*.expected.json)
│                       are pack test data, not in git (see Pack data); publish/ while a pull request publishes data
└── candidates/         ports still being evaluated: scored and reported, never gated, never shipped
```

`<id>` is lowercase letters, digits and dashes. It is the directory name, the `id` in `pack.json`, and the npm name
`@neralabs/rerail-pack-<id>`.

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

- The core version range lives in `peerDependencies`. It is a declaration, not an import: a pack has no
  `dependencies`. The `-0` admits core's prereleases (`0.1.0-next.0`). Rerail's catalog enforces it: an installed pack
  whose range excludes the active core is left out ([rule packs](rule-packs.md#installing-packs)). npm does not, since
  core is never installed beside the packs in `$RERAIL_HOME/packs`.
- The peer is `optional`: `packs/*` are workspaces of this repository, whose root package is core itself, and npm
  would otherwise try to install a published core into them.
- `files` publishes the pack and its licence records. `conformance.json`, `divergences.json`, `upstream/` and
  `candidates/` are validation material: they stay in the repository and never reach npm, which keeps copied
  upstream tests out of the package.
- `repository` names this repository and the pack's directory: npm's trusted publishing requires the repository it
  publishes from. `publishConfig.access` is `public`, like core.
- The schema is `rulePackPackageSchema` in `src/engine/catalog/rule-packs.ts`.

### pack.json

Today's `rerail.rule-pack/v1` ([format](rule-packs.md#format)): `id`, `name`, `category`, `covers`, `reason`,
`match`, `honourSuppressionComments`, and per rule `id`, `title`, `group`, optional `mode`, `on` and
`examples: { caught, passes }`. One change: a rule now names its document, `rules/<id>.json`, so a pack is
self-contained. A pack may ship no rules (`"rules": []`) while all its ports are candidates; such a pack is never
recommended. A ported rule starts at the level upstream's recommended configuration gives it: a rule upstream sets to
`warn` (an ESLint-family `"warn"`, a Clippy lint whose default level is `warn`, a PHP_CodeSniffer warning) carries
`"mode": "log"` in pack.json, so it is recorded and steers no one until a repository switches it to `steer`; a rule
upstream sets to `error` or `deny` keeps its document's `steer`. Tools with no warning level (Ruff, Bandit, SQLFluff,
PHPStan, Psalm, tlint, blade-parser) keep the document's mode, and the document itself stays a faithful port with no
level. `document` is required (`rulePackFileSchema` in `src/engine/catalog/rule-packs.ts`); core reads each rule's
document from the pack:

```json
{ "id": "complexity-cyclomatic-python", "document": "rules/complexity-cyclomatic-python.json", "title": "Keep Python functions at cyclomatic complexity 6 or lower", "group": "Complexity" }
```

An example's `path` is required in a pack: it decides the file's language and must be inside the rule's scope.

**Rule ids.** Every pack uses one id style, because people turn rules on and off by id and an id must not change once
its pack is published: lowercase letters, digits and dashes (`RULE_PACK_RULE_ID` in `src/engine/catalog/rule-packs.ts`),
the upstream tool's name then its rule for a port (`eslint-no-empty`, `typescript-eslint-no-explicit-any`,
`ruff-b011-assert-false`, `n-no-sync`), and the pack's name then the rule for a rule of Rerail's own
(`complexity-cyclomatic-python`). The id is the same string everywhere: the rule's `id` in `pack.json`, its document's
`id`, its document's file name (`rules/<id>.json`, or `candidates/rules/<id>.json` while it is a candidate, so it keeps
its id when it ships), and its port's `id` in `conformance.json`. Check 1 enforces this, and the pack tests reject an id
that two documents in any packs share.

### conformance.json, divergences.json, upstream/

Today's schemas from `conformance/languages/pack.ts`, moved next to the rules they validate. The runner reads a pack's
expected findings from `upstream/` unless `conformance.json` sets `oracles`:

- `conformance.json`: the language family, the engine capabilities the pack declares (with exemptions and their
  evidence), the pinned tools, and for each ported rule the upstream tool and rule it reproduces,
  how findings compare (`line` or `line+column`) and the capabilities it exercises. `suppressionExemptions` lists the
  ports, by `id`, whose documents need not carry their tool's suppression comment yet, each with its `reason` (check
  4). A port whose document is under `candidates/` is scored and reported but never fails a check; it is scored on the
  fixtures, whose expected findings must cover it. A port may
  set `fixtures`, `path.matchesGlob` patterns over the tool's fixture paths: it is then evaluated and scored only on
  the fixtures they match, the tests its upstream configuration ran on. That is how a tool whose tests run each rule
  with several option sets is scored: one document per option set, each on its own tests.
- `upstream/<tool>/fixtures.expected.json`: the expected findings, what the tool reported on its own tests, written by
  the generator in the same directory and run through `npm run fixtures:derive` (each tool-pack's recipe in
  `scripts/pack-data/recipes/`, runtimes in `scripts/pack-data/runtimes/`; tools live under `.rerail/`, gitignored)
  ([docs/upstream.md](upstream.md#take-a-new-snapshot)). Where a tool's tests state
  their expected findings (as markup in the test files), the generator reads them instead of running the tool: the
  tool's own test suite requires it to report exactly those. The ESLint-based generators share
  `scripts/pack-data/eslint-expected.mjs`. Its `source` block names the upstream `origin` (repository `@` commit), the
  `rules` the run reported, the `files` (units: a path, or a list of paths for a multi-file unit), and `units`, each
  unit's upstream origin by its key (its path, or its first path): `rules`, the upstream rules whose tests the unit was
  written from. A unit without an entry (Rerail's probes, or one whose origin is unknown) is scored but is no rule's
  own evidence for `stays-silent`.
- Rename variants (the ECMA tool-packs): the generator writes a renamed copy of every upstream unit under
  `fixtures/renamed/<the original's path>`, its identifiers and string and number literal contents renamed by
  `scripts/pack-data/rename/ecmascript.ts` (same length and shape, one bijective map per file, keywords, the standard
  library and common host names protected), and lints it with upstream like any unit. Its `source.units` entry adds
  `renamedFrom`, the key of the original unit, which must be an original with an origin. The conformance check scores
  variants apart from the originals (`holds-under-rename`), so a port that matches the tests' names or literals rather
  than the construct fails. A family without variants takes its overfit evidence from its samples alone.
- `upstream/<tool>/gates.yaml` (a gated tool-pack): the upstream manifest the conformance check reads: the pins, the
  file types upstream lints, and per upstream rule its declared deprecation and recommended configuration
  ([conformance/README.md](../conformance/README.md#declared-upstream-facts-gatesyaml)).
  Its `watch:` block says what the nightly upstream report ([docs/upstream.md](upstream.md)) compares between the
  pinned commit and upstream's latest release. Every path is relative to upstream's repository root; a glob takes `*`
  within a path segment and `**` across segments; `{rule}` stands for each ported rule, which is conformance.json's
  `upstream.rule` for this tool without `rule-prefix`, plus `extra-rules`:

  ```yaml
  watch:
    rule-prefix: "unicorn/"          # optional: removed from upstream.rule to give {rule}
    extra-rules: [NoArgumentsValidator]  # optional: rules the generator reads that conformance.json does not port
    tests: ["test/{rule}.js"]        # the tests the generator reads; or { file: fixtures.json }
    code: ["rules/{rule}.js"]        # each ported rule's code; or { index: clippy-declared-lints | ruff-codes }
    shared: ["rules/utils/**"]       # code every ported rule depends on; no {rule}
    new-tests: ["tests/{rule}/**"]   # optional: where upstream adds tests for a ported rule that the generator does not read
    rules:                           # optional: a rule's own tests or code, in place of the {rule} patterns
      DebugDirectiveValidator: { tests: ["tests/Validation/DebugDirectiveTest.php", "src/Compiler/**"] }
  ```

  `{ file: fixtures.json }` watches the paths a data file beside the generator lists for each rule
  (`{ "rules": { "<rule>": ["<upstream path>", …] } }`), for a generator that keeps its tests in a list rather than
  by convention. `{ index: … }` names the upstream file the report reads to find each rule's code
  (`clippy_lints/src/declared_lints.rs`, `crates/ruff_linter/src/codes.rs`). `expandWatch` in
  `conformance/languages/watch.ts` resolves the block. A `new-tests` glob may cover tests the generator reads too; a
  file counts as a new test only when `tests` does not cover it. `test/scripts/upstream/watch-coverage.test.ts` checks
  that `tests` covers every upstream file the tool-pack's pulled `provenance.json` records as read.
- `upstream/<tool>/test-data.lock`, `samples.lock` and `SAMPLES.md`: the pins of the tool-pack's pack data
  ([Pack data](#pack-data)).
- `divergences.json`: each place Rerail and the tool disagree, with its kind (`upstream-wrong`, `not-comparable`,
  `rerail-unknown`) and the evidence checked ([conformance/README.md](../conformance/README.md#divergences)).

## Pack data

A tool-pack's test data and real-code samples are not in git and never reach npm. Each kind is one archive in S3 that a
tracked lock in the tool directory names; [docs/upstream.md](upstream.md) says how the data is made, published and
judged, and [CONTRIBUTING.md](../CONTRIBUTING.md#pack-test-data) who does what.

**Test data** is the files under `packs/<id>/upstream/<tool>/` that `PACK_FIXTURE_PATTERNS`
(`scripts/s3/pack-fixtures.mjs`) matches: the tool's test cases (`fixtures/`, `tests/`, case lists such as
`cases.json` and `*.cases.json`, `fixtures.upstream-check.json`, `lizard-*.tsv`), its expected findings
(`*.expected.json`), `authored/`, and the archive's provenance files, `provenance.json` and `LICENSES.md`
([below](#provenancejson-and-licensesmd)). `.gitignore` covers the patterns. The data is of two kinds:

- **authored**: what Rerail wrote for the tool to score, which nothing can regenerate: the files the tool's own
  `upstream/<tool>/UPSTREAM.md` lists ([UPSTREAM.md](#upstreammd)), and everything under `upstream/<tool>/authored/`, a
  contributor's code corpus with its ground truth where the corpus is no public repository a pin can name;
- **derived**: everything else, upstream's tests copied or derived and the tool's expected findings, which the
  generators make from the pins.

**Samples** are the tool-pack's labelled real files ([docs/upstream.md, "Samples"](upstream.md#samples)). The
archive's layout is `scripts/samples/format.ts`: each sample is `<sample-id>/<source path>` beside its
`<source path>.expected.json` (the source repository, commit, path, sha256 and SPDX licence, and per rule the labelled
lines), with any context files the sample needs under the same `<sample-id>/`, and `LICENSES.md` at the root. The
tracked `SAMPLES.md` beside `samples.lock` lists the set's repositories with their licences, commits and file counts,
says which licence texts the archive's `LICENSES.md` holds, and how the set was trimmed if it was. A tool-pack without
a `samples.lock` has no samples.

**Archives and locks.** An archive is the canonical tar of its files (one regular-file entry per file, sorted by its
path's UTF-8 bytes, at its path relative to the tool directory, mode 0644, owner 0, time 0, no directory entries;
`scripts/s3/test-data-archive.mjs`), compressed with zstd, at
`s3://nera-test-fixtures/rerail/archives/<pack>/<tool>/<sha256>.tar.zst` for test data and
`s3://nera-test-fixtures/rerail/archives/samples/<pack>/<tool>/<sha256>.tar.zst` for samples (us-east-2). Every
write is create-only and the bucket is versioned, so changed data is a new archive and a new lock. `test-data.lock`
and `samples.lock` share one format, which `parseLock` reads: `{ "format": 1, "sha256", "files", "bytes" }`, the
tar's sha256 (not the compressed file's, so a different zstd build changes nothing), its file count and its bytes.
Every tool directory with a tracked file outside `publish/` has a `test-data.lock`; `samples.lock` is optional.

**Pulling.** `npm run fixtures:pull` (`-- --pack <id> [--tool <tool>]` for some, `-- --dry-run` to see what it would
fetch) keeps each archive in `.rerail/pack-data/archives/` and fetches from S3, with the AWS CLI and read access to the
bucket, only what it has not fetched before. It verifies every archive against its lock before it extracts any, and
refuses an archive with a path that has a control character, is not NFC, or equals another's ignoring case. It
extracts the test data into the tool directory, lays the tool-pack's `publish/` over it, deletes the derived files on
disk that neither holds (stale) and keeps authored ones, saying so; it extracts samples under
`.rerail/pack-data/samples/<pack>/<tool>/<sha256>/`. It never writes or reads through a symbolic link: when `packs/`,
a tool directory, its `publish/` or a directory a file goes into is one, it stops before writing anything. Every
consumer (checks 4 and 6, the conformance runner, the generators, the Rust tests) says to pull when the data is
absent. Pull again after a checkout or merge that changes a lock.

**In git** under `upstream/<tool>/`: `gates.yaml`, the generators, `UPSTREAM.md`, `test-data.lock`, `samples.lock`,
`SAMPLES.md`, and, while a pull request publishes data, `publish/`
([docs/upstream.md](upstream.md#publishing-pack-data)). Structure metric #3 fails while pack test data is tracked in
the pulled layout, a tool directory lacks its `test-data.lock`, a lock does not parse, or a `publish/` is tracked.

## Rule text

A rule document carries two kinds of text, for two readers. Its `description` is for people browsing the dashboard and
the rule catalog; the coding agent never reads it. Each clause's `violation.message` is for the agent, which reads it in
the steer block Rerail writes around it:

```
Rerail found 2 problems in your last edit:
- src/grade.js:1:8 — <message> [complexity-cyclomatic-javascript]
  Suggested fix: delete 14:3–14:12
- src/api.py:42:5 — <message> [bandit-b113-request-without-timeout]
Fix these, or tell the user in one line why you won't.
```

The block already gives the location, the rule id, the lead and the latitude ("fix or explain"), so the message only
advises what to do about this code. Check 8 enforces the description criteria a machine can check; review covers the
rest of the description criteria and all of the message criteria (`just rerail-guidance-review` shows every rule's
description, messages and the block for each of its cases).

### Description

1. Readable at a glance by a developer, in plain English.
2. States the intent: what the rule wants the developer to do or avoid.
3. No code or source material: no snippets, regexes, AST node kinds, globs, JSON, config keys, token lists or
   backticks. Naming the API or construct the rule is about is fine when the intent needs it ("dd() and dump() calls",
   "mutable default arguments", "useEffect").
4. No origin or source: no upstream tool or rule names, no versions, no "port of", no conformance or divergence notes.
5. Clear and concise.
6. Accurate: it matches what the clauses and scope actually catch. It doesn't overpromise (if tests are excluded and
   that matters, don't claim "everywhere") or underdescribe.
7. One voice for the whole repository: one imperative sentence that starts with a verb (Avoid, Use, Prefer, Keep,
   Remove, Add, Declare, Close, Handle, Don't…) and ends with a period, optionally followed by `; <short reason>`.
8. At most 200 characters, a hard cap.
9. A reason after the semicolon only when it isn't obvious and still fits.
10. Siblings are told apart: when similar rules exist for several languages or file types, name the language or file
    type (`complexity-cyclomatic-python`, `-typescript`, `-javascript`).
11. Meaningful thresholds stay ("above 6", "more than 5 parameters"); implementation trivia goes.
12. It doesn't just repeat the rule's title or its message.

Detail a description leaves out (exact counting rules, edge cases, deviations from upstream, scope detail) goes in the
pack's README, under "Rule details". For example: `Avoid dd() and dump() calls; they leak debug output to users.`

### Message

The message tells the agent what to change and, when the change could be wrong, when not to.

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

| Instead of | Write |
|---|---|
| `Bandit B113 (request-without-timeout)` | `Pass a finite timeout to this HTTP call; without one a slow server can hang the program.` |
| `zip() without an explicit strict= parameter` | ``Pass `strict=` explicitly: `strict=True` when lengths must match, `strict=False` when truncation is intended.`` |
| `` Comparison to `None` should be `cond is None` ``, one clause for `==` and `!=` | One clause each: ``Use `is None` instead of `== None` for this comparison.`` and ``Use `is not None` instead of `!= None` for this comparison.`` |

These criteria are for reviewers and never block a merge: no check reads a message's text.

## The validation contract

Implemented: `validatePack(directory, { listed? })` in `conformance/pack-validation.ts` runs
checks 1 to 8 over one pack and returns each check's failures and each rule's status (`ok`, `failed` or `not-supported-yet`).
`test/engine/catalog/packs.test.ts` calls it for every directory under `packs/`, with one test per check and one per rule, and has
the validator's own tests over the fixture packs in `fixtures/packs/`. `npm test` runs it with the local core.
Running it against the newest published core inside each pack's range is for pack CI, which does not exist yet.

The contract is the floor every pack meets. A ported rule must also pass the [conformance check's gate](../conformance/README.md#the-gate)
to ship: it judges the rule on upstream's cases and their rename variants (no unexplained false positive or miss), on
real code, for size, and for silence where it should be silent; it blocks CI for shipped rules and only reports
candidates.

```sh
npx vitest run test/engine/catalog/packs.test.ts                         # every pack: checks 1 to 8, each rule's status
npx vitest run test/engine/catalog/packs.test.ts -t "pack python"        # one pack
```

Check 4 scores ported rules on the tools' fixtures. The rename variants and the real-code samples are scored by the
conformance check (`npm run test:conformance`), not here. To see every false positive and false negative of one
ported rule: `node --import tsx conformance/languages/cli.ts --pack <id> --rule <rule> --fixtures`.

| # | Check | Fails when |
|---|---|---|
| 1 | Rules | A file in `rules/` is not listed in `pack.json`, a listed `document` is missing, is not named for its rule's id, its `id` differs from the listing, or it does not compile against core's capabilities; a rule id breaks the [id rule](#packjson); a candidate is not `rerail.policy/v1`, not in a `rules/` directory or not named for its id; a port's `id` differs from its document's; a `conformance.json` under `candidates/` does not parse or names a missing document |
| 2 | Proof | A rule has neither `examples` nor a port entry in `conformance.json` |
| 3 | Examples | A `caught` example produces no violation of its rule, or a `passes` example produces one |
| 4 | Ports | With `conformance.json`: a shipped ported rule's violations and the tool's expected findings differ outside `divergences.json`; a divergence matches nothing (stale); a declared capability has neither a ported rule nor an exemption; a port's violation does not carry the suppression comment its tool honours, outside `suppressionExemptions`, or an exemption is stale; a port carries `suppress.oxlint` and `suppress.eslint` on different violations; a violation carries `suppress.eslint` without the Biome rules the same as its ESLint rule under `suppress.biome`, or `suppress.biome` without `suppress.eslint` |
| 5 | Package | `package.json` misnames the pack, names another repository or directory, is not `public`, has `dependencies`, or publishes validation material; its licence disagrees with what `licenses/` records |
| 6 | Licences | A pulled file falls under a `Not copied` glob, or in a tool's subdirectory is neither authored nor under a `Taken from it` glob; a declared glob matches no pulled file; an `UPSTREAM.md` is malformed, or records a licence off the allowlist; the tool's own `UPSTREAM.md` lists a file outside its tool directory or one that does not exist |
| 7 | Manifest | `pack.json` has no `match`, or its `match` fails the schema; core's generated pack list does not name the pack or is stale |
| 8 | Text | A rule document's description fails a machine-checkable [description](#description) criterion |

Precisely:

1. `pack.json` must parse (`rulePackFileSchema`) and its `id` must be the directory name. Each rule's `id` is
   lowercase letters, digits and dashes, and its `document` is `rules/<id>.json`. Each document must be a valid code
   rule (`rerail.policy/v1`, the host's schema) and compile in the engine with core's
   `native/src/code_rule_dsl/capabilities.json`, as the catalog compiles it. Every rule document under `candidates/`
   (each JSON file in a `rules/` directory there, and any file there whose `format` names a policy, such as the older
   `tempo.policy/v1`) must be `rerail.policy/v1`, sit in a `rules/` directory and be named `<id>.json`
   (`candidates/rules/<id>.json`, `candidates/probes/rules/<id>.json`); candidates need not compile. Every port in the
   pack's `conformance.json`, and in each `conformance.json` under `candidates/` (probes), has its document's `id`.
   Check 4 loads only the root `conformance.json`, so check 1 also fails a `conformance.json` under `candidates/` that
   does not parse (`packSchema`) or has a port whose document does not exist.
2. A port entry is a `conformance.json` rule whose `policy` is the rule's `document`.
3. Each example is evaluated alone, as a one-file repository at its `path`, through the engine (`evaluateExample` in
   `conformance/pack-validation.ts`). Its path must be in the rule's scope, and it must produce no engine failure. A
   violation counts when its code is one of the rule's clauses' codes.
4. The fixture tier of the conformance runner (`conformance/languages/runner.ts`, the `language-conformance` test's),
   over the production workspace path: an unexplained false positive fails on every source, an unexplained false
   negative on the fixtures only, as do engine failures, expected-findings errors and stale divergences or exemptions.
   Ports in `conformance.json` that `pack.json` does not list (candidates) are scored too; their false positives,
   false negatives and engine failures are reported and fail nothing.

   Every port, candidates included, must also honour the suppression comments of its upstream tool, as the tool
   does: each violation of its document carries the `suppress` key below naming the port's `upstream.rule`. The
   data has to be in the document, because `conformance.json` never ships. A port of any other tool (Bladestan,
   tlint, blade-parser, lizard) needs none.

   | `upstream.tool` | The comment it honours | `violation.suppress` |
   |---|---|---|
   | `eslint`, `eslint-plugin-*`, `typescript-eslint` | `// eslint-disable-line <rule>` and the other `eslint-disable` forms | `eslint.rules` |
   | `ruff` | `# noqa: <code>` | `noqa.codes` |
   | `bandit` | `# nosec <id>` | `nosec.codes` |
   | `phpstan` | `@phpstan-ignore <identifier>` | `phpstan.identifiers` |
   | `psalm` | `@psalm-suppress <issue>` | `psalm.issues` |
   | `phpcs` | `phpcs:ignore <sniff>` | `phpcs.sniffs`: the sniff or one of its message codes, `<sniff>.<code>` |

   A port whose document does not model the comment is listed in `conformance.json` `suppressionExemptions`, as
   `{ "rule": <port id>, "reason": … }`: one whose expected findings were made with the comments ignored (Bandit's `--ignore-nosec`),
   or one with a clause the engine cannot suppress (it refuses `suppress` on a `regex` query). An exemption for a port
   that carries the suppression on every violation, or whose tool honours no comment, fails as stale.

   A port of a rule that Oxlint 1.88.0 also implements carries `oxlint.rules` too, with Oxlint's rule name without its
   plugin (`rules-of-hooks`, `no-console`), so `oxlint-disable` comments silence it as they silence Oxlint's. The two
   name the same rule and Oxlint reads `eslint-disable` comments as well, so once any violation of a document carries
   `suppress.oxlint`, each violation that carries `suppress.eslint` must carry it, and none may carry it alone.

   A port of a rule Biome 2.5.15 has a rule the same as (its `sources` mark the ESLint rule `.same()`, not
   `.inspired()`: `useHookAtTopLevel` for `react-hooks/rules-of-hooks`) carries `biome.rules` too, with Biome's full
   name (`lint/correctness/useHookAtTopLevel`), so `biome-ignore` comments naming it, its group or `lint` silence the
   port as they silence Biome's rule. Each violation that carries `suppress.eslint` must carry the Biome rules the same
   as the ESLint rules it names, and none may carry `suppress.biome` without `suppress.eslint`. The mapping,
   `conformance/biome-same-sources.generated.ts`, is read from Biome's source by
   `scripts/generate/packs/biome-suppression.mjs`, which also writes `suppress.biome` into the pack documents.
5. The npm name is `@neralabs/rerail-pack-<id>`, `repository` is `{ "type": "git", "url": <the root package.json's
   repository url>, "directory": "packs/<id>" }`, and `publishConfig.access` is `public`. `files` must publish
   `pack.json`, `rules`, `LICENSE` and, when present, `licenses`, and no entry may cover `conformance.json`,
   `divergences.json`, `upstream` or `candidates`. Some way to satisfy `license` must name exactly Apache-2.0 plus,
   for each `licenses/<project>/UPSTREAM.md`, one way to satisfy the SPDX expression its `License:` line records: an
   `OR` there lets the package elect either side (`MIT OR Apache-2.0` with `Apache-2.0 AND MIT`), and an `AND` must
   be named whole.
6. The pack's provenance records ([UPSTREAM.md](#upstreammd)) must parse and agree with the pulled test data. Each
   `licenses/<project>/UPSTREAM.md` needs a well-formed `License:` expression that some way to satisfy uses only
   licences on the repository allowlist (`scripts/check/licence-allowlist.json`), a complete `Source` row, and only
   well-formed `Not copied` lines; the pack needs a `LICENSE`. Every pulled file in a subdirectory of
   `upstream/<tool>/` (the tool's copied tests: `fixtures/`, `tests/`) must fall under no `Not copied` glob, and then
   be authored or fall under some project's `Taken from it` glob, where equal-length longest globs that disagree on
   licence or copy kind fail; files directly in `upstream/` or `upstream/<tool>/` (expected findings, generators) are
   Rerail's. Each declared glob must match at least one pulled file (a checkout without the pack's test data fails
   once per tool, saying to run `npm run fixtures:pull`). The tool's own `UPSTREAM.md` may list only files under its
   own `upstream/<tool>/`, and every file it lists must exist. Check 6 and the publish decide a file's licence by the
   same rules ([Licence of each file](#licence-of-each-file)). `reuse lint` and
   `node scripts/check/check-licenses.mjs` are repository-wide gates in CI, not part of check 6, and structure metric 7
   (`npm run check:structure`) checks the rule documents' REUSE licence.
7. `match` must be present: without it set-up never recommends the pack. For a directory under `packs/`,
   `src/engine/catalog/pack-list.generated.ts` must list the pack's id and equal what `npm run generate:packs` writes.
   Fixture packs elsewhere are not published, so no list names them; `listed: true` checks the list for them too.
8. Every rule document's description, shipped and candidate (not probes), meets the parts of the
   [description](#description) criteria a machine can check (`textFailures`): it is present, one sentence ending in a
   period, at most 200 characters, with no backticks, and names no upstream tool outside quoted text (`'…'` or
   `` `…` ``). Violation messages are not checked: the [message](#message) criteria are for review. A message's
   `$` placeholders are checked by check 1, since a rule whose message names a capture its clause doesn't bind does not
   compile.

**Languages not supported yet.** A pack may hold code rules for a language core's
[language registry](language-packs.md) lists as `planned`, or rules that read a capability their language has
`not-supported-yet`. Such a rule fails to compile with the code `language-not-supported`. The tests report it as
`not supported yet` (its rule test is skipped with the engine's reason) and do not fail: check 1 passes it, and checks
3 and 4 skip it until core reads the language. Any other compile error still fails check 1. So a pack for a planned
language can land before the engine supports it.

A public `rerail pack check` command, for packs written outside this repository, can wrap the same checks later; it
is not part of this contract.

## Versioning and release

Packs are pre-release until 1.0.0: below it, every change to a pack is a `patch`. From 1.0.0 the bump follows the
table:

| Change | Version bump from 1.0.0 |
|---|---|
| A false positive or false negative fixed; wording, examples or expected findings changed | patch |
| A rule added; the core range widened | minor |
| A rule removed or renamed; a default mode made stricter; the core range narrowed | major |

Pack versions are managed with [changesets](https://github.com/changesets/changesets): a pull request that changes a
pack adds a changeset (`npx changeset`) naming the pack's package and its bump, with a one-line summary for users.
A change counts when it touches what the pack publishes: its `package.json` (except `publishConfig` alone, which only
steers `npm publish`) or a path its `files` lists (`pack.json`, `rules/`, `licenses/`, `LICENSE`, `NOTICE`,
`README.md`); expected findings, candidates and `CHANGELOG.md` do not. The
**Changesets** job of `release-packs.yml` fails a pull request that changes what a pack publishes with no changeset
of its own naming that pack (`missingChangesets` in `scripts/release/packs.mjs`); the Release pull request is exempt.
`npm test` refuses a pending changeset that gives a pack below 1.0.0 a `minor` or `major` bump
(`changesetBumpFailures` in `scripts/release/packs.mjs`), which `changeset version` would turn into 0.2.0 or 1.0.0.
Nobody edits a pack's `version` or `CHANGELOG.md` by hand: `npx changeset version` applies the pending changesets,
bumps each pack and writes its `CHANGELOG.md` when packs are released. Changesets cover only the packs: the core
package at the repository root is bumped by the same Release pull request when what it ships changes, with its notes
in the root `CHANGELOG.md` ([release](release.md#release)), and a changeset naming `@neralabs/rerail` is refused. A pack's first pull request adds it at `0.1.0` with a `patch` changeset for its initial
rules, so its first release is `0.1.1`.

### Release

`.github/workflows/release-pr.yml` versions the packs, with core, and `.github/workflows/release-packs.yml` releases
them; core's `v*` release (`release.yml`) is separate.

1. **Version.** On each push to `main` with anything releasable, `release-pr.yml` opens or updates the **Release**
   pull request (opened by the `neralabs-ci` app, so CI runs on it). With changesets pending it runs
   `npm run version:packs`: `changeset version`, then `scripts/release/sync-pack-lockfile.mjs`, which records the new
   versions in `package-lock.json`. The same pull request bumps core when what core ships has changed. Its
   description lists every package and version merging it releases, with their changelog entries. Review it like any
   pull request; merging it is the release. A newer push to `main` cancels an older run, and with nothing releasable
   an open Release pull request is closed. CI runs only its licence and lint jobs on it, since `release-packs.yml`
   validates and packs every pack on it and `main` runs everything once it is merged.
2. **Publish.** On each push to `main`, every pack whose `package.json` version has no git tag
   `@neralabs/rerail-pack-<id>@<version>` goes through the publish gate and is published:
   - the whole pack validation for that pack (`test/engine/catalog/packs.test.ts`, every check and every rule), so a
     check added to the contract gates publishing with no change to the workflow;
   - `npm pack`, then a check that the tarball holds only `package.json` and what `files` publishes, and no
     validation material whatever `files` says (`packedFileFailures` in `conformance/pack-validation.ts`);
   - `npm publish --access public --tag latest` with npm trusted publishing (the `release` environment, so a
     reviewer approves the run), a check that an anonymous reader gets the version, and the tag.
     Set-up and `rerail packs add <id>` fetch under `latest` (on the default update channel), so the version is used
     from then on. A pack held at an older `latest` (step 3) is published under `next` instead, and its hold stays.

   Packs are published one by one, and any failed run can be run again: a version already tagged is skipped, and
   one the registry already has without its tag (a run that failed after publishing) is checked and tagged, not
   published again. The run's **Published dist-tags** job then lists each published pack's dist-tags in the run
   summary and fails if one's `latest` (`next` for a held pack) is not the version just published. A pull request
   that touches packs runs the gate and the packing as a dry run.
3. **Put back.** To put an older released version back, run the workflow from `main` (Actions → Release packs → Run
   workflow) with the pack's id and that version. Its first job lists, in the run summary, what it will move, and its
   promote job, once a reviewer approves the `release` environment, points the pack's `latest` at that version and
   tags it `held` there. Publishes and runs with no inputs leave a held pack's `latest` alone (the summary names it)
   until the pack is promoted by id again, at its `package.json` version, which releases the hold. Run with no inputs,
   the workflow points `latest` at every pack's released version that is not `latest` yet, less any held. A pack that
   fails to move, or whose dist-tags cannot be read, does not stop the others; the run fails naming it and can be run
   again. A version that was never released fails the run. These runs go one at a time.

The scripts are in `scripts/release/`. To check a pack's tarball locally:
`node --import tsx scripts/release/packs-pack.ts --out /tmp/rule-packs <id>`.

### Publishing a new pack

npm only lets a trusted publisher be configured for a package that exists, and a new configuration expires unless a
publish uses it within 2 days. So a pack's first version is published by hand, and the workflow publishes the next.
Until then each release run reports the pack as needing its first publish and fails after publishing the others. The
steps are in [publishing-a-new-pack.md](publishing-a-new-pack.md).

## Licensing

Rerail's own material in a pack is Apache-2.0. Material copied or adapted from an upstream project keeps its licence,
recorded per repository in `licenses/<project>/UPSTREAM.md`. The pack's `license` names every licence its published
files carry (for example `Apache-2.0 AND MIT` for rules adapted from an MIT tool; check 5). The pack's rule documents
(`rules/`, and the rule documents under `candidates/`) are annotated in `REUSE.toml` with that same expression,
`Apache-2.0 AND <upstream>`, so what `reuse lint` reports for them matches what the package publishes; whether adapted
rule documents need the upstream licence is an open legal question. Copied test data is not in git: its licences are
the `UPSTREAM.md` records and each archive's `LICENSES.md`. `REUSE.toml` annotates a tool's copied paths, pulled and
under `publish/`, for some tools (the rust and vue packs' entries), so that `reuse lint` passes on such a copy while it
is staged. Only licences on the repository allowlist (`scripts/check/licence-allowlist.json`) are accepted.

### UPSTREAM.md

A pack keeps two kinds of provenance record in git.

**`licenses/<project>/UPSTREAM.md`**, one per upstream repository, beside that repository's licence text (verbatim):

- a `License:` line: an SPDX expression (ids, `AND`, `OR`, `WITH` and parentheses), up to the note after it;
- a `Source` section whose table row gives the repository, a tag, the 40-hex commit and the licence;
- a `Taken from it` table: per row, backticked pack-path globs starting with `upstream/` in the first column, the
  upstream paths, and in the last column how that kind of file was made. A How cell's copy kind is the most modified
  of `verbatim`, `extracted`, `renamed` and `adapted` it names (`verbatim …; or adapted …` is `adapted`). A glob may
  hold at most two `**`; a `**` segment matches zero or more segments (`a/**/x` matches `a/x`);
- `Not copied` lines, one per upstream path nothing is copied from (an upstream's material derived from a copyleft
  project, such as Ruff's fixtures for its pylint ports), each exactly
  ``- Not copied: `<upstream path prefix>` as `<pack path glob>` — <project>, <SPDX id>``. The pack-path glob is
  required, and a list item that means to be such a line but is spelled otherwise (`* Not copied:`, `- Not Copied:`,
  an indented one, or one without the backticks) fails.

The record names patterns, never single files: the per-file record is in each archive
([below](#provenancejson-and-licensesmd)).

**`upstream/<tool>/UPSTREAM.md`**, the tool's own record, when Rerail wrote files for the tool to score: a table with
one row per file, its backticked pack path under `upstream/<tool>/` first (`` | `upstream/ruff/fixtures/rerail/noqa/B002.py` | written by Rerail | ``).
Those files are authored, Rerail's (Apache-2.0), and add no licence to `licenses/` or to the package's `license`.
Removing a row removes the file from the archive at the next publish, so a pull request that removes one is a publish
request ([docs/upstream.md](upstream.md#removing-data)). An `authored/` corpus needs no rows.

### provenance.json and LICENSES.md

Each test data archive holds `upstream/<tool>/provenance.json`, every file with its source, its upstream path and how
it was made, and `upstream/<tool>/LICENSES.md`, each source's licence and licence text. Both are derive output: the
generator writes them last, through `scripts/pack-data/provenance.mjs`, which refuses to write either when the files
disagree with the pack's `UPSTREAM.md` records ([docs/upstream.md](upstream.md#what-the-generators-write)). A
`snapshot` key appears only in archives bootstrapped by the one-off snapshot taken when the records moved from per-file
rows to patterns, until the tool-pack is derived again. They are for audit: no licence check reads them. The publish
workflow's prep reads only the key set of provenance.json's `files`, to refuse an archive whose files it does not
describe. A samples archive's `LICENSES.md` lists each source repository, its licence and the files taken from it
([docs/upstream.md](upstream.md#sample-licences)).

### Licence of each file

Pack check 6 and the publish workflow's prep decide each test data file's licence by the same rules, from the same
parsing and pattern choice (`readUpstreamSources`, `decidingPattern` and `inToolSubdirectory` in
`scripts/s3/pack-fixtures.mjs`). A file under a `Not copied` glob is refused, even one the tool's `UPSTREAM.md` lists
or one under `authored/`. Otherwise a listed file (or one under `authored/`) is Rerail's Apache-2.0, and any other file
takes the licence of the longest `Taken from it` glob that covers it; equal-length longest globs that disagree on
licence or copy kind are refused. A file in a subdirectory of `upstream/<tool>/` that none of these takes is refused;
one directly in `upstream/<tool>/` (expected findings, `cases.json`, `provenance.json`) is Rerail's. The publish adds
one rule: a file from an Apache-2.0-only source is refused unless that glob's kind is `verbatim`, since a modified
Apache-2.0 file must carry its change notices. A licence passes when some way to satisfy its SPDX expression uses only
allowlisted licences (`AND` binds tighter than `OR`); a malformed expression is refused. A sample's licence is its
repository's recorded one ([docs/upstream.md](upstream.md#sample-licences)).

## Moving to this layout

| Today | Becomes |
|---|---|
| Core's shipped complexity rules and their documents | `packs/complexity/pack.json` and `rules/`, published on their own; core ships no packs |
| `conformance/languages/<lang>/pack.json`, `rules/`, `divergences.json` | `packs/<lang>/conformance.json`, `rules/` or `candidates/`, `divergences.json`, plus a `pack.json` |
| `fixtures/<family>/rules/<tool>/` expected findings, copied tests and generators | `packs/<lang>/upstream/<tool>/`; for a language without rules yet, `fixtures/<family>/upstream/<tool>/` |
| `fixtures/<family>/{candidates,probes,policies}/` | `packs/<lang>/candidates/` |
| `fixtures/<family>/licenses/<project>/` for rule tools | `packs/<lang>/licenses/<project>/`, keeping only the rule tests' rows; rows for engine oracles stay in `fixtures/` |
| `conformance/languages/` runner and scorer | Test support in core that the pack tests call: `conformance/pack-validation.ts` |
| Everything else under `fixtures/` (parsing, scope, values, calls oracles) | Stays: engine tests, not pack validation |

Moved: `python`, `php`, `blade`, `typescript`, `react`, `node`, `next`, `sql` and `rust`, each from `conformance/languages/<id>/`,
`fixtures/<id>/rules/`, its candidates and probes, and the rule rows of its licence records. The ESLint expected-findings
helper they share is `scripts/pack-data/eslint-expected.mjs`, and the TypeScript grammar census is `conformance/t1/typescript-syntax.ts`.
The complexity rules moved to `packs/complexity/` and still ship inside core's package; their lizard
parity test stays in `test/engine/code-rule/complexity-rules.test.ts`. `vue` moved from `fixtures/vue/upstream/`, its
draft `rules.json` becoming `packs/vue/conformance.json`.
