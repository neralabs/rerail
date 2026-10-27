# Rerail rule pack: Blade

10 rules for Laravel Blade templates, ported to Rerail: tlint 9.7.0's 5 lints, stillat/blade-parser 2.1.0's
`DebugDirectiveValidator`, `ForElseStructureValidator` and `ComponentShorthandVariableParameterValidator`, and
Bladestan 0.11.7's `if.alwaysTrue` and `identical.alwaysTrue` (PHPStan 2.2.16 rules over the compiled template). 7
more ports are candidates ([candidates/README.md](candidates/README.md)). The two Bladestan rules start off (`"on": false`): PHPStan
registers them at level 4 and Bladestan's recommended level is 0, so they are experimental. A port starts in
`candidates/rules/` and moves to `rules/` and `pack.json` once it passes every blocking port readiness gate
([docs/port-readiness.md](../../docs/port-readiness.md)); each of the pack's three tools is gated. Every port is scored
against the tool's own findings on the tool's test fixtures and on three pinned corpora, through the engine's
production workspace path, with every disagreement declared in `divergences.json`. The conformance check (`--check`,
run in CI) scores the fixtures: it gates the ports in `rules/` and reports the candidates without gating them.

It is checked by `npx vitest run test/engine/catalog/packs.test.ts -t "pack blade"` ([pack structure and validation
contract](../../docs/pack-format.md)).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): its match definition, each rule's document, title, group and, for a rule that starts off, `on`; two rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `candidates/rules/` | The candidates' code-rule documents (`rerail.policy/v1`), scored but never shipped |
| `candidates/README.md` | What each candidate is waiting on and its rule details; not published |
| `conformance.json` | Every port (shipped under `rules/`, or a candidate under `candidates/rules/`): its upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools, the pinned corpora (BookStack, Filament, Bagisto) and the `calls` exemption with its evidence |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-<name>.expected.json`; both pack test data, derived from the pins and archived in S3: `npm run fixtures:pull`) and the recorder (`generate.py` with its PHP helpers, sharing `upstream/common.py`; the template capture tlint's and stillat/blade-parser's test readers run, and stillat/blade-parser's reader itself, are the port readiness recorder's, under `conformance/recorders/port-readiness/adapters/`) |
| `upstream/<tool>/gates.yaml`, `recording.lock` | The port readiness gates' manifest for the tool and the lock naming its recordings (kept out of git, by sha256) ([docs/port-readiness.md](../../docs/port-readiness.md)) |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, listing every copied file |

Re-recording is a maintainer deriving the pack's test data from its pins and archiving it
([CONTRIBUTING.md](../../CONTRIBUTING.md#pack-test-data)); it is never committed. The recipe
(`scripts/pack-data/recipes/blade.ts`) runs each tool's `generate.py`, in `fixtures` mode and then `corpus` mode over
bookstack, filament and bagisto, against the pinned tool versions, with the pinned PHP 8.5.11 first on `PATH` (each
recorder's docstring has the setup):

```sh
npm run fixtures:derive -- --pack blade                    # every tool
npm run fixtures:derive -- --pack blade --tool bladestan   # one
npm run fixtures:push -- --pack blade --tool bladestan     # on your own branch: archive it and rewrite its test-data.lock
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack blade --rule tlint-space-after-blade-directives --fixtures
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `bladestan-identical-always-true`

Port of Bladestan 0.11.7 identical.alwaysTrue (PHPStan 2.2.16 StrictComparisonOfDifferentTypesRule over the compiled template). Constant evaluation follows variables through their one reaching binding. The condition of the last @elseif of a chain with no @else is not reported, matching PHPStan's reportAlwaysTrueInLastCondition: false.

### `bladestan-if-always-true`

Port of Bladestan 0.11.7 if.alwaysTrue (PHPStan 2.2.16 IfConstantConditionRule over the compiled template). Not reported: comparisons, negations, ternaries, isset and empty, conditions inside a trait, and an if preceded by an inline @var doc comment.

### `tlint-no-json-directive`

Port of tlint 9.7.0 NoJsonDirective. Read from the compiled view, where Blade compiles @json to a call, so it is caught in text, attribute values and script alike.

### `tlint-no-ray-directive`

Port of tlint 9.7.0 NoRayDirective. @ray is spatie/laravel-ray's directive, which core Blade does not compile, so it is read from the template's tree (where every @name is a directive) rather than the compiled view.

### `tlint-space-after-blade-directives`

Port of tlint 9.7.0 SpaceAfterBladeDirectives. Flags a control directive whose name is not followed by a space or tab, where Blade compiles it to control flow. Read from the compiled view's if/elseif/for/foreach/while statement whose host is the directive (not the if a @forelse's @empty compiles to). In an attribute value, where Forte does not pair directives into a block and the statement's host is the whole value, it is read from the directive itself where the view reads it as control flow.

### `tlint-spaces-around-blade-render-content`

Port of tlint 9.7.0 SpacesAroundBladeRenderContent. Only single-line echoes; covers {{ }} and {!! !!}, but not {{-- --}} comments or escaped @{{ }} echoes.
