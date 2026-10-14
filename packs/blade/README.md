# Rerail rule pack: Blade

17 ports of rules for Laravel Blade templates: Bladestan 0.11.7 (8 PHPStan 2.2.16 rules over the compiled template),
tlint 9.7.0 (5 lints) and stillat/blade-parser 2.1.0 (4 validators). A port starts in `candidates/rules/` and moves to
`rules/` and `pack.json` once it passes the V1 rule validation gates
([nera-rule-validation](https://github.com/neralabs/nera-rule-validation)). Every port is a candidate today, so
`pack.json` lists no rules: the pack ships none and is never recommended. Every port is scored against the tool's own
findings on the tool's test fixtures and on three pinned corpora, through the engine's production workspace path, with
every disagreement declared in `divergences.json`. The conformance check (`--check`, run in CI) scores the fixtures: it
gates the ports in `rules/` and reports the candidates without gating them.

It is checked by `npx vitest run test/engine/catalog/packs.test.ts -t "pack blade"` ([pack structure and validation
contract](../../docs/pack-format.md)).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): its match definition, and no rules until a port passes |
| `candidates/rules/` | The candidates' code-rule documents (`rerail.policy/v1`), scored but never shipped |
| `conformance.json` | Every port (shipped under `rules/`, or a candidate under `candidates/rules/`): its upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools, the pinned corpora (BookStack, Filament, Bagisto) and the `calls` exemption with its evidence |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-<name>.expected.json`) and the recorder (`generate.py` with its PHP helpers, sharing `upstream/common.py` and `upstream/capture.php`) |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, listing every copied file |

## Candidates

The title is what `pack.json` lists for a candidate once it ships; its group is the tool the title names.

| Candidate | Title |
|---|---|
| `bladestan-variable-undefined` | Bladestan variable.undefined: Undefined variable |
| `bladestan-argument-sprintf` | Bladestan argument.sprintf: sprintf() arguments and format disagree |
| `bladestan-array-duplicate-key` | Bladestan array.duplicateKey: Duplicate array key |
| `bladestan-random-int-parameters` | Bladestan argument.type: random_int() minimum above maximum |
| `bladestan-if-always-true` | Bladestan if.alwaysTrue: If condition always true |
| `bladestan-if-always-false` | Bladestan if.alwaysFalse: If condition always false |
| `bladestan-identical-always-true` | Bladestan identical.alwaysTrue: Strict comparison always true |
| `bladestan-missing-function-return-type` | Bladestan missingType.return: Function without a return type |
| `tlint-no-json-directive` | tlint NoJsonDirective: No @json directive |
| `tlint-no-dump-directives` | tlint NoDumpDirectives: No @dd or @dump directive |
| `tlint-no-ray-directive` | tlint NoRayDirective: No @ray directive |
| `tlint-space-after-blade-directives` | tlint SpaceAfterBladeDirectives: Space after control structure directives |
| `tlint-spaces-around-blade-render-content` | tlint SpacesAroundBladeRenderContent: One space inside echo delimiters |
| `blade-parser-debug-directive` | blade-parser DebugDirectiveValidator: No debug directive |
| `blade-parser-duplicate-condition-expressions` | blade-parser DuplicateConditionExpressionsValidator: Duplicate @elseif condition |
| `blade-parser-for-else-structure` | blade-parser ForElseStructureValidator: @forelse with one @empty |
| `blade-parser-component-shorthand-variable-parameter` | blade-parser ComponentShorthandVariableParameterValidator: Malformed component shorthand attribute |

Eight candidates carried an example pair while they shipped, checked through the engine. For six of them
(`bladestan-variable-undefined`, `bladestan-argument-sprintf`, `bladestan-array-duplicate-key`,
`bladestan-random-int-parameters`, `bladestan-if-always-false`, `bladestan-missing-function-return-type`) the tool's
fixture findings are empty or all excused, so the pair is what proved them while they shipped; the pairs go back
into `pack.json` with their rules:

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

Re-recording runs offline against the pinned tool versions, with PHP 8.5.11 (each recorder's docstring has the setup):

```sh
python3 upstream/bladestan/generate.py fixtures <bladestan checkout at 0.11.7>
python3 upstream/tlint/generate.py fixtures <tlint checkout at v9.7.0> <composer project with tightenco/tlint 9.7.0>
python3 upstream/blade-parser/generate.py fixtures <blade-parser checkout at v2.1.0> <composer project with stillat/blade-parser 2.1.0>
python3 upstream/bladestan/generate.py corpus <bladestan checkout> bookstack=<checkout> filament=<checkout> bagisto=<checkout>
python3 upstream/tlint/generate.py corpus <tlint composer project> bookstack=<checkout> filament=<checkout> bagisto=<checkout>
python3 upstream/blade-parser/generate.py corpus <blade-parser composer project> bookstack=<checkout> filament=<checkout> bagisto=<checkout>
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack blade --rule tlint-space-after-blade-directives --fixtures
```
