# Rule pack format

Status: draft specification. Nothing here is implemented yet: today's packs use `rerail.rule-pack/v1`
([rule packs](rule-packs.md)), and the conformance harness under `conformance/` scores rule ports. This document
defines what replaces both: the `rerail.rule-pack/v2` package format and the `rerail pack check` command.

## Principles

1. **A pack is data.** Rule documents, a manifest, and the evidence that the rules behave. It contains no code and
   never imports Rerail: core reads it to run its rules and core validates it.
2. **A pack proves itself.** Every rule carries examples it must catch and let pass. A rule ported from another tool
   also carries that tool's recorded findings, and every disagreement with the tool is declared with evidence.
3. **One contract, two consumers.** The same manifest drives the dashboard (titles, groups, match, modes) and
   validation (`rerail pack check`).
4. **Packs version on their own.** A pack is its own npm package with its own version and declares which core
   versions it works with.

## Layout

```text
packs/<id>/
├── package.json             npm package metadata; no dependencies
├── rerail-pack.json         the manifest (format below)
├── README.md                what the pack covers, for people
├── LICENSE                  the pack's licence
├── NOTICE                   the pack's notice, with any upstream NOTICE text it must carry
├── rules/
│   └── <name>.json          one rule document per rule (`rerail.policy/v1` for file rules)
├── licenses/
│   └── <project>/
│       ├── LICENSE          the upstream licence, verbatim at the pinned version
│       └── UPSTREAM.md      every file copied or adapted from <project>: verbatim, derived or excerpt
└── validation/              never published
    ├── divergences.json     declared disagreements with upstream tools
    ├── candidates/          rule documents under evaluation; scored, reported, never shipped
    └── upstream/
        └── <tool>/
            ├── fixtures/                     the tool's own test inputs, copied at the pinned version
            ├── fixtures.expected.json        what the tool reported on them
            ├── corpus-<name>.expected.json   what the tool reported on a pinned public corpus
            └── record.<py|mjs>               the recorder that produced the recordings (run by hand)
```

`<id>` is lowercase letters, digits and dashes, and names the pack everywhere: the directory, the manifest `id`, and
the npm package `@neralabs/rerail-pack-<id>`.

What is published and what is not:

| Published (in `package.json` `files`) | Not published |
|---|---|
| `rerail-pack.json`, `rules/`, `licenses/`, `LICENSE`, `NOTICE`, `README.md` | `validation/` |

`validation/` stays in the repository: it holds third-party test material that users do not need to run the
rules, and keeping it out of the package keeps that material out of npm. A pack's examples live in its manifest and
are published, so an installed pack can still be checked at the example level.

## package.json

```json
{
  "name": "@neralabs/rerail-pack-python",
  "version": "0.1.0",
  "description": "Ruff and Bandit rules for Python, ported to Rerail",
  "license": "Apache-2.0 AND MIT",
  "files": ["rerail-pack.json", "rules", "licenses", "LICENSE", "NOTICE", "README.md"],
  "peerDependencies": { "@neralabs/rerail": ">=0.1.0 <0.2.0" },
  "rerail": { "pack": "./rerail-pack.json" },
  "publishConfig": { "access": "restricted" }
}
```

- `rerail.pack` marks the package as a pack and points at its manifest. Core discovers installed packs by this
  field, not by name, so packs from other scopes work the same way.
- `peerDependencies["@neralabs/rerail"]` mirrors the manifest's `coreRange`, so npm warns at install when the core
  version does not fit. It is a declaration, not an import: there are no `dependencies`.
- `license` must cover everything in `files` and equal the manifest's `license`.

## Manifest: `rerail-pack.json`

```json
{
  "format": "rerail.rule-pack/v2",
  "id": "python",
  "name": "Python (Ruff and Bandit)",
  "category": "language",
  "covers": "Ruff and Bandit rules ported to Rerail, scored against both tools on their own tests and on Django.",
  "reason": "matches Python",
  "coreRange": ">=0.1.0 <0.2.0",
  "languages": ["python"],
  "license": "Apache-2.0 AND MIT",
  "upstream": [
    { "project": "ruff", "version": "0.16.9", "license": "MIT", "url": "https://github.com/astral-sh/ruff" },
    { "project": "bandit", "version": "1.8.6", "license": "Apache-2.0", "url": "https://github.com/PyCQA/bandit" }
  ],
  "match": {
    "any": [{ "files": ["**/*.py", "**/*.pyi"], "label": "*.py files" }]
  },
  "honourSuppressionComments": true,
  "rules": [
    {
      "id": "ruff:f401",
      "document": "rules/ruff-f401-unused-import.json",
      "title": "Remove unused imports",
      "group": "Ruff",
      "mode": "steer",
      "on": true,
      "examples": {
        "caught": { "path": "app.py", "text": "import os\n" },
        "passes": { "path": "app.py", "text": "import os\nprint(os.sep)\n" }
      },
      "upstream": { "tool": "ruff", "rule": "F401", "match": "line+column", "capabilities": ["scope"] }
    }
  ],
  "validation": {
    "capabilities": ["syntax", "value", "scope", "calls", "suppression"],
    "exemptCapabilities": [],
    "tools": [
      { "name": "ruff", "version": "0.16.9" },
      { "name": "bandit", "version": "1.8.6" }
    ],
    "corpora": [
      { "name": "django", "repository": "https://github.com/django/django", "commit": "<sha>", "paths": ["django"], "exclude": [] }
    ]
  }
}
```

### Pack fields

| Field | Required | Meaning |
|---|---|---|
| `format` | yes | `rerail.rule-pack/v2` |
| `id` | yes | The pack id (see Layout) |
| `name` | yes | Display name |
| `category` | yes | `language`, `framework` or `repository`; orders and filters packs in the dashboard |
| `covers` | yes | One sentence: what the pack covers |
| `reason` | yes | Why it fits, in general terms; the reason shown for a repository is built from what matched |
| `coreRange` | yes | Semver range of core versions the pack works with |
| `languages` | yes | Code-rule languages its rules read; each must be one the running core supports |
| `license` | yes | SPDX expression for everything the package publishes |
| `upstream` | yes | Projects the pack copies or adapts from; each has `licenses/<project>/`. Empty when none |
| `match` | yes | When a repository fits, as in v1: `{ "any": [signal, …] }` with `files` or `npmDependencies` signals |
| `honourSuppressionComments` | no | Whether rules honour their linters' suppression comments; default `true` |
| `rules` | yes | The shipped rules, at least one |
| `validation` | no | Upstream evidence settings; required when any rule has `upstream` |

### Rule fields

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | `<namespace>:<name>`, lowercase; must equal the `id` inside the rule document; unique across all installed packs |
| `document` | yes | Path of the rule document, relative to the pack |
| `title`, `group` | yes | What the dashboard shows |
| `mode` | no | Mode the rule starts in when the pack is added; default: the document's mode |
| `on` | no | Whether the rule starts on; default `true` |
| `examples` | yes | `{ "caught": example, "passes": example }`; an example is `{ "path"?, "text" }` (`path` for file rules) |
| `upstream` | no | For a ported rule: `tool`, the upstream `rule` name, how findings compare (`match`: `line` or `line+column`), and the `capabilities` it exercises |

### Validation fields

| Field | Meaning |
|---|---|
| `capabilities` | Engine capabilities the pack relies on; each needs at least one ported rule exercising it, or an exemption |
| `exemptCapabilities` | Capabilities no upstream rule reads like-for-like: `capability`, `reason`, `evidence`, `coveredBy` |
| `tools` | Pinned upstream tools and versions; each must have `validation/upstream/<tool>/` |
| `corpora` | Pinned public corpora: `name`, `repository`, `commit`, `paths`, `exclude` |

## Validation data

**Recordings** (`validation/upstream/<tool>/*.expected.json`) are what the upstream tool reported, produced by the
tool's recorder and committed:

```json
{
  "tool": "ruff",
  "version": "0.16.9",
  "source": { "kind": "fixtures", "origin": "https://github.com/astral-sh/ruff@<sha>", "rules": ["F401"], "files": ["F401_0.py"] },
  "findings": [{ "rule": "F401", "path": "F401_0.py", "line": 2, "column": 8 }]
}
```

A `corpus` source names `name`, `repository`, `commit` and `rules`, and may list `skipped` files and `unanalysed`
line ranges the tool itself did not analyse; those are never scored. Paths are POSIX, relative to the fixtures
directory or the corpus checkout. Lines and columns are 1-based; columns count Unicode code points.

**Divergences** (`validation/divergences.json`) declare every place Rerail and the tool disagree:

```json
[
  {
    "rule": "ruff:f401",
    "source": "fixtures",
    "path": "F401_5.py",
    "line": 4,
    "kind": "upstream-wrong",
    "evidence": "Ruff 0.16.9 reports the re-export as unused; CPython imports it and __all__ lists it (see …)."
  }
]
```

`kind` is `upstream-wrong` (the tool is mistaken), `not-comparable` (the tool reads something Rerail by design does
not), or `rerail-unknown` (Rerail is wrong and the gap is accepted for now). `evidence` cites what was checked.

**Candidates** (`validation/candidates/*.json`) are rule documents still being ported. `pack check` scores them and
reports the results, but they never fail the check and are never published. A candidate becomes a rule by moving it
to `rules/` and adding it to the manifest, with examples.

**Recorders** run the pinned upstream tool and write recordings. They are authoring tools, run by hand, never by
CI or the product; their header names the tool versions and how to obtain them. Pinned tools and corpus checkouts
live under `.rerail/{tools,corpora}/<pack>/` in the repository (gitignored), or are passed as arguments.

## `rerail pack check`

```text
rerail pack check <pack directory or installed package>... [--corpora <dir>] [--json]
```

It runs these checks in order and exits non-zero if any fails:

| # | Check | Fails when |
|---|---|---|
| 1 | Manifest | The manifest does not match the schema; ids repeat; a `document` is missing, its `id` differs, or a file in `rules/` is not listed |
| 2 | Package | `package.json` lacks `rerail.pack`; `name`, `license` or the core peer range disagree with the manifest; `files` misses a published path or includes `validation/`; it has `dependencies` |
| 3 | Compatibility | The running core is outside `coreRange`; a language is not supported; a rule document does not compile against core's capabilities |
| 4 | Examples | A `caught` example produces no violation for its rule, or a `passes` example produces one |
| 5 | Upstream evidence | With `validation`: a ported rule's violations and the tool's recordings differ outside the declared divergences; a divergence no longer matches anything (stale); a declared capability has neither a ported rule nor an exemption |
| 6 | Licences | An `upstream` project has no `licenses/<project>/`; a file under `validation/upstream/` is not listed in an `UPSTREAM.md`; a licence falls outside the repository allowlist |

Checks 1 to 4 need only the published package. Check 5 needs a source checkout (`validation/`), and corpus
recordings also need the pinned corpus under `--corpora` or `.rerail/corpora/<pack>/`; a missing corpus is reported
and fails in CI. `--json` prints per-rule results: compile status, examples, and true positives, false positives and
false negatives per source.

In this repository, `npm test` runs `rerail pack check packs/*` with the local core. Pack CI also runs it against
the newest published core that satisfies each pack's `coreRange`, so a pack never relies on unreleased engine work
without saying so.

## Versioning and release

| Change | Version bump |
|---|---|
| A rule's false positive or false negative fixed, wording, examples, recordings | patch |
| A rule added (off or on), `coreRange` widened | minor |
| A rule removed or its id renamed, a default `mode` made stricter, `coreRange` narrowed | major |

Each pack is released on its own: a tag `pack-<id>-v<version>` publishes `packs/<id>/` to npm through the
repository's release workflow and trusted publishing, under `next` first and `latest` once promoted. Packs stay
`restricted` until launch, like the core package.

## Licensing

- Rerail's own material in a pack is Apache-2.0.
- Material copied or adapted from an upstream project keeps that project's licence: it is listed in
  `licenses/<project>/UPSTREAM.md` and annotated in the repository's `REUSE.toml`.
- The pack's `license` names every licence its published files carry, for example `Apache-2.0 AND MIT` for rules
  adapted from an MIT tool. Whether adapted rule documents need the upstream licence or Apache-2.0 alone is an open
  legal question; until it is settled, packs name both.
- Only licences on the repository allowlist are accepted (`scripts/check-licenses.mjs`).

## Moving to this format

| Today | Becomes |
|---|---|
| `policies/rule-packs/tslint.json` and `code-rule-dsl/rules/opt-in/tslint/` (shipped in core) | `packs/tslint/`: manifest and `rules/`, once core loads installed packs |
| `conformance/tslint/rules/**` (option variants and ports that are not shipped) | `packs/tslint/validation/candidates/` |
| `conformance/tslint/` golden manifest and audit | The `tslint` manifest's rule list and `validation/divergences.json` |
| `conformance/languages/<lang>/pack.json`, `rules/`, `divergences.json` | `packs/<lang>/` manifest, `rules/` or `validation/candidates/`, and `validation/divergences.json` |
| `fixtures/<family>/rules/<tool>/` recordings, copied tests and recorders | `packs/<lang>/validation/upstream/<tool>/` |
| `fixtures/<family>/{candidates,probes,policies}/` | `packs/<lang>/validation/candidates/` |
| `fixtures/<family>/licenses/<project>/` for rule tools | `packs/<lang>/licenses/<project>/` |
| `conformance/languages/` runner and scorer | `rerail pack check` in core (`src/`) |
| Everything else under `fixtures/` (parsing, scope, values, calls oracles) | Stays: engine tests, not pack validation |
| TSLint's upstream tests, fetched at a pinned commit by `conformance/tslint/fixtures.ts` | A `fixtures` recording under `packs/tslint/validation/upstream/tslint/`, like any other tool |
