# Language packs

Status: the registry, the not-supported-yet behaviour and the engine adapter seam are implemented. Each family's
engine code sits behind one adapter (`native/src/code_rule_dsl/language/adapter.rs`), and the host's schema checks
shapes only: the engine alone refuses what a rule's language lacks.

## What a language pack is

A language is three things, each owned by one place:

| Part | Where | Holds |
|---|---|---|
| Registry entry | `languages/<id>.json` | What the language is: id, family, dialects (file suffixes, parser, grammar, tape tag), which capabilities it supports, suppression comments, embedded languages, and its status |
| Engine adapter | `native/src/code_rule_dsl/language/adapter.rs` and its concern files (one adapter per family) | How the engine reads it: scope, values, call arguments, call graph, format strings, suppressions, and the DSL features it refuses |
| Rule pack | `packs/<id>/` ([pack structure](pack-format.md)) | Code rules written in it, with the evidence that they behave |

## The layering rule

Language code lives only in its adapter and its registry entry. Core, the host and packs never branch on a language:
they ask the registry or the adapter. The engine generates its language tables from the registry at build time
(`native/build.rs`, `native/src/build_languages.rs`), and the host reads a module generated from the same files
(`src/engine/code-rule/code-rule-language-registry.generated.ts`, written by `npm run generate:languages`). A test fails if the
generated module is stale, and a native test checks that the host module and the engine's tables agree.

Whether a rule uses something its language lacks (a key, an option, a format style, a suppression comment, another
family's grammar) is the engine's answer alone. The host's schema checks a rule's shape; every write, and
`validateCodeRule` from `@neralabs/rerail/rules`, then compiles it in the engine, which refuses it as invalid, or as
`language-not-supported` when the registry plans the language or capability.

## The engine adapter

Each registry family has one adapter type (`Python`, `Ecma`, `Php`, `Blade`, `Rust`, `Vue`, `Sql`) implementing `Adapter`. The
`with_adapter!` macro, generated from the registry's families, is the one exhaustive match on a family, and the
registry also generates `Family`, `GrammarDialect`, `Family::of` and `honours`, so no enum is edited by hand. Every
other call site asks the adapter. A call site that needs a concern's associated type calls a function generic over
`A: Adapter` from inside the macro. A family's call names carry their family (`CallRules::NAMES`), and its value
semantics (`truthy`, `type_name`, `equals`, `order`, `relation`, `numeric`, `integer`, `to_str`) belong to
`ValueRules`. [Adding a language](#adding-a-language) lists the steps.

| Part | Holds |
|---|---|
| `Adapter` | The family's diagnostic label, line terminators, name normalisation (Python's NFKC, SQL's PostgreSQL case folding), module paths from package markers (Python), and inline suppression comments (an embedding family's read every document of its files) |
| `ScopeRules` (`adapter/scope.rs`) | Scope tables: building, resolving, bindings, usage, PHP's flow, SQL's query blocks, and refusals of binding and resolution features |
| `ValueRules` (`adapter/value.rs`) | Value decoding, kinds, structures, names, keys and references for comparisons, type names, and refusals of compare modes |
| `ArgumentRules` (`adapter/arguments.rs`) | Call kinds, the arguments a call passes, and refusals of signature markers and modes |
| `CallRules` (`adapter/calls.rs`) | Call and function kinds, per-file call summaries, call-graph links and labels |
| `FormatArgRules` (`adapter/format_args.rs`) | The format styles a family reads |
| `SyntaxRules` (`adapter/syntax.rs`) | Literal grammars a family hosts (`parse`), `languageOptions` and `when` keys |

A concern the registry does not mark `supported` for a family is `Refused` (Ecma's, Vue's and Sql's `calls` and
`formatArgs`). An embedding family's semantics are the embedded language's, over the embedded program, which is a
document of its own read with that language's family (Blade's view with PHP's, Vue's program with Ecma's); its own
concerns name the embedded language's for the compile's checks, and a semantic key over the file's own tree is
refused (`compile::documents::check_sides`). Refusals go through one gate (`gate`), which reports a refused feature
as invalid and a planned one as `language-not-supported`. The adapter kit (`adapter/kit.rs`) runs one generic suite
over every adapter, its semantic checks over the document each family's semantics read, and checks that each family's
adapter reads exactly the capabilities its registry entries support.

| Family | Languages | Adapter | Capabilities | Suppressions |
|---|---|---|---|---|
| `python` | `python` | `Python` (`adapter/scope.rs`, `value.rs`, ...) | all five | `noqa`, `nosec` |
| `ecma` | `typescript`, `javascript` | `Ecma` | `scope`, `value`, `arguments`; `calls` and `formatArgs` `Refused` | `eslint`, `oxlint`, `biome` |
| `php` | `php` | `Php` | all five | `phpstan`, `psalm`, `phpcs` |
| `blade` | `blade` | `Blade`, delegating to `Php` over the compiled view | all five | none |
| `rust` | `rust` | `Rust` (`adapter/rust.rs`) | all five | `clippy` |
| `vue` | `vue` | `Vue` (`adapter/vue.rs`), delegating to `Ecma` over the script-block program | `scope`, `value`, `arguments`; `calls` and `formatArgs` `Refused` | `eslint` |
| `sql` | `sql` | `Sql` (`adapter/sql.rs`) | `scope`, `value`, `arguments`; `calls` and `formatArgs` `Refused` | `sqlfluff` |

### Vue

Vue's adapter is `adapter/vue.rs`. A `.vue` file is two documents (`embed`): its tree-sitter-vue tree, and the one
program its `<script>` blocks and template expressions make (`vue::view`, embedded through `script-block`,
`embed/script_block.rs`): TypeScript for `lang="ts"`, TSX for `lang="tsx"`, JavaScript otherwise; a script in another
language (`coffee`) embeds none. The program is a tree of its own grammar over its own text, with the segments that map
its text to the file's bytes; a clause reads it with `ast.embedded: script-block`, and a template rule reaches it with
`embedded`.

| Concern | Vue |
|---|---|
| Documents | The component's tree; the program, read in the dialect `ScriptBlock` built it in. Each has its own syntax errors: the template's leave a clause over a clean program complete |
| Scope, value, arguments | Ecma's, over the program: template references resolve in the scopes Vue's compiler gives them; a token the view writes (an interpolation's `(` and `);`) is not spelled (`embed::rewritten`) |
| Calls, format arguments | Refused, as ECMAScript's are: vue-eslint-parser hands the scripts to ESLint, which has neither |
| Suppressions | `eslint`: ESLint's directives in the `<script>` blocks' comments (`suppress::vue::script_directives`, the program's comments at the file bytes they copy), or eslint-plugin-vue's template comment directives (`vue/comment-directive`, `VueDirectives`) |
| Patterns | A clause over the component's tree has Vue's patterns; one over the program, its dialect's |
| `languageOptions`, `when` | Ecma's keys |

### The Rust adapter

`adapter/rust.rs` reads Rust from tree-sitter-rust's syntax alone (`ast-grep:Rust`, tape tag 9): what needs types or
macro expansion, and names inside macro token trees and `macro_rules!` bodies, are not read. Each concern selects the
Rust module that implements it, and each is proven against a recording of the Rust 1.99.0 toolchain under
`fixtures/rust/` (`generate.py` beside each; run by hand, never in CI):

| Capability | Module | Reads | Oracle |
|---|---|---|---|
| `scope` | `scope/rust.rs`, `scope/usage/rust.rs` | Lexical name resolution as rustc does it: items visible in their whole block or module (an `extern` block's too), `let` and pattern bindings from where they are made, generic and `for<'a>` lifetimes in their item or type, no local of an enclosing `fn`; types, values, macros, lifetimes and labels as separate namespaces. A macro invoked where an item may stand may define items: a name nothing else binds there is not unresolved. `resolves.import` lists `::` paths, a read's head read through `use` and `extern crate`, `self::`/`super::` normalised to `crate::` paths | rust-analyzer's SCIP occurrences and rustc's unused-name diagnostics (`fixtures/rust/scope`) |
| `value` | `value/rust.rs` | Literals and constant expressions as rustc evaluates them (integers of any width with their suffix's type, floats, `char`, strings, byte and C strings), within the decode's depth cap; `follow` reads a `const` item of the file | rustc's values for every literal and expression case (`fixtures/rust/literal`, `fixtures/rust/value`) |
| `arguments` | `matcher/arguments/rust.rs` | A call's positional arguments | rustc's binding of each case's arguments (`fixtures/rust/matcher/arguments`) |
| `calls` | `calls/rust.rs`, `Graph::link_rust` | A crate's functions by path, `Self::f`/`Type::f` through its `impl` blocks, `self.m()` in an inherent `impl`; method calls on other values, trait dispatch, closures, function pointers and macros whose expansion may call a crate function are open | rust-analyzer's resolution of every call in a test crate (`fixtures/rust/calls`) |
| `formatArgs` | `literal/rust_format.rs`, `matcher/format_args/rust.rs` | `rustFormat` templates and style `rust`: the `format_args!` family as rustc binds it | rustc's diagnostics for every format macro case (`fixtures/rust/matcher/format_args`) |

Suppression is `clippy` (`suppress/clippy.rs`): rustc's and Clippy's lint-level attributes (`allow`, `expect`, `warn`,
`deny`, `forbid`), outer and inner, with those a `cfg_attr` applies when its predicate holds, and with
`suppress.clippy.msrv` the `#[clippy::msrv]` in force. Its oracle is `clippy-driver` over UI fixtures and the ripgrep
and tokio corpora (`fixtures/rust/suppress`). The attributes are read per file: a level set in another file (a crate
root's `#![allow(..)]` over its modules) or on the command line is not seen, and a `cfg` predicate other than
`all`, `any` and `not` over `clippy` is false. Rust refuses `languageOptions`, `resolves.imported` and every other
family's grammar and format style; the Clippy pack (`packs/rust/`) ships the syntax ports that pass the port readiness gates and keeps the rest as candidates.

### The SQL adapter

`adapter/sql.rs` reads SQL as SQLFluff 4.4.0 does, over one generic dialect (`GrammarDialect::Sql`, the vendored
tree-sitter-sql with Rerail's patches, `native/grammars/tree-sitter-sql/PATCHES.md`) for PostgreSQL, SQLite, MySQL
and ANSI files. Where the dialects read a file differently it reads PostgreSQL's, and the pack declares the other
readings as divergences. Each concern and its oracle:

| Concern | What it reads | Oracle |
|---|---|---|
| `Adapter` | A naked identifier folds to lower case and a quoted one is exact, as PostgreSQL looks names up (`value::sql::identifier`); SQLFluff's `-- noqa` (`suppress/sqlfluff.rs`) | `fixtures/sql/suppress/sqlfluff.expected.json`, SQLFluff's reports with `noqa` ignored and honoured |
| `ScopeRules` | Query blocks (`ScopeKind::Query`, scope `query`) from `scope/sql.rs`: a block binds its `FROM`/`JOIN` relations (`Form::Relation`), its `UPDATE`/`DELETE`/`INSERT` target (`Form::Target`), a block around a `CREATE POLICY` or `ALTER POLICY` its table (`Form::Target`, which the queries in its `USING` and `WITH CHECK` see, as SQLFluff's RF01 reads it on PostgreSQL), and a block around a `WITH` statement its CTE names (`Form::Cte`); a qualified column reference's table part resolves to a relation or target of its block or an enclosing one (a CTE only through a relation over it). `resolves.unresolved` is RF01's report and `binds: { used: false }` AL05's | SQLFluff's RF01 and AL05 recordings (`packs/sql/upstream/sqlfluff/fixtures.expected.json`, pack test data: `npm run fixtures:pull`), with the declared ANSI-versus-PostgreSQL mismatches |
| `ValueRules` | Literals and names (`value/sql.rs`) only where PostgreSQL's and SQLite's readings agree; `type`, the key, member, reference and structural compare modes are refused | `fixtures/sql/scope/postgres.expected.json` (PostgreSQL 18.6 through libpg_query) and `fixtures/sql/value/sqlite.expected.json` (SQLite 3.50.4) |
| `ArgumentRules` | An `invocation`'s arguments as PostgreSQL 18.6's parser binds them (`matcher/arguments_sql.rs`): by position, then by name; signature markers, `syntactic` and `estree` are refused | `fixtures/sql/matcher/arguments/postgres.expected.json`, 50 calls: 49 matched by position, 1 asserted unreadable (`substring(s FOR n)`, whose start PostgreSQL supplies) |
| `SyntaxRules` | No literal grammar (`parse`) and no `languageOptions`; `when` reads the document's options | |

`calls` and `formatArgs` are `refused` in `languages/sql.json`, because SQL gives the engine nothing to read for them
(the evidence is in the module doc of `adapter/sql.rs`):

- `calls`: no SQLFluff rule follows a call into a routine. A routine's body is a string literal of another language
  (`$$ ... $$`, PL/pgSQL), not SQL the grammar parses, and a `.sql` file has no workspace-resolvable call graph: no
  import or module links one file's routines to another's.
- `formatArgs`: no SQLFluff rule reads a format string, and ISO SQL has no format function; PostgreSQL's `format()`
  and SQLite's `printf()` are dialect extensions.

A rule reading either is invalid for `sql`, never `language-not-supported`. The pack's ports read `syntax`, `value`,
`compare`, `scope` and `suppression`; none reads `arguments`, which the capability's own oracle proves.

## The registry entry

One file per language, `languages/<id>.json`, so a language is added or changed without touching another's file.

```json
{
  "format": "rerail.language/v1",
  "id": "blade",
  "status": "supported",
  "family": "blade",
  "dialects": [
    { "label": "blade", "variant": "Blade", "suffixes": [".blade.php"], "parser": "forte", "grammar": "native:forte", "tapeTag": 8 }
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
| `dialects` | yes | The grammars its files are parsed with. Each has a `label` (the file language hosts and the store use), its file `suffixes` (lowercase, with the dot; compound ones such as `.blade.php` win over shorter ones), and, when supported, the engine's `variant` (`GrammarDialect::<variant>`), its `parser` (`tree-sitter`, or `forte`: Rerail's port of Forte, [docs/blade-parser.md](blade-parser.md)) and its `grammar` (`vendored:<name>`, built by `build.rs`; `native:<name>`, a parser Rerail implements itself; or `ast-grep:<Language>`). The `tapeTag` is required on a supported dialect; a `planned` dialect may declare it to reserve its future tag. It is persisted, unique across all entries, and never reused. Taken today: Rust 9, SQL 10 and Vue 12 (11 is free for a second SQL dialect) |
| `runsOn` | no | The dialect labels its rules run on, its own first; default: its own dialects. A `typescript` rule also runs on JavaScript files |
| `capabilities` | yes | For each engine capability (`scope`, `value`, `arguments`, `calls`, `formatArgs`): `supported`, `refused` (the language has no such thing, or no plan for it: a rule using it is invalid) or `not-supported-yet` |
| `suppressions` | yes | Linter suppression comments its files honour (`noqa`, `nosec`, `eslint`, `oxlint`, `biome`, `phpstan`, `psalm`, `phpcs`, `clippy`, `sqlfluff`) |
| `embeds` | no | Languages embedded in its files and how they are reached (`via`): Blade's PHP through the compiled view, Vue's script blocks. A rule runs on the program with `ast.embedded: "<via>"` and reaches it from the file's own tree with `embedded` ([DSL reference](code-rule-dsl-reference.html#embedded)) |

`parser` is validated but drives no behaviour yet. `embeds` names the embedded language and how it is reached
(`via`); a supported entry may embed only supported languages. An embedding's `Embedder` builds at most one program
per file and `via`, a document derived from the file's own (`tape::DerivedImage`), with a tape of its own that the
store keeps as a row of its own beside the file's (`file_documents`). Changes
stay additive within `rerail.language/v1`: a new optional field or a new value is a v1 change, and renaming or removing
a field, or changing what a value means, needs `rerail.language/v2`.

The build fails, and so does `test/engine/code-rule/language-registry.test.ts`, when ids, dialect labels, suffixes or tape tags repeat;
when a supported dialect lacks a variant, parser, grammar or tape tag; when a planned language supports a capability
or names `runsOn`; when entries name different capabilities; when `runsOn` names a dialect of another family; or when
`embeds` names an unknown language.

## Status lifecycle

1. **Planned.** The entry lists the language's dialects with their suffixes and no grammar, and every capability is
   `not-supported-yet`. A code rule may name the language, so packs for it can land first, but the engine reads none
   of it:
   - compiling the rule fails with the stable code `language-not-supported` (the engine's message starts with it and
     names the language), never a schema error;
   - the catalog loads the rule but keeps it inactive: every audit skips it, it is never an audit failure, an installed
     pack holding it is not skipped, and the rule APIs (`GET /api/repository/rules`, `GET /api/rerail-rules`,
     `GET /api/library-rules`) and the activity API (`GET /api/rules/activity`, `measured: false`) return it with
     `inactive: { code, reason }`;
   - audits parse no file of it: its suffixes select no dialect, and its files are read as `text` for regex clauses;
   - the pack tests report its rules as `not supported yet` without failing
     ([validation contract](pack-format.md#the-validation-contract)).
2. **Capabilities, one at a time.** A capability becomes `supported` only when its oracle suite passes for the
   language (its `fixtures/` oracles and the conformance scores of the code rules that read it). A rule that reads a
   capability still `not-supported-yet` is refused as `language-not-supported` and kept inactive, as above.
3. **Supported.** The entry flips to `supported` once its dialects have a grammar, a parser and a tape tag and its
   adapter is in place; the build fails until each exists.

## The test-only language

`native/test-languages/exemplar.json` is a planned language only tests see: family `exemplar`, one dialect `exemplar`
with the suffix `.exemplar`, every capability `not-supported-yet`, no suppressions and no tape tag. A test that needs
"a planned language" names it, so flipping a real planned language to supported never touches a test. It is in no
package and never in the product:

- `languages/` and the generated host module do not list it; `npm run generate:languages` reads `languages/` alone.
- `native/build.rs` validates it with the same rules as any entry, and against the registry for repeated ids, labels,
  suffixes and tape tags, and generates it only as `#[cfg(test)]` items: `PLANNED_LANGUAGES` lists it in test builds
  (`TEST_PLANNED_LANGUAGES` names the languages only tests plan) and not otherwise. It is no part of the engine's
  identity.
- A host test mocks the generated module with `exemplarRegistryModule()` and the native module with
  `nativeRefusingExemplar` (`test/engine/support/exemplar-registry.ts`, `exemplar-engine.ts`): the native binding the host
  loads is a product build and does not know it. `nativeRefusingExemplar` refuses only rules that name the exemplar and
  hands every other rule to the real binding. The engine's own refusal, `language-not-supported`, is checked by a native
  test and, through the real binding with nothing mocked, by `test/engine/code-rule/language-not-supported-engine.test.ts`,
  which compiles a rule of a product language the registry still plans.

Embedding has a test-only counterpart in the engine: the test-only HTML grammar embeds TypeScript, TSX and JavaScript
through `Via::TestScript` (`language::test_embeds`, `native/src/code_rule_dsl/embed/test_script.rs`), whose program is
an HTML file's scripts and then its `on*` handlers. Its tests (`embed/tests.rs`) run the tape's embedder loop, the
derived documents persisted apart from the file's tape and their dialects, and scope and usage over a program whose
text maps to the file out of order.
The generated `Via` and `with_embedder!` carry it only under `#[cfg(test)]`.

## Adding a language

1. Add `languages/<id>.json` with `status: "planned"`: its family, its dialects' labels and suffixes, every
   capability `not-supported-yet`, its suppressions and anything it embeds. Give each dialect the `tapeTag` it
   reserves (Rust 9, SQL 10 and Vue 12 are taken; 11 is free for a second SQL dialect). Run
   `npm run generate:languages` and commit the regenerated `src/engine/code-rule/code-rule-language-registry.generated.ts`.
   Rules and packs for it can land now.
2. Vendor or pick its grammar (a vendored one is added to `GRAMMARS` in `native/build.rs`), and flip the entry to
   `supported` with each dialect's `variant`, `parser` and `grammar`. The build generates `GrammarDialect::<Variant>`,
   `Family::<Family>`, `Family::of` and `honours`, and the `with_adapter!` arm; no enum is edited by hand. The build
   fails until `adapter::<Family>` exists.
3. Write its adapter module, in its own file, registered by one `mod` and one `pub(crate) use` line in
   `native/src/code_rule_dsl/language/adapter.rs`. For a new family, its type implements `Adapter` and each concern
   trait, or `Refused` for a concern the language will not have:
   - `ScopeRules`;
   - `ValueRules`, with `const LANG: Family = Family::<Family>` and the hooks `truthy`, `type_name`, `equals`,
     `order`, `relation`, `numeric`, `integer` and `to_str`;
   - `ArgumentRules`;
   - `CallRules`, with `const NAMES: Option<Family>`, the family whose namespace its call names live in;
   - `FormatArgRules`;
   - `SyntaxRules`.
   A new dialect in an existing family needs no per-dialect table: ECMAScript's scope reads each dialect's grammar
   (`scope::ecma::script`).
4. A suppression kind the language needs and the engine lacks is one row in `suppression_kinds!` in
   `native/src/code_rule_dsl/suppress.rs` (`Variant key: Spec = linter::spec, "detail";`, its parser in
   `suppress/<linter>.rs`), one entry in `codeRuleSuppressSchema` in
   `src/engine/code-rule/code-rule-schema-semantic.ts`, its name in the registry entry's `suppressions`, and its use in
   the adapter's `suppresses`.
5. An embedding (`embeds` with a new `via`) adds `native/src/code_rule_dsl/embed/<via>.rs`, implementing `Embedder`
   (`build`, which returns an `embed::Program`: its dialect, its tree over its own text, the `SegmentMap` that maps the
   text to the host's bytes, copied or written over them and in any order, and the host's syntax errors it reads
   through), and one `mod` and one `pub(crate) use` line in `embed.rs`. `build` returns one program or none, so a file
   holds at most one program per `via`. Nothing else in the engine changes:
   - The program is a document derived from the file's (`tape::DerivedImage`): its text, source map, host errors and
     its own tape, in its own grammar's codes, checked as every tape is. The store keeps that tape as a row of its
     own, related to the file's tape by `file_documents` ([store](content-addressed-workspace-store.md)).
   - A clause reads it with `ast.embedded: <via>` and a host rule reaches it with `embedded`; a program node reaches
     the host with `host` (`matcher::embedded`). Its witnesses, captures and fixes are placed at the file bytes they
     stand for: every document is an `origin::Origin` into the one it was read from, and `origin::InFile` places
     its nodes and ranges in the file (a fix only over bytes it copies). Nothing else maps a position. A node
     stands for the file ranges of its pieces (`embed::segments::FileRanges`), apart where the program is written
     out of file order and joined only over white space; a single span reports the range it starts in, and
     positions are half-open (a boundary belongs to the range starting there).
   - Each document has its own syntax errors as ranges of the file (`soundness::SyntaxErrors::of_doc`): a program
     whose `host_errors` are empty is complete whatever the host's errors.
   - Calls read the program whose dialect's calls join the graph (`calls::calls_document`).

   The embedding family's adapter (step 3) names the embedded family's concerns, as Blade's does PHP's.
6. Add one `#[test]` function for the family to `native/src/code_rule_dsl/language/adapter/kit.rs`, calling
   `kit::<Family>(&Expect { .. })` with the family's expectations.
7. Flip each capability to `supported` only when it is implemented and proven by its oracle suite; mark one the
   language will never have `refused`. Flip `status` last.

## Start a pack

A pack is a directory `packs/<id>/` that passes the [validation contract](pack-format.md#the-validation-contract).
The models to copy: `packs/python/` or `packs/typescript/` (ported language packs), `packs/node/` or `packs/next/`
(framework packs over a supported language), `packs/rust/` or `packs/sql/` (language packs whose ports
stay in `candidates/rules/` until they pass the port readiness gates), `fixtures/packs/valid/` (the
smallest pack, with an example-proven rule and a ported one) and `fixtures/packs/exemplar/` (a pack for a planned
language).

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
      "id": "laravel-no-dd",
      "document": "rules/laravel-no-dd.json",
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

`packs/laravel/rules/laravel-no-dd.json`, a code rule (`rerail.policy/v1`, see the
[code-rule DSL reference](code-rule-dsl-reference.html)) whose `id` is the one `pack.json` lists and its file name.
A rule id is lowercase letters, digits and dashes, in one style for every pack ([rule ids](pack-format.md#packjson)):

```json
{
  "format": "rerail.policy/v1",
  "id": "laravel-no-dd",
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
npx vitest run test/engine/catalog/packs.test.ts -t "pack laravel"
```

Then add a changeset for the new pack (`patch`, "Add the Laravel pack: …"). Every later change to a pack adds one too;
[versioning](pack-format.md#versioning-and-release) says which bump (`patch` until the pack reaches 1.0.0):

```sh
npx changeset   # pick @neralabs/rerail-pack-laravel, choose the bump, write the summary
```

Each check and each rule is one test; a failure names the rule and the file, for example
`laravel-no-dd: the caught example (app/Http/Controllers/UserController.php) produces no LARAVEL-NO-DD violation`.
A rule needs proof: `examples` (one caught, one that passes, each with a `path` inside the rule's scope), or a port
entry (below).

### A pack for a planned language

The same files, with rules whose `language` the registry lists as `planned`, or that read a capability their
language has `not-supported-yet`. The pack lands: its checks pass, and each such rule's test is skipped as
`not supported yet` with the engine's reason, for example
``/clauses/0/forbid/ast/language: `exemplar` is not supported yet``. Write its `examples` now; checks 3 and 4 run
them once core reads the language, and any other compile error fails check 1 today. `fixtures/packs/exemplar/` is a
complete example, for the test-only planned language above.

### A ported pack

A pack whose rules reproduce another tool's rules adds the validation material, which `files` never publishes:

1. `conformance.json` ([schema](pack-format.md#conformancejson-divergencesjson-upstream)): the family, the
   capabilities the pack declares, the pinned `tools` and `corpora`, and per ported rule its `upstream` tool and rule,
   its `id` (the rule's own id) and `policy` (the same `rules/<id>.json` as the rule's `document`), how findings
   compare (`line` or `line+column`) and the capabilities it exercises. A port's clauses report the upstream rule's
   code. Every declared capability needs a ported rule or an `exemptCapabilities` entry with its evidence.
2. `upstream/<tool>/`: the tool's tests copied into `fixtures/`, and `fixtures.expected.json`, what the pinned tool
   reported on them (`recordingSchema` in `conformance/languages/pack.ts`), written by a recorder you keep beside it
   (`packs/python/upstream/ruff/generate.py` is one). Optionally `corpus-<name>.expected.json` per pinned corpus.
   These are derived pack test data, never in git: add the tool-pack's recipe under `scripts/pack-data/recipes/` (one
   module per pack, listed in its `index.ts`), so `npm run fixtures:derive -- --pack <id>` makes them from the pins.
   Cases you wrote for the tool (listed in its `UPSTREAM.md`) or a corpus with its ground truth under
   `upstream/<tool>/authored/` are authored: check them into your pull request with `git add -f`. Before merge a
   maintainer archives the tool-pack's data from a trusted worktree of main (`npm run fixtures:push -- --from <ref>`),
   which writes its `upstream/<tool>/test-data.lock` for the pull request to commit
   ([CONTRIBUTING.md](../CONTRIBUTING.md#merging-maintainers)). Get the other packs' with `npm run fixtures:pull`.
3. `licenses/<tool>/`: the tool's licence, verbatim, and `UPSTREAM.md` with a `License: <SPDX id>` line and a table
   listing every copied file by its pack-relative path. Add the tool's licence to `package.json`'s `license`
   (`Apache-2.0 AND MIT` for an MIT tool), and annotate the copied files in `REUSE.toml`.
4. `divergences.json`: each line where Rerail and the tool disagree, with its kind and the evidence checked. Start
   with `[]` and add an entry only for a disagreement you have checked.

Then score one rule with every false positive and false negative listed, and run the contract:

```sh
node --import tsx conformance/languages/cli.ts --pack <id> --rule <port id> --fixtures
npx vitest run test/engine/catalog/packs.test.ts -t "pack <id>"
```
