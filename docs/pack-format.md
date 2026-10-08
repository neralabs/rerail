# Rule pack structure and validation contract

Status: draft. Packs still ship inside core today (`policies/rule-packs/tslint.json` and
`code-rule-dsl/rules/opt-in/tslint/`, see [rule packs](rule-packs.md)), and the conformance harness under
`conformance/` scores rule ports. This document defines where packs move to in this repository and what a pack must
contain to pass. It introduces no new file format: a pack combines the two schemas the repository already has.

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
  "peerDependencies": { "@neralabs/rerail": ">=0.1.0 <0.2.0" },
  "publishConfig": { "access": "restricted" }
}
```

- The core version range lives in `peerDependencies`, so npm warns when the installed core does not fit. It is a
  declaration, not an import: a pack has no `dependencies`.
- `files` publishes the pack and its licence records. `conformance.json`, `divergences.json`, `upstream/` and
  `candidates/` are validation material: they stay in the repository and never reach npm, which keeps copied
  upstream tests out of the package.

### pack.json

Today's `rerail.rule-pack/v1` ([format](rule-packs.md#format)): `id`, `name`, `category`, `covers`, `reason`,
`match`, `honourSuppressionComments`, and per rule `id`, `title`, `group`, optional `mode`, `on` and
`examples: { caught, passes }`. One change: a rule now names its document, so a pack is self-contained:

```json
{ "id": "tslint:triple-equals", "document": "rules/triple-equals.json", "title": "Requires === and !==", "group": "TSLint" }
```

### conformance.json, divergences.json, upstream/

Today's schemas from `conformance/languages/pack.ts`, moved next to the rules they validate:

- `conformance.json`: the language family, the engine capabilities the pack declares (with exemptions and their
  evidence), the pinned tools and public corpora, and for each ported rule the upstream tool and rule it reproduces,
  how findings compare (`line` or `line+column`) and the capabilities it exercises.
- `upstream/<tool>/fixtures.expected.json` and `corpus-<name>.expected.json`: what the tool reported on its own tests
  and on each pinned corpus, produced by the recorder in the same directory. Recorders are run by hand against the
  pinned tool versions; tools and corpus checkouts live under `.rerail/{tools,corpora}/<id>/` (gitignored).
- `divergences.json`: each place Rerail and the tool disagree, with its kind (`upstream-wrong`, `not-comparable`,
  `rerail-unknown`) and the evidence checked.

## The validation contract

The repository's tests check every `packs/*` directory. `npm test` runs them with the local core; pack CI also runs
them against the newest published core inside each pack's range.

| # | Check | Fails when |
|---|---|---|
| 1 | Rules | A file in `rules/` is not listed in `pack.json`, a listed `document` is missing, its `id` differs from the listing, or it does not compile against core's capabilities |
| 2 | Proof | A rule has neither `examples` nor a port entry in `conformance.json` |
| 3 | Examples | A `caught` example produces no violation of its rule, or a `passes` example produces one |
| 4 | Ports | With `conformance.json`: a ported rule's violations and the tool's recordings differ outside `divergences.json`; a divergence matches nothing (stale); a declared capability has neither a ported rule nor an exemption |
| 5 | Package | `package.json` misnames the pack, has `dependencies`, or publishes validation material; its licence disagrees with what `licenses/` records |
| 6 | Licences | A copied file is missing from an `UPSTREAM.md`; `reuse lint` or the licence allowlist fails |

Checks 1 to 4 are today's `bundled-rules` and `language-conformance` tests, pointed at `packs/*` instead of
`policies/` and `conformance/`. Check 6 is the repository's existing licence gate.

**Languages not supported yet.** A pack may hold code rules for a language core's
[language registry](language-packs.md) lists as `planned`, or rules that read a capability their language has
`not-supported-yet`. Such a rule fails to compile with the code `language-not-supported`. The tests report it as
`not supported yet` and do not fail: check 1 passes it, and checks 3 and 4 skip it until core reads the language. Any
other compile error still fails check 1. So a pack for a planned language can land before the engine supports it.

A public `rerail pack check` command, for packs written outside this repository, can wrap the same checks later; it
is not part of this contract.

## Versioning and release

| Change | Version bump |
|---|---|
| A false positive or false negative fixed; wording, examples or recordings changed | patch |
| A rule added; the core range widened | minor |
| A rule removed or renamed; a default mode made stricter; the core range narrowed | major |

Each pack is released on its own: a tag `pack-<id>-v<version>` publishes `packs/<id>/` through the release workflow
and trusted publishing, under `next` first and `latest` once promoted, `restricted` until launch like core.

## Licensing

Rerail's own material in a pack is Apache-2.0. Material copied or adapted from an upstream project keeps its licence:
listed in `licenses/<project>/UPSTREAM.md` and annotated in the repository's `REUSE.toml`. The pack's `license`
names every licence its published files carry (for example `Apache-2.0 AND MIT` for rules adapted from an MIT tool;
whether adapted rule documents need the upstream licence is an open legal question). Only licences on the repository
allowlist are accepted.

## Moving to this layout

| Today | Becomes |
|---|---|
| `policies/rule-packs/tslint.json`, `code-rule-dsl/rules/opt-in/tslint/` (shipped in core) | `packs/tslint/pack.json` and `rules/`, once core loads installed packs |
| `conformance/tslint/rules/**` (option variants, ports not shipped) | `packs/tslint/candidates/` |
| `conformance/tslint/` golden manifest and audit | The `tslint` pack's rule list and its `divergences.json` |
| TSLint's upstream tests, fetched at a pinned commit by `conformance/tslint/fixtures.ts` | A recording under `packs/tslint/upstream/tslint/` |
| `conformance/languages/<lang>/pack.json`, `rules/`, `divergences.json` | `packs/<lang>/conformance.json`, `rules/` or `candidates/`, `divergences.json`, plus a `pack.json` |
| `fixtures/<family>/rules/<tool>/` recordings, copied tests and recorders | `packs/<lang>/upstream/<tool>/` |
| `fixtures/<family>/{candidates,probes,policies}/` | `packs/<lang>/candidates/` |
| `fixtures/<family>/licenses/<project>/` for rule tools | `packs/<lang>/licenses/<project>/` |
| `conformance/languages/` runner and scorer | Test support in core that the pack tests call |
| Everything else under `fixtures/` (parsing, scope, values, calls oracles) | Stays: engine tests, not pack validation |
