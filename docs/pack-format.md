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
   tool's recorded findings with every disagreement declared.
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
│   └── <project>/      upstream LICENSE (verbatim) and UPSTREAM.md (every copied or adapted file)
│
│   only for packs that port rules from another tool:
├── conformance.json    the conformance pack schema of conformance/languages/pack.ts, as today
├── divergences.json    declared disagreements with the tools, as today
├── upstream/
│   └── <tool>/         the tool's copied tests (fixtures/), its recordings (*.expected.json), the recorder, and
│                       UPSTREAM.md when Rerail wrote files for the tool to score (an authored corpus)
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
  "publishConfig": { "access": "restricted" }
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
  publishes from. `publishConfig.access` is `restricted` until launch, like core.
- The schema is `rulePackPackageSchema` in `src/engine/catalog/rule-packs.ts`.

### pack.json

Today's `rerail.rule-pack/v1` ([format](rule-packs.md#format)): `id`, `name`, `category`, `covers`, `reason`,
`match`, `honourSuppressionComments`, and per rule `id`, `title`, `group`, optional `mode`, `on` and
`examples: { caught, passes }`. One change: a rule now names its document, `rules/<id>.json`, so a pack is
self-contained. A pack may ship no rules (`"rules": []`) while all its ports are candidates; such a pack is never
recommended. `document` is required (`rulePackFileSchema` in `src/engine/catalog/rule-packs.ts`); core reads each rule's document
from the pack:

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
recordings from `upstream/` unless `conformance.json` sets `oracles`:

- `conformance.json`: the language family, the engine capabilities the pack declares (with exemptions and their
  evidence), the pinned tools and public corpora, and for each ported rule the upstream tool and rule it reproduces,
  how findings compare (`line` or `line+column`) and the capabilities it exercises. `suppressionExemptions` lists the
  ports, by `id`, whose documents need not carry their tool's suppression comment yet, each with its `reason` (check
  4). A port whose document is under `candidates/` is scored and reported but never fails a check; it is scored on the
  fixtures, whose recording must cover it, and on a corpus only where that corpus's recording covers it. A port may
  set `fixtures`, `path.matchesGlob` patterns over the tool's fixture paths: it is then evaluated and scored only on
  the fixtures they match, the tests its upstream configuration ran on. That is how a tool whose tests run each rule
  with several option sets is scored: one document per option set, each on its own tests.
- `upstream/<tool>/fixtures.expected.json` and `corpus-<name>.expected.json`: what the tool reported on its own tests
  and on each pinned corpus, produced by the recorder in the same directory. Recorders are run by hand against the
  pinned tool versions; tools and corpus checkouts live under `.rerail/{tools,corpora}/<id>/` (gitignored). Where a
  tool's tests state their expected findings (as markup in the test files), the recorder reads them instead of running the
  tool: the tool's own test suite requires it to report exactly those.
- `upstream/<tool>/protected-identifiers.json` (optional, ECMA family): opts the pack into the identifier-rename gate,
  `test/conformance/identifier-rename.test.ts`, which scores every shipped port again on the fixtures with each
  user-chosen identifier renamed and fails when a score moves. It maps fixture paths or directories to the names and
  patterns the rename must keep (names the rule's implementation or the test's options mention).
- `divergences.json`: each place Rerail and the tool disagree, with its kind (`upstream-wrong`, `not-comparable`,
  `rerail-unknown`) and the evidence checked.

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

The message is advice to the agent: what is wrong, why it matters, what to change and when it is fine to leave it. A
message that states a fact or a ban gives the agent nothing to act on, and one the agent follows without judgement can
break working code.

1. It gives advice: it says what to change in this code and names the replacement. A fact or a ban alone ("Use of the
   insecure ECB cipher mode.", "Do not use bare except") is not advice.
2. Where following it can be wrong, because the matcher can't see the intent or the fix changes behaviour, it says when
   to leave the code alone or what the fix trades off ("…`strict=False` if truncation is intended").
3. It is calibrated: no hedging ("Consider maybe…", "may not be"), no shouting (NEVER, MUST, ALWAYS), and no alarm the
   finding doesn't warrant.
4. It is right for every match of its clause and agrees with the clause's `fix` when it has one. It uses the values the
   clause captures where they help (`$NAME`, `$CCN`, thresholds), never a binding the clause doesn't define.
5. It leaves out what the block adds: no rule id, location, "Rerail" or "fix it or explain". It names no upstream tool,
   rule code ("B113", "PIE810") or version, and no implementation text (AST node kinds, regexes).
6. At most 200 characters, counting placeholders as written. It may open with what was found and run to more than one
   sentence.
7. Where a rule's clauses match different things, each clause has its own message. One message for `x == None` and
   `x != None` that says "use `is None`" gives the opposite advice for `!=`.

For example: `This exception is silently discarded with pass. Handle or log it, or catch a specific expected exception when
suppression is intentional.`, or `` `$NAME` has cyclomatic complexity $CCN, over the ceiling of 6. Decompose it into smaller
functions. ``

These criteria are for reviewers and never block a merge: no check reads a message's text.

## The validation contract

Implemented: `validatePack(directory, { corpora?, corpusRoot?, listed? })` in `conformance/pack-validation.ts` runs
checks 1 to 8 over one pack and returns each check's failures and each rule's status (`ok`, `failed` or `not-supported-yet`).
`test/engine/catalog/packs.test.ts` calls it for every directory under `packs/`, with one test per check and one per rule, and has
the validator's own tests over the fixture packs in `fixtures/packs/`. `npm test` runs it with the local core.
Running it against the newest published core inside each pack's range is for pack CI, which does not exist yet.

The contract is the floor every pack meets. A ported rule must also pass the [port readiness gates](port-readiness.md) to ship:
they compare it with its upstream on start and end positions, on real code, and for overfitting and size; they block
CI for shipped rules and only report candidates.

```sh
npx vitest run test/engine/catalog/packs.test.ts                         # every pack: checks 1 to 8, each rule's status
npx vitest run test/engine/catalog/packs.test.ts -t "pack python"        # one pack
RERAIL_PACK_CORPORA=1 npx vitest run test/engine/catalog/packs.test.ts   # check 4 on every pinned corpus too
```

Check 4 always scores ported rules on the tools' fixtures. It scores a pinned corpus when its checkout is present
under `.rerail/corpora/<id>/<corpus>` (the nearest ancestor checkout's); `RERAIL_PACK_CORPORA=1` scores every corpus
and fails on a missing checkout, and `RERAIL_PACK_CORPORA=0` scores none. To see every false positive and false
negative of one ported rule: `node --import tsx conformance/languages/cli.ts --pack <id> --rule <rule> --fixtures`.

| # | Check | Fails when |
|---|---|---|
| 1 | Rules | A file in `rules/` is not listed in `pack.json`, a listed `document` is missing, is not named for its rule's id, its `id` differs from the listing, or it does not compile against core's capabilities; a rule id breaks the [id rule](#packjson); a candidate is not `rerail.policy/v1`, not in a `rules/` directory or not named for its id; a port's `id` differs from its document's; a `conformance.json` under `candidates/` does not parse or names a missing document |
| 2 | Proof | A rule has neither `examples` nor a port entry in `conformance.json` |
| 3 | Examples | A `caught` example produces no violation of its rule, or a `passes` example produces one |
| 4 | Ports | With `conformance.json`: a shipped ported rule's violations and the tool's recordings differ outside `divergences.json`; a divergence matches nothing (stale); a declared capability has neither a ported rule nor an exemption; a port's violation does not carry the suppression comment its tool honours, outside `suppressionExemptions`, or an exemption is stale; a port carries `suppress.oxlint` and `suppress.eslint` on different violations; a violation carries `suppress.eslint` without the Biome rules the same as its ESLint rule under `suppress.biome`, or `suppress.biome` without `suppress.eslint` |
| 5 | Package | `package.json` misnames the pack, names another repository or directory, is not `restricted`, has `dependencies`, or publishes validation material; its licence disagrees with what `licenses/` records |
| 6 | Licences | A copied file is missing from an `UPSTREAM.md`, or is copied from an upstream path its `UPSTREAM.md` lists as not copied; `reuse lint` or the licence allowlist fails |
| 7 | Manifest | `pack.json` has no `match`, or its `match` fails the schema; core's generated pack list does not name the pack or is stale |

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
   negative on the fixtures only, as do engine failures, recording errors and stale divergences or exemptions.
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
   `{ "rule": <port id>, "reason": … }`: one whose recording ran with the comments ignored (Bandit's `--ignore-nosec`),
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
   repository url>, "directory": "packs/<id>" }`, and `publishConfig.access` is `restricted`. `files` must publish
   `pack.json`, `rules`, `LICENSE` and, when present, `licenses`, and no entry may cover `conformance.json`,
   `divergences.json`, `upstream` or `candidates`. The SPDX ids `license` names must be exactly Apache-2.0 plus the
   licence each `licenses/<project>/UPSTREAM.md` records on its `License: <SPDX id>` line.
6. Each `licenses/<project>/` holds the upstream licence text and an `UPSTREAM.md` whose table rows start with a
   pack-relative path in backticks. Every file in a subdirectory of `upstream/<tool>/` (the tool's copied tests:
   `fixtures/`, `tests/`) must be listed; files directly in `upstream/` or `upstream/<tool>/` (recordings, recorders)
   are Rerail's. A file Rerail wrote for the tool to score (an authored corpus, such as `packs/complexity/`'s lizard
   corpus) is listed instead in the tool's own `upstream/<tool>/UPSTREAM.md`, with the same table rows: it is Rerail's,
   so it adds no licence to `licenses/` or to the package's `license`. That record may list only files under its own
   `upstream/<tool>/`. Every listed file must exist, every recorded licence must be on the repository allowlist
   (`scripts/check/licence-allowlist.json`), and the pack needs a `LICENSE`. An `UPSTREAM.md` may declare upstream
   paths nothing is copied from, one per line as ``- Not copied: `<upstream path prefix>` — <project>, <SPDX id>``
   (an upstream's material derived from a copyleft project, such as Ruff's fixtures for its pylint ports); a row whose
   upstream path (its second column) starts with one fails. `reuse lint` and `node scripts/check/check-licenses.mjs`
   stay repository-wide gates in CI; annotate copied files in `REUSE.toml` as well, since `reuse lint` alone accepts
   a copied file the catch-all Apache-2.0 annotation covers. Structure metric 7 (`npm run check:structure`) fails a
   listed file whose REUSE licence lacks the one its `UPSTREAM.md` records, or whose REUSE copyright names only Rerail.
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
| A false positive or false negative fixed; wording, examples or recordings changed | patch |
| A rule added; the core range widened | minor |
| A rule removed or renamed; a default mode made stricter; the core range narrowed | major |

Pack versions are managed with [changesets](https://github.com/changesets/changesets): a pull request that changes a
pack adds a changeset (`npx changeset`) naming the pack's package and its bump, with a one-line summary for users.
`npm test` refuses a pending changeset that gives a pack below 1.0.0 a `minor` or `major` bump
(`changesetBumpFailures` in `scripts/release/packs.mjs`), which `changeset version` would turn into 0.2.0 or 1.0.0.
Nobody edits a pack's `version` or `CHANGELOG.md` by hand: `npx changeset version` applies the pending changesets,
bumps each pack and writes its `CHANGELOG.md` when packs are released. Changesets cover only the packs: the core
package at the repository root keeps its `v<version>` tag release and the root `CHANGELOG.md`, and a changeset naming
`@neralabs/rerail` is refused. A pack's first pull request adds it at `0.1.0` with a `patch` changeset for its initial
rules, so its first release is `0.1.1`.

### Release

`.github/workflows/version-packs.yml` versions the packs and `.github/workflows/release-packs.yml` releases them;
core's `v*` release (`release.yml`) is separate.

1. **Version.** On each push to `main` with changesets pending, `version-packs.yml` opens or updates the **Version
   packs** pull request (opened by the `neralabs-ci` app, so CI runs on it). It runs `npm run version:packs`:
   `changeset version`, then `scripts/release/sync-pack-lockfile.mjs`, which records the new versions in
   `package-lock.json`. Review it like any pull request; merging it is the release. A newer push to `main` cancels an
   older run. CI runs only its licence and lint jobs on it, since `release-packs.yml` validates and packs every pack
   on it and `main` runs everything once it is merged.
2. **Publish.** On each push to `main`, every pack whose `package.json` version has no git tag
   `@neralabs/rerail-pack-<id>@<version>` goes through the publish gate and is published:
   - the whole pack validation for that pack (`test/engine/catalog/packs.test.ts`, every check and every rule), so a
     check added to the contract gates publishing with no change to the workflow;
   - `npm pack`, then a check that the tarball holds only `package.json` and what `files` publishes, and no
     validation material whatever `files` says (`packedFileFailures` in `conformance/pack-validation.ts`);
   - `npm publish --access restricted --tag next` with npm trusted publishing (the `release` environment, so a
     reviewer approves the run), a check that the registry refuses the package to an anonymous reader, and the tag.

   Packs are published one by one, and any failed run can be run again: a version already tagged is skipped, and
   one the registry already has without its tag (a run that failed after publishing) is checked and tagged, not
   published again. A pull request that touches packs runs the gate and the packing as a dry run.
3. **Promote.** Publishing changes nothing for users: set-up and `rerail packs add <id>` fetch under `latest` (on
   the default update channel), so a version is used once it is promoted. Run the workflow from `main` (Actions →
   Release packs → Run workflow) with no inputs: its first job lists, in the run summary, every pack whose
   `package.json` version is released and is not `latest` yet, and its promote job, once a reviewer approves the
   `release` environment, points each one's `latest` at that version. A pack that fails to promote, or whose
   dist-tags cannot be read, does not stop the others; the run fails naming it and can be run again. Promotions run
   one at a time.

   To promote one pack alone, give its id. To put an older released version back, give the id and that version: the
   pack is also tagged `held` there, and runs with no inputs leave it alone (the summary names it) until the pack is
   promoted by id again, which releases the hold. A version that was never released fails the run. By hand, from a
   2FA npm login: `npm dist-tag add @neralabs/rerail-pack-<id>@<version> latest` (and `held` to hold it). To try a
   version before promoting it: `rerail packs add <id>@next`.

The scripts are in `scripts/release/`. To check a pack's tarball locally:
`node --import tsx scripts/release/packs-pack.ts --out /tmp/rule-packs <id>`.

### Publishing a new pack

npm only lets a trusted publisher be configured for a package that exists, and a new configuration expires unless a
publish uses it within 2 days. So a pack's first version is published by hand, and the workflow publishes the next.
Until then each release run reports the pack as needing its first publish and fails after publishing the others. The
steps are in [publishing-a-new-pack.md](publishing-a-new-pack.md).

## Licensing

Rerail's own material in a pack is Apache-2.0. Material copied or adapted from an upstream project keeps its licence:
listed in `licenses/<project>/UPSTREAM.md` and annotated in the repository's `REUSE.toml`. The pack's `license`
names every licence its published files carry (for example `Apache-2.0 AND MIT` for rules adapted from an MIT tool).
The pack's rule documents (`rules/`, and the rule documents under `candidates/`) are annotated in `REUSE.toml` with
that same expression, `Apache-2.0 AND <upstream>`, so what `reuse lint` reports for them matches what the package
publishes; whether adapted rule documents need the upstream licence is an open legal question. Only licences on the
repository allowlist are accepted.

## Moving to this layout

| Today | Becomes |
|---|---|
| Core's shipped complexity rules and their documents | `packs/complexity/pack.json` and `rules/`, published on their own; core ships no packs |
| `conformance/languages/<lang>/pack.json`, `rules/`, `divergences.json` | `packs/<lang>/conformance.json`, `rules/` or `candidates/`, `divergences.json`, plus a `pack.json` |
| `fixtures/<family>/rules/<tool>/` recordings, copied tests and recorders | `packs/<lang>/upstream/<tool>/`; for a language without rules yet, `fixtures/<family>/upstream/<tool>/` |
| `fixtures/<family>/{candidates,probes,policies}/` | `packs/<lang>/candidates/` |
| `fixtures/<family>/licenses/<project>/` for rule tools | `packs/<lang>/licenses/<project>/`, keeping only the rule tests' rows; rows for engine oracles stay in `fixtures/` |
| `conformance/languages/` runner and scorer | Test support in core that the pack tests call: `conformance/pack-validation.ts` |
| Everything else under `fixtures/` (parsing, scope, values, calls oracles) | Stays: engine tests, not pack validation |

Moved: `python`, `php`, `blade`, `typescript`, `react`, `node`, `next`, `sql` and `rust`, each from `conformance/languages/<id>/`,
`fixtures/<id>/rules/`, its candidates and probes, and the rule rows of its licence records. The ESLint recorder they
share is `conformance/recorders/eslint-record.mjs`, and the TypeScript grammar census is `conformance/t1/typescript-syntax.ts`.
The complexity rules moved to `packs/complexity/` and still ship inside core's package; their lizard
parity test stays in `test/engine/code-rule/complexity-rules.test.ts`. `vue` moved from `fixtures/vue/upstream/`, its
draft `rules.json` becoming `packs/vue/conformance.json`.
