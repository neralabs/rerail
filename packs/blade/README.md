# Rerail rule pack: Blade

7 rules for Laravel Blade templates, ported to Rerail: tlint 9.7.0's `NoDumpDirectives` and `NoRayDirective`,
stillat/blade-parser 2.1.0's `DebugDirectiveValidator`, `ForElseStructureValidator` and
`ComponentShorthandVariableParameterValidator`, and Bladestan 0.11.7's `if.alwaysTrue` and `identical.alwaysTrue`
(PHPStan 2.2.16 rules over the compiled template). 10 more ports are candidates
([candidates/README.md](candidates/README.md)). The two Bladestan rules start off (`"on": false`): PHPStan
registers them at level 4 and Bladestan's recommended level is 0, so they are experimental. A port starts in
`candidates/rules/` and moves to `rules/` and `pack.json` once it passes the conformance check
([conformance/README.md](../../conformance/README.md#the-gate)); each of the pack's three tools is gated. Every port is
scored against the tool's own findings on the tool's test fixtures, and against the labelled lines of the tool's
real-code samples ([docs/upstream.md](../../docs/upstream.md#samples)), through the engine's production workspace
path, with every disagreement declared in `divergences.json`. The conformance check (`--check`, run in CI) scores the
fixtures and the samples: it gates the ports in `rules/` and reports the candidates without gating them. None of the
three tools is ECMAScript, so the fixtures carry no rename variants (`holds-under-rename`); the real-code samples are
the pack's only evidence that a port is not fitted to its tests.

It is checked by `npx vitest run test/engine/catalog/packs.test.ts -t "pack blade"` ([pack structure and validation
contract](../../docs/pack-format.md)).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): its match definition, each rule's document, title, group and, for a rule that starts off, `on` |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `candidates/rules/` | The candidates' code-rule documents (`rerail.policy/v1`), scored but never shipped |
| `candidates/README.md` | What each candidate is waiting on and its rule details; not published |
| `conformance.json` | Every port (shipped under `rules/`, or a candidate under `candidates/rules/`): its upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools and the `calls` exemption with its evidence |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`), its expected findings (`fixtures.expected.json`; pack test data, derived from the pins and archived in S3: `npm run fixtures:pull`), the generator (`generate.py` with its PHP helpers, sharing `upstream/common.py` and the template capture tlint's and stillat/blade-parser's test readers run, `upstream/blade_capture.php`), and the lock naming its real-code samples (`samples.lock`, fetched by `npm run fixtures:pull`) with their sources and licences (`SAMPLES.md`) |
| `upstream/<tool>/gates.yaml` | The tool's upstream manifest: its pins, approved configurations, the file types it lints and the facts of every rule the pack ports ([docs/upstream.md](../../docs/upstream.md)) |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, per upstream repository: its source, the patterns of the files taken and how (each file's provenance is `provenance.json` in the tool's test-data archive) |

Taking a new snapshot is a maintainer deriving the pack's test data from its pins and publishing it
([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)); a pull request carries it only as a publish request
under `upstream/<tool>/publish/`. The recipe (`scripts/pack-data/recipes/blade.ts`) runs each tool's `generate.py`
over its fixtures, against the pinned tool versions, with the pinned PHP 8.5.11 first on `PATH` (each generator's
docstring has the setup):

```sh
npm run fixtures:derive -- --pack blade                    # every tool
npm run fixtures:derive -- --pack blade --tool bladestan   # one
npm run fixtures:stage -- --pack blade --tool bladestan    # copy what changed into upstream/bladestan/publish/
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack blade --rule tlint-no-dump-directives --fixtures
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `bladestan-identical-always-true`

Port of Bladestan 0.11.7 identical.alwaysTrue (PHPStan 2.2.16 StrictComparisonOfDifferentTypesRule over the compiled template). Constant evaluation follows variables through their one reaching binding. The condition of the last @elseif of a chain with no @else is not reported, matching PHPStan's reportAlwaysTrueInLastCondition: false.

### `bladestan-if-always-true`

Port of Bladestan 0.11.7 if.alwaysTrue (PHPStan 2.2.16 IfConstantConditionRule over the compiled template). Not reported: comparisons, negations, ternaries, isset and empty, conditions inside a trait, and an if preceded by an inline @var doc comment.

### `tlint-no-ray-directive`

Port of tlint 9.7.0 NoRayDirective. @ray is spatie/laravel-ray's directive, which core Blade does not compile, so it is read from the template's tree (where every @name is a directive) rather than the compiled view.
