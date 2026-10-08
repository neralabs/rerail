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
