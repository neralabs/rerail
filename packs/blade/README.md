# Rerail rule pack: Blade

17 ports of rules for Laravel Blade templates: Bladestan 0.11.7 (8 PHPStan 2.2.16 rules over the compiled template),
tlint 9.7.0 (5 lints) and stillat/blade-parser 2.1.0 (4 validators). 7 ship: tlint's `NoDumpDirectives` and `NoRayDirective`, blade-parser's
`DebugDirectiveValidator`, `ForElseStructureValidator` and `ComponentShorthandVariableParameterValidator`, and
Bladestan's `if.alwaysTrue` and `identical.alwaysTrue`. The two Bladestan rules start off (`"on": false`): PHPStan
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
| `conformance.json` | Every port (shipped under `rules/`, or a candidate under `candidates/rules/`): its upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools and the `calls` exemption with its evidence |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`), its expected findings (`fixtures.expected.json`; pack test data, derived from the pins and archived in S3: `npm run fixtures:pull`), the generator (`generate.py` with its PHP helpers, sharing `upstream/common.py` and the template capture tlint's and stillat/blade-parser's test readers run, `upstream/blade_capture.php`), and the lock naming its real-code samples (`samples.lock`, fetched by `npm run fixtures:pull`) with their sources and licences (`SAMPLES.md`) |
| `upstream/<tool>/gates.yaml` | The tool's upstream manifest: its pins, approved configurations, the file types it lints and the facts of every rule the pack ports ([docs/upstream.md](../../docs/upstream.md)) |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, per upstream repository: its source, the patterns of the files taken and how (each file's provenance is `provenance.json` in the tool's test-data archive) |

## Candidates

Ten ports stay candidates: seven have no upstream evidence or a template the engine reads only in part, and three
tlint lints miss findings on real templates. The title is what
`pack.json` lists for a candidate once it ships; its group is the tool the title names.

| Candidate | Title |
|---|---|
| `bladestan-variable-undefined` | Bladestan variable.undefined: Undefined variable |
| `bladestan-argument-sprintf` | Bladestan argument.sprintf: sprintf() arguments and format disagree |
| `bladestan-array-duplicate-key` | Bladestan array.duplicateKey: Duplicate array key |
| `bladestan-random-int-parameters` | Bladestan argument.type: random_int() minimum above maximum |
| `bladestan-if-always-false` | Bladestan if.alwaysFalse: If condition always false |
| `bladestan-missing-function-return-type` | Bladestan missingType.return: Function without a return type |
| `blade-parser-duplicate-condition-expressions` | blade-parser DuplicateConditionExpressionsValidator: Duplicate @elseif condition |
| `tlint-no-json-directive` | tlint NoJsonDirective: No @json directive |
| `tlint-space-after-blade-directives` | tlint SpaceAfterBladeDirectives: Space after control structure directives |
| `tlint-spaces-around-blade-render-content` | tlint SpacesAroundBladeRenderContent: One space inside echo delimiters |

The six Bladestan candidates have nothing to match: Bladestan's own test templates hold none of
their findings (`variable.undefined`'s appear only when a template is rendered with its caller's data), so nothing
upstream wrote proves them. `blade-parser-duplicate-condition-expressions` is
`source-incomplete` on two of blade-parser's `DebugDirectiveTest` templates, a malformed `@forelse` and `@foreach` that
blade-parser accepts and Laravel's `BladeCompiler` rejects: the parsers disagree on them and no known engine gap
excuses the rule.

`tlint-no-json-directive`, `tlint-space-after-blade-directives` and `tlint-spaces-around-blade-render-content` match
tlint on its test templates but miss 9, 17 and 10 labelled violations in real Bagisto templates. The engine reads those
templates only in part (`source-partial` or `source-incomplete`), because they embed large Vue and JavaScript script
blocks, and the rules find nothing in the part it does not read. Their patterns are not wrong; they wait for the engine
to read such templates whole.

Each of the six Bladestan candidates, `tlint-no-json-directive` and `tlint-space-after-blade-directives` carried an
example pair while it shipped. The Bladestan tool's fixture findings are empty or all excused, so the pair is what
proved it then; `test/engine/catalog/candidate-examples.test.ts` still checks each pair below through the engine, so
they keep guarding Blade's embedded PHP and directives, and the pairs go back into `pack.json` with their rules:

### `bladestan-variable-undefined`

Caught (`resources/views/orders/index.blade.php`):

```blade
@php
    $money = function ($cents) {
        return $currency . ' ' . number_format($cents / 100, 2);
    };
@endphp
<p>{{ $money($order->total) }}</p>
```

Passes (`resources/views/orders/index.blade.php`):

```blade
@php
    $money = function ($cents) use ($currency) {
        return $currency . ' ' . number_format($cents / 100, 2);
    };
@endphp
<p>{{ $money($order->total) }}</p>
```

### `bladestan-argument-sprintf`

Caught (`resources/views/posts/pager.blade.php`):

```blade
<p>{{ sprintf('Page %d of %d', $page) }}</p>
```

Passes (`resources/views/posts/pager.blade.php`):

```blade
<p>{{ sprintf('Page %d of %d', $page, $pages) }}</p>
```

### `bladestan-array-duplicate-key`

Caught (`resources/views/posts/status.blade.php`):

```blade
@php
    $labels = ['draft' => 'Draft', 'published' => 'Published', 'draft' => 'Pending review'];
@endphp
<span>{{ $labels[$post->status] }}</span>
```

Passes (`resources/views/posts/status.blade.php`):

```blade
@php
    $labels = ['draft' => 'Draft', 'published' => 'Published', 'pending' => 'Pending review'];
@endphp
<span>{{ $labels[$post->status] }}</span>
```

### `bladestan-random-int-parameters`

Caught (`resources/views/auth/captcha.blade.php`):

```blade
<input type="hidden" name="nonce" value="{{ random_int(999999, 100000) }}">
```

Passes (`resources/views/auth/captcha.blade.php`):

```blade
<input type="hidden" name="nonce" value="{{ random_int(100000, 999999) }}">
```

### `bladestan-if-always-false`

Caught (`resources/views/layouts/banner.blade.php`):

```blade
@php
    $showBanner = false;
@endphp
@if ($showBanner)
    <div class="banner">{{ $message }}</div>
@endif
```

Passes (`resources/views/layouts/banner.blade.php`):

```blade
@if ($showBanner)
    <div class="banner">{{ $message }}</div>
@endif
```

### `bladestan-missing-function-return-type`

Caught (`resources/views/users/card.blade.php`):

```blade
@php
    function initials($name)
    {
        return strtoupper(substr($name, 0, 1));
    }
@endphp
<span>{{ initials($user->name) }}</span>
```

Passes (`resources/views/users/card.blade.php`):

```blade
@php
    function initials(string $name): string
    {
        return strtoupper(substr($name, 0, 1));
    }
@endphp
<span>{{ initials($user->name) }}</span>
```

### `tlint-no-json-directive`

Caught (`resources/views/profile.blade.php`):

```blade
<script>
    const user = @json($user);
</script>
```

Passes (`resources/views/profile.blade.php`):

```blade
<script>
    const user = {{ Js::from($user) }};
</script>
```

### `tlint-space-after-blade-directives`

Caught (`resources/views/profile.blade.php`):

```blade
@if($user)
    <p>{{ $user->name }}</p>
@endif
```

Passes (`resources/views/profile.blade.php`):

```blade
@if ($user)
    <p>{{ $user->name }}</p>
@endif
```

Taking a new snapshot is a maintainer deriving the pack's test data from its pins and publishing it
([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)); a pull request carries it only as a publish request
under `upstream/<tool>/publish/`. The recipe (`scripts/pack-data/recipes/blade.ts`) runs each tool's `generate.py`
over its fixtures, against the pinned tool versions, with the pinned PHP 8.5.11 first on `PATH` (each generator's
docstring has the setup):

Paused: `fixtures:derive` refuses every tool-pack until the generators write per-repo provenance
([docs/upstream.md](../../docs/upstream.md#derive-and-bumps-are-paused)). The commands below take a snapshot once the
pause lifts.

```sh
npm run fixtures:derive -- --pack blade                    # every tool
npm run fixtures:derive -- --pack blade --tool bladestan   # one
npm run fixtures:stage -- --pack blade --tool bladestan    # copy what changed into upstream/bladestan/publish/
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack blade --rule tlint-space-after-blade-directives --fixtures
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `blade-parser-duplicate-condition-expressions`

Port of stillat/blade-parser 2.1.0 DuplicateConditionExpressionsValidator. Reports only the first @elseif of an @if chain whose condition, as written (source text), equals an earlier branch's; read from the compiled view, where the chain is one if statement. A later duplicate in the same chain is not reported, because blade-parser stops at the first. The chain is the witness and the @elseif a capture of it, so the rule asks nothing negative and reads a view with syntax errors elsewhere, as blade-parser does.

### `bladestan-array-duplicate-key`

Port of Bladestan 0.11.7 array.duplicateKey (PHPStan 2.2.16 DuplicateKeysInLiteralArraysRule over the compiled template). Reports, as PHPStan does, at the first explicitly keyed element whose key PHP coerces to the same key as a later explicit key's. A key PHP cannot evaluate (e.g. a method call) equals none. Arrays spanning more than 256 lines are not checked: comparing every pair of keys of a larger literal (generated data tables) exceeds the per-file AST work budget, which would fail every policy on the file.

### `bladestan-identical-always-true`

Port of Bladestan 0.11.7 identical.alwaysTrue (PHPStan 2.2.16 StrictComparisonOfDifferentTypesRule over the compiled template). Constant evaluation follows variables through their one reaching binding. The condition of the last @elseif of a chain with no @else is not reported, matching PHPStan's reportAlwaysTrueInLastCondition: false.

### `bladestan-if-always-false`

Port of Bladestan 0.11.7 if.alwaysFalse (PHPStan 2.2.16 IfConstantConditionRule over the compiled template). Constant evaluation follows variables through their one reaching binding. Not reported: comparisons, negations, ternaries, isset and empty (PHPStan leaves those to other rules), a condition inside a trait, or an if preceded by an inline @var doc comment (PHPStan reads the variable as the PHPDoc type).

### `bladestan-if-always-true`

Port of Bladestan 0.11.7 if.alwaysTrue (PHPStan 2.2.16 IfConstantConditionRule over the compiled template). Same exclusions as `bladestan-if-always-false`: comparisons, negations, ternaries, isset and empty, conditions inside a trait, and an if preceded by an inline @var doc comment.

### `bladestan-missing-function-return-type`

Port of Bladestan 0.11.7 missingType.return as PHPStan 2.2.16's MissingFunctionReturnTypehintRule reports it. Only function declarations (not methods or closures). A @return, @phpstan-return or @psalm-return tag in the doc comment counts as a return type.

### `bladestan-random-int-parameters`

Port of Bladestan 0.11.7 argument.type as PHPStan 2.2.16's RandomIntParametersRule reports it over the compiled template. Arguments are bound as PHP binds them (named arguments included); only reported when both $min and $max are constants with $min > $max.

### `bladestan-variable-undefined`

Port of Bladestan 0.11.7 variable.undefined (PHPStan 2.2.16 DefinedVariableRule over the compiled template). A read at the template's top level that the template does not define is view data, which Bladestan reads as "might not be defined" when the render passes none; neither side claims it.

### `tlint-no-json-directive`

Port of tlint 9.7.0 NoJsonDirective. Read from the compiled view, where Blade compiles @json to a call, so it is caught in text, attribute values and script alike. A candidate: on Bagisto it misses labelled violations in templates the engine reads only in part, because they embed large script blocks.

### `tlint-no-ray-directive`

Port of tlint 9.7.0 NoRayDirective. @ray is spatie/laravel-ray's directive, which core Blade does not compile, so it is read from the template's tree (where every @name is a directive) rather than the compiled view.

### `tlint-space-after-blade-directives`

Port of tlint 9.7.0 SpaceAfterBladeDirectives. Flags a control directive whose name is not followed by a space or tab, where Blade compiles it to control flow. Read from the compiled view's if/elseif/for/foreach/while statement whose host is the directive (not the if a @forelse's @empty compiles to). In an attribute value, where Forte does not pair directives into a block and the statement's host is the whole value, it is read from the directive itself where the view reads it as control flow. A candidate: on Bagisto it misses labelled violations in templates the engine reads only in part, because they embed large script blocks.

### `tlint-spaces-around-blade-render-content`

Port of tlint 9.7.0 SpacesAroundBladeRenderContent. Only single-line echoes; covers {{ }} and {!! !!}, but not {{-- --}} comments or escaped @{{ }} echoes. A candidate: on Bagisto it misses labelled violations in templates the engine reads only in part, because they embed large script blocks.
