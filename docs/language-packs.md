# Language packs

Status: the registry and the not-supported-yet behaviour are implemented. The adapter seam is not: each language's
engine code is still matched on its family across the engine (`Family::` arms in `native/src/code_rule_dsl/`) and in
the host's schema checks (`src/code-rule-schema.ts`). Moving that code behind one adapter per language is the next
change.

## What a language pack is

A language is three things, each owned by one place:

| Part | Where | Holds |
|---|---|---|
| Registry entry | `languages/<id>.json` | What the language is: id, family, dialects (file suffixes, parser, grammar, tape tag), which capabilities it supports, suppression comments, embedded languages, and its status |
| Engine adapter | `native/src/code_rule_dsl/` (today: its `GrammarDialect` variant and its family's arms) | How the engine reads it: scope, values, call arguments, call graph, format strings |
| Rule pack | `packs/<id>/` ([pack structure](pack-format.md)) | Code rules written in it, with the evidence that they behave |

## The layering rule

Language code lives only in its adapter and its registry entry. Core, the host and packs never branch on a language:
they ask the registry. The engine generates its language tables from the registry at build time
(`native/build.rs`, `native/src/build_languages.rs`), and the host reads a module generated from the same files
(`src/code-rule-language-registry.generated.ts`, written by `npm run generate:languages`). A test fails if the
generated module is stale, and a native test checks that the host module and the engine's tables agree.

## The registry entry

One file per language, `languages/<id>.json`, so a language is added or changed without touching another's file.

```json
{
  "format": "rerail.language/v1",
  "id": "blade",
  "status": "supported",
  "family": "blade",
  "dialects": [
    { "label": "blade", "variant": "Blade", "suffixes": [".blade.php"], "parser": "tree-sitter", "grammar": "vendored:blade", "tapeTag": 8 }
  ],
  "capabilities": { "scope": "supported", "value": "supported", "arguments": "supported", "calls": "supported", "formatArgs": "supported" },
  "suppressions": [],
  "embeds": [{ "language": "php", "via": "compiled-view" }]
}
```

| Field | Required | Meaning |
|---|---|---|
| `format` | yes | `rerail.language/v1` |
| `id` | yes | The code-rule language: what a code rule's `language` names. Equals the file name |
| `status` | yes | `supported` or `planned` (see the lifecycle) |
| `family` | yes | The semantic family that reads it. Languages that share one (`typescript` and `javascript` share `ecma`) share its suppressions |
| `dialects` | yes | The grammars its files are parsed with. Each has a `label` (the file language hosts and the store use), its file `suffixes` (lowercase, with the dot; compound ones such as `.blade.php` win over shorter ones), and, when supported, the engine's `variant` (`GrammarDialect::<variant>`), its `parser`, its `grammar` (`vendored:<name>`, built by `build.rs`, or `ast-grep:<Language>`) and its `tapeTag` (persisted, never reused) |
| `runsOn` | no | The dialect labels its rules run on, its own first; default: its own dialects. A `typescript` rule also runs on JavaScript files |
| `capabilities` | yes | For each engine capability (`scope`, `value`, `arguments`, `calls`, `formatArgs`): `supported`, `refused` (the language has no such thing, or no plan for it: a rule using it is invalid) or `not-supported-yet` |
| `suppressions` | yes | Linter suppression comments its files honour (`noqa`, `eslint`, `phpstan`, `psalm`, `phpcs`) |
| `embeds` | no | Languages embedded in its files and how they are reached (`via`): Blade's PHP through the compiled view, Vue's script blocks |

`parser` and `embeds` are validated but drive no behaviour yet; the adapter refactor will use them. Changes stay
additive within `rerail.language/v1`: a new optional field or a new value is a v1 change, and renaming or removing a
field, or changing what a value means, needs `rerail.language/v2`.

The build fails, and so does `test/language-registry.test.ts`, when ids, dialect labels, suffixes or tape tags repeat;
when a supported dialect lacks a variant, parser, grammar or tape tag; when a planned language supports a capability
or names `runsOn`; when entries name different capabilities; when `runsOn` names a dialect of another family; or when
`embeds` names an unknown language.

## Status lifecycle

1. **Planned.** The entry lists the language's dialects with their suffixes and no grammar, and every capability is
   `not-supported-yet`. A code rule may name the language, so packs for it can land first, but the engine reads none
   of it:
   - compiling the rule fails with the stable code `language-not-supported` (the engine's message starts with it and
     names the language), never a schema error;
   - the catalog stores the rule but keeps it inactive: every audit skips it, it is never an audit failure, and the
     rule APIs (`GET /api/rules`, `GET /api/rules/{id}`, `PUT /api/rules/{id}`) and the activity API
     (`GET /api/rules/activity`, `measured: false`) return it with `inactive: { code, reason }`;
   - audits route no file to it: its suffixes select no dialect;
   - the pack tests report its rules as `not supported yet` without failing
     ([validation contract](pack-format.md#the-validation-contract)).
2. **Capabilities, one at a time.** A capability becomes `supported` only when its oracle suite passes for the
   language (its `fixtures/` oracles and the conformance scores of the code rules that read it). A rule that reads a
   capability still `not-supported-yet` is refused as `language-not-supported` and kept inactive, as above.
3. **Supported.** The entry flips to `supported` once its dialects have a grammar, a parser and a tape tag and its
   adapter is in place; the build fails until each exists.

## Adding a language

1. Add `languages/<id>.json` with `status: "planned"`: its family, its dialects' labels and suffixes, every
   capability `not-supported-yet`, its suppressions and anything it embeds. Run `npm run generate:languages` and
   commit the regenerated `src/code-rule-language-registry.generated.ts`. Rules and packs for it can land now.
2. Vendor or pick its grammar (a vendored one is added to `GRAMMARS` in `native/build.rs`), and give each dialect a
   `variant`, `parser`, `grammar` and the next unused `tapeTag`.
3. Write its adapter: add the `GrammarDialect` variant and, for a new family, the `Family` variant; the build then
   names every place the engine must learn the language.
4. Flip `status` to `supported`, and each capability to `supported` as its oracle suite passes; mark one the language
   will never have `refused`.

## Known parser gaps

A known parser gap is source a language accepts but the engine's grammar cannot parse, confirmed and listed on
purpose. The ledger in `conformance/t1/` belongs to the language conformance layer: each gap names its grammar, the
grammar's version (`Cargo.lock`, or `native/grammars/<grammar>/PATCHES.md` for a vendored one) and the smallest probe
that fails to parse, and a test parses every probe with the engine. A probe that parses is stale and fails the test,
so the ledger always says what the engine cannot read today: when a grammar fix or upgrade makes a probe parse, remove
its gap in the same change.

The ledger starts with the 13 gaps rule validation confirmed in upstream's tests for the Python and ESLint-family
packs: 6 in tree-sitter-javascript 0.25.0, 5 in tree-sitter-typescript 0.23.2 and 2 in tree-sitter-python 0.25.0. It
records gaps; it excuses nothing. The [validation contract](pack-format.md#the-validation-contract) does not read it,
so a difference a gap causes in a port's score stays that port's failure.

## Start a pack

A pack is a directory `packs/<id>/` that passes the [validation contract](pack-format.md#the-validation-contract).
The models to copy: `packs/python/`, `packs/php/` or `packs/typescript/` (ported language packs), `packs/react/`, `packs/node/` or
`packs/next/` (framework packs over a supported language), `test/fixtures/packs/valid/` (the smallest pack, with an
example-proven rule and a ported one) and `test/fixtures/packs/vue/` (a pack for a planned language).

### A pack for a supported language

A Laravel pack with one code rule proven by examples. The skeleton passes as written.

```sh
mkdir -p packs/laravel/rules
cp LICENSE packs/laravel/LICENSE
```

`packs/laravel/package.json`:

```json
{
  "name": "@neralabs/rerail-pack-laravel",
  "version": "0.1.0",
  "license": "Apache-2.0",
  "files": ["pack.json", "rules", "LICENSE", "README.md"],
  "peerDependencies": { "@neralabs/rerail": ">=0.1.0-0 <0.2.0" },
  "peerDependenciesMeta": { "@neralabs/rerail": { "optional": true } },
  "publishConfig": { "access": "restricted" }
}
```

`packs/laravel/pack.json`:

```json
{
  "format": "rerail.rule-pack/v1",
  "id": "laravel",
  "name": "Laravel",
  "category": "framework",
  "covers": "Laravel conventions: no debugging helpers left in application code.",
  "reason": "matches Laravel",
  "match": { "any": [{ "files": ["artisan"], "label": "an artisan file" }] },
  "rules": [
    {
      "id": "laravel:no-dd",
      "document": "rules/no-dd.json",
      "title": "No dd() or dump() calls",
      "group": "Laravel",
      "examples": {
        "caught": { "path": "app/Http/Controllers/UserController.php", "text": "<?php\nfunction show($user) {\n    dd($user);\n}\n" },
        "passes": { "path": "app/Http/Controllers/UserController.php", "text": "<?php\nfunction show($user) {\n    return view('user', ['user' => $user]);\n}\n" }
      }
    }
  ]
}
```

`packs/laravel/rules/no-dd.json`, a code rule (`rerail.policy/v1`, see the
[code-rule DSL reference](code-rule-dsl-reference.html)) whose `id` is the one `pack.json` lists:

```json
{
  "format": "rerail.policy/v1",
  "id": "laravel:no-dd",
  "revision": 1,
  "mode": "steer",
  "description": "A call to Laravel's dd() or dump() debugging helper.",
  "scope": { "include": ["app/**/*.php", "routes/**/*.php"], "exclude": [] },
  "clauses": [
    {
      "id": "no-dd",
      "forbid": { "ast": { "language": "php", "rule": { "kind": "function_call_expression", "has": { "field": "function", "regex": "^(dd|dump)$" } } } },
      "violation": { "anchor": "witness.span", "code": "LARAVEL-NO-DD", "message": "Remove dd()/dump() before committing" }
    }
  ]
}
```

Add a `README.md` saying what the pack covers, then record the new workspace and run the contract:

```sh
npm install --package-lock-only --ignore-scripts   # packs/* are npm workspaces: package-lock.json lists each
npx vitest run test/packs.test.ts -t "pack laravel"
```

Then add a changeset for the new pack (`minor`, "Add the Laravel pack: …"). Every later change to a pack adds one too;
[versioning](pack-format.md#versioning-and-release) says which bump:

```sh
npx changeset   # pick @neralabs/rerail-pack-laravel, choose the bump, write the summary
```

Each check and each rule is one test; a failure names the rule and the file, for example
`laravel:no-dd: the caught example (app/Http/Controllers/UserController.php) produces no LARAVEL-NO-DD violation`.
A rule needs proof: `examples` (one caught, one that passes, each with a `path` inside the rule's scope), or a port
entry (below).

### A pack for a planned language

The same files, with rules whose `language` the registry lists as `planned` (`vue`, `rust`, `sql`), or that read a
capability their language has `not-supported-yet`. The pack lands: its checks pass, and each such rule's test is
skipped as `not supported yet` with the engine's reason, for example
``/clauses/0/forbid/ast/language: `vue` is not supported yet``. Write its `examples` now; checks 3 and 4, and 7 to 9
for its ports, run once core reads the language, and any other compile error fails check 1 today.
`test/fixtures/packs/vue/` is a complete example.

### A ported pack

A pack whose rules reproduce another tool's rules adds the validation material, which `files` never publishes, and
its ports face every check of the [validation contract](pack-format.md#the-validation-contract), 1 to 9:

1. `conformance.json` ([schema](pack-format.md#conformancejson-divergencesjson-upstream)): the family, the
   capabilities the pack declares, the pinned `tools` and `corpora`, and per ported rule its `upstream` tool and rule,
   its `policy` (the same `rules/<name>.json` as the rule's `document`), how findings compare (`line` or
   `line+column`) and the capabilities it exercises. A port's clauses report the upstream rule's code. Every declared
   capability needs a ported rule or an `exemptCapabilities` entry with its evidence. Each tool lists its approved
   configurations in `recommended`, in order, which declares its source facts: the
   [guard](pack-format.md#the-scrub-list-scrubjson) refuses a new port whose tool has none.
2. `upstream/<tool>/`, written by a recorder you keep beside it (`packs/python/upstream/ruff/generate.py` is one) and
   run against the pinned tool and upstream checkout: the tool's tests copied into `fixtures/`;
   `fixtures.expected.json`, what the tool reported on them under the first approved configuration that turns each
   rule on, with each finding's end where the tool reports one (`recordingSchema` in `conformance/languages/pack.ts`);
   `source.json`, the tool's [source facts](pack-format.md#source-facts-and-recordings) (`upstreamSourceSchema`),
   naming each upstream test of a ported rule the fixtures leave out (`unscored`, which `ports:coverage` fails); and
   optionally `corpus-<name>.expected.json` per pinned corpus.
3. `licenses/<tool>/`: the tool's licence, verbatim, and `UPSTREAM.md` with a `License: <SPDX id>` line and a table
   listing every copied file and `upstream/<tool>/source.json` (extracted from the tool) by its pack-relative path.
   Add the tool's licence to `package.json`'s `license` (`Apache-2.0 AND MIT` for an MIT tool), and annotate the
   copied files (with the tool's licence) and `source.json` (with the tool's licence and Apache-2.0) in `REUSE.toml`.
4. No `divergences.json` entries and no `scrub.json`: the guard refuses new divergences in `packs/*` and scrub entries
   for new rules, so a new port agrees with its tool outright. The existing packs' divergences and scrub entries only
   shrink.

Then score one rule with every false positive and false negative listed, run the contract, and read the pack's report:

```sh
node --import tsx conformance/languages/cli.ts --pack <id> --rule <port id> --fixtures
npx vitest run test/packs.test.ts -t "pack <id>"
npm run validate:packs -- --write   # .rerail/reports/<id>-validation.md
```
