# Rule pack structure and validation contract

Status: the layout and the validation contract (checks 1 to 6) are implemented. `test/packs.test.ts` validates every
`packs/*` directory: `python`, `php`, `blade`, `typescript`, `react`, `node`, `next` and `tslint`. Core does not load
installed packs yet: it ships the TSLint pack inside its own package (`packs/tslint/pack.json` and `packs/tslint/rules/`
in its `files`, read from `CORE_RULE_PACKS` in `src/rule-bootstrap.ts`; see [rule packs](rule-packs.md)), and no pack
is published. A pack introduces no new file format: it combines the two schemas the repository already has. To start
one, see [language packs](language-packs.md#start-a-pack).

Core's own built-in rules are not a pack: they would live in `code-rule-dsl/rules/` beside the engine's
`capabilities.json` and be seeded enabled. None ship today. Packs hold every other rule document; their rules are
seeded disabled.

## Principles

1. **A pack is data.** Rule documents plus the evidence that they behave. It contains no code and never imports the
   engine: core runs its rules, and the repository's tests validate it.
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
│   └── <name>.json     code-rule documents (rerail.policy/v1)
├── licenses/
│   └── <project>/      upstream LICENSE (verbatim) and UPSTREAM.md (every copied or adapted file)
│
│   only for packs that port rules from another tool:
├── conformance.json    the conformance pack schema of conformance/languages/pack.ts, as today
├── divergences.json    declared disagreements with the tools, as today
├── upstream/
│   └── <tool>/         the tool's copied tests (fixtures/), its recordings (*.expected.json), the recorder
└── candidates/         ports still being evaluated: scored, never shipped
```

`<id>` is lowercase letters, digits and dashes. It is the directory name, the `id` in `pack.json`, and the npm name
`@neralabs/rerail-pack-<id>`.

### package.json

```json
{
  "name": "@neralabs/rerail-pack-tslint",
  "version": "0.1.0",
  "license": "Apache-2.0",
  "files": ["pack.json", "rules", "licenses", "LICENSE", "NOTICE", "README.md"],
  "peerDependencies": { "@neralabs/rerail": ">=0.1.0-0 <0.2.0" },
  "peerDependenciesMeta": { "@neralabs/rerail": { "optional": true } },
  "publishConfig": { "access": "restricted" }
}
```

- The core version range lives in `peerDependencies`, so npm warns when the installed core does not fit. It is a
  declaration, not an import: a pack has no `dependencies`. The `-0` admits core's prereleases (`0.1.0-next.0`).
- The peer is `optional`: `packs/*` are workspaces of this repository, whose root package is core itself, and npm
  would otherwise try to install a published core into them. npm still checks the range when core is installed.
- `files` publishes the pack and its licence records. `conformance.json`, `divergences.json`, `upstream/` and
  `candidates/` are validation material: they stay in the repository and never reach npm, which keeps copied
  upstream tests out of the package.
- The schema is `rulePackPackageSchema` in `src/rule-packs.ts`.

### pack.json

Today's `rerail.rule-pack/v1` ([format](rule-packs.md#format)): `id`, `name`, `category`, `covers`, `reason`,
`match`, `honourSuppressionComments`, and per rule `id`, `title`, `group`, optional `mode`, `on` and
`examples: { caught, passes }`. One change: a rule now names its document, `rules/<name>.json`, so a pack is
self-contained. `document` is required (`rulePackFileSchema` in `src/rule-packs.ts`); core reads each rule's document
from the pack:

```json
{ "id": "tslint:triple-equals", "document": "rules/triple-equals.json", "title": "Requires === and !==", "group": "TSLint" }
```

An example's `path` is required in a pack: it decides the file's language and must be inside the rule's scope.

### conformance.json, divergences.json, upstream/

Today's schemas from `conformance/languages/pack.ts`, moved next to the rules they validate. The runner reads a pack's
recordings from `upstream/` unless `conformance.json` sets `oracles`:

- `conformance.json`: the language family, the engine capabilities the pack declares (with exemptions and their
  evidence), the pinned tools and public corpora, and for each ported rule the upstream tool and rule it reproduces,
  how findings compare (`line` or `line+column`) and the capabilities it exercises.
- `upstream/<tool>/fixtures.expected.json` and `corpus-<name>.expected.json`: what the tool reported on its own tests
  and on each pinned corpus, produced by the recorder in the same directory. Recorders are run by hand against the
  pinned tool versions; tools and corpus checkouts live under `.rerail/{tools,corpora}/<id>/` (gitignored).
- `divergences.json`: each place Rerail and the tool disagree, with its kind (`upstream-wrong`, `not-comparable`,
  `rerail-unknown`) and the evidence checked.

**TSLint** is scored by its own harness, `conformance/tslint/` ([README](../conformance/tslint/README.md)), because
TSLint's tests are `.lint` files with option variants that the harness fetches at a pinned commit instead of
recordings. Its files carry a `format` that names them: `conformance.json` is the golden manifest
(`rerail-tslint-golden/v1`: per shipped rule, the verdict and true positives the golden gate holds it to), and
`divergences.json` is the PASS audit (`rerail-tslint-audit/v1`: per port that passes, whether it is genuine or where
it differs from TSLint on real code). `candidates/` holds the ports not shipped, their verdict sidecars and every
rule's option-variant overrides (`candidates/<rule>/<variant>.json`). Check 2 counts a golden manifest entry as a
port entry; check 4 checks that the manifest lists exactly the pack's rules, and the golden gate
(`conformance/tslint/golden-gate.test.ts`, run by `npm run test:conformance`) scores them against TSLint's tests.

## The validation contract

Implemented: `validatePack(directory, { corpora?, corpusRoot? })` in `conformance/pack-validation.ts` runs checks 1
to 6 over one pack and returns each check's failures and each rule's status (`ok`, `failed` or `not-supported-yet`).
`test/packs.test.ts` calls it for every directory under `packs/`, with one test per check and one per rule, and has
the validator's own tests over the fixture packs in `test/fixtures/packs/`. `npm test` runs it with the local core.
Running it against the newest published core inside each pack's range is for pack CI, which does not exist yet.

```sh
npx vitest run test/packs.test.ts                         # every pack: checks 1 to 6, each rule's status
npx vitest run test/packs.test.ts -t "pack python"        # one pack
RERAIL_PACK_CORPORA=1 npx vitest run test/packs.test.ts   # check 4 on every pinned corpus too
```

Check 4 always scores ported rules on the tools' fixtures. It scores a pinned corpus when its checkout is present
under `.rerail/corpora/<id>/<corpus>` (the nearest ancestor checkout's); `RERAIL_PACK_CORPORA=1` scores every corpus
and fails on a missing checkout, and `RERAIL_PACK_CORPORA=0` scores none. To see every false positive and false
negative of one ported rule: `node --import tsx conformance/languages/cli.ts --pack <id> --rule <rule> --fixtures`.

| # | Check | Fails when |
|---|---|---|
| 1 | Rules | A file in `rules/` is not listed in `pack.json`, a listed `document` is missing, its `id` differs from the listing, or it does not compile against core's capabilities |
| 2 | Proof | A rule has neither `examples` nor a port entry in `conformance.json` |
| 3 | Examples | A `caught` example produces no violation of its rule, or a `passes` example produces one |
| 4 | Ports | With `conformance.json`: a ported rule's violations and the tool's recordings differ outside `divergences.json`; a divergence matches nothing (stale); a declared capability has neither a ported rule nor an exemption |
| 5 | Package | `package.json` misnames the pack, has `dependencies`, or publishes validation material; its licence disagrees with what `licenses/` records |
| 6 | Licences | A copied file is missing from an `UPSTREAM.md`; `reuse lint` or the licence allowlist fails |

Precisely:

1. `pack.json` must parse (`rulePackFileSchema`) and its `id` must be the directory name. Each document must
   be a valid code rule (`rerail.policy/v1`, the host's schema) and compile in the engine with core's
   `code-rule-dsl/capabilities.json`, as the catalog compiles it.
2. A port entry is a `conformance.json` rule whose `policy` is the rule's `document`.
3. Each example is evaluated alone, as a one-file repository at its `path`, through the engine (the `bundled-rules`
   test's path). Its path must be in the rule's scope, and it must produce no engine failure. A violation counts when
   its code is one of the rule's clauses' codes.
4. The fixture tier of the conformance runner (`conformance/languages/runner.ts`, the `language-conformance` test's),
   over the production workspace path: an unexplained false positive fails on every source, an unexplained false
   negative on the fixtures only, as do engine failures, recording errors and stale divergences or exemptions.
   Ports in `conformance.json` that `pack.json` does not list (candidates) are scored too.
5. The npm name is `@neralabs/rerail-pack-<id>`. `files` must publish `pack.json`, `rules`, `LICENSE` and, when
   present, `licenses`, and no entry may cover `conformance.json`, `divergences.json`, `upstream` or `candidates`. The
   SPDX ids `license` names must be exactly Apache-2.0 plus the licence each `licenses/<project>/UPSTREAM.md` records
   on its `License: <SPDX id>` line.
6. Each `licenses/<project>/` holds the upstream licence text and an `UPSTREAM.md` whose table rows start with a
   pack-relative path in backticks. Every file in a subdirectory of `upstream/<tool>/` (the tool's copied tests:
   `fixtures/`, `tests/`) must be listed; files directly in `upstream/` or `upstream/<tool>/` (recordings, recorders)
   are Rerail's. Every listed file must exist, every recorded licence must be on the repository allowlist
   (`scripts/licence-allowlist.json`), and the pack needs a `LICENSE`. `reuse lint` and `node scripts/check-licenses.mjs`
   stay repository-wide gates in CI; annotate copied files in `REUSE.toml` as well, since `reuse lint` alone accepts
   a copied file the catch-all Apache-2.0 annotation covers.

**Languages not supported yet.** A pack may hold code rules for a language core's
[language registry](language-packs.md) lists as `planned`, or rules that read a capability their language has
`not-supported-yet`. Such a rule fails to compile with the code `language-not-supported`. The tests report it as
`not supported yet` (its rule test is skipped with the engine's reason) and do not fail: check 1 passes it, and checks
3 and 4 skip it until core reads the language. Any other compile error still fails check 1. So a pack for a planned
language can land before the engine supports it.

A public `rerail pack check` command, for packs written outside this repository, can wrap the same checks later; it
is not part of this contract.

## Versioning and release

| Change | Version bump |
|---|---|
| A false positive or false negative fixed; wording, examples or recordings changed | patch |
| A rule added; the core range widened | minor |
| A rule removed or renamed; a default mode made stricter; the core range narrowed | major |

Pack versions are managed with [changesets](https://github.com/changesets/changesets): a pull request that changes a
pack adds a changeset (`npx changeset`) naming the pack's package and the bump from the table above, with a one-line
summary for users. Nobody edits a pack's `version` or `CHANGELOG.md` by hand: `npx changeset version` applies the
pending changesets, bumps each pack and writes its `CHANGELOG.md` when packs are released. Changesets cover only the
packs: the core package at the repository root keeps its `v<version>` tag release and the root `CHANGELOG.md`, and a
changeset naming `@neralabs/rerail` is refused. A pack's first pull request adds it at `0.1.0` with a `minor` changeset
for its initial rules.

Each pack is released on its own: its package publishes `packs/<id>/` through the release workflow and trusted
publishing, under `next` first and `latest` once promoted, `restricted` until launch like core. Publishing from the
release workflow is not wired yet; until it is, changesets accumulate and nothing is published.

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
| `policies/rule-packs/tslint.json`, `code-rule-dsl/rules/opt-in/tslint/` (shipped in core) | `packs/tslint/pack.json` and `rules/`, still shipped in core's package until core loads installed packs |
| `conformance/tslint/rules/**` (option variants, ports not shipped) | `packs/tslint/candidates/` |
| `conformance/tslint/golden.json` and the audit in `conformance/tslint/audit.ts` | `packs/tslint/conformance.json` (golden manifest) and `divergences.json` (PASS audit) |
| TSLint's upstream tests, fetched at a pinned commit by `conformance/tslint/fixtures.ts` | Not moved: still fetched into `tmp/tslint-fixtures/`; a recording under `packs/tslint/upstream/tslint/` would let the language runner score TSLint |
| `conformance/languages/<lang>/pack.json`, `rules/`, `divergences.json` | `packs/<lang>/conformance.json`, `rules/` or `candidates/`, `divergences.json`, plus a `pack.json` |
| `fixtures/<family>/rules/<tool>/` recordings, copied tests and recorders | `packs/<lang>/upstream/<tool>/`; for a language without rules yet, `fixtures/<family>/upstream/<tool>/` |
| `fixtures/<family>/{candidates,probes,policies}/` | `packs/<lang>/candidates/` |
| `fixtures/<family>/licenses/<project>/` for rule tools | `packs/<lang>/licenses/<project>/`, keeping only the rule tests' rows; rows for engine oracles stay in `fixtures/` |
| `conformance/languages/` runner and scorer | Test support in core that the pack tests call: `conformance/pack-validation.ts` |
| Everything else under `fixtures/` (parsing, scope, values, calls oracles) | Stays: engine tests, not pack validation |

Moved: `python`, `php`, `blade`, `typescript`, `react`, `node` and `next`, each from `conformance/languages/<id>/`,
`fixtures/<id>/rules/`, its candidates and probes, and the rule rows of its licence records. The ESLint recorder they
share is `conformance/recorders/eslint-record.mjs`, and the TypeScript grammar census is `conformance/t1/typescript-syntax.ts`.
TSLint moved to `packs/tslint/` and still ships inside core's package; its harness stays in `conformance/tslint/` as
test support. The complexity rules (`code-rule-dsl/rules/opt-in/complexity/`, `policies/rule-packs/complexity.json`)
moved to `packs/complexity/` the same way; their lizard parity test stays in `test/complexity-rules.test.ts`. Not
moved yet: `rust`, `sql` and `vue`, which have upstream
recordings but no rules: they stay in `fixtures/<id>/upstream/` beside each language's draft manifest
(`fixtures/rust/upstream/conformance.draft.json`, `fixtures/sql/upstream/conformance.draft.json`,
`fixtures/vue/upstream/rules.json`) until their packs gain rules, when the draft becomes `packs/<id>/conformance.json`
and `upstream/` moves to `packs/<id>/upstream/`.
