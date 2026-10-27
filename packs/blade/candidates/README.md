# Rerail rule pack: Blade, candidates

The ports in `rules/` here are still being evaluated: scored and reported against the tools' recordings, never gated
and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

## Candidates

Seven ports stay candidates: they fail `matches-upstream-tests`, a blocking port readiness gate. The title is what
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

The six Bladestan candidates fail `matches-upstream-tests` with `fires`: Bladestan's own test templates hold none of
their findings (`variable.undefined`'s appear only when a template is rendered with its caller's data), so nothing
upstream wrote proves them, and they wait for goldens. `blade-parser-duplicate-condition-expressions` is
`source-incomplete` on two of blade-parser's `DebugDirectiveTest` templates, a malformed `@forelse` and `@foreach` that
blade-parser accepts and Laravel's `BladeCompiler` rejects: the parsers disagree on them and no known engine gap
excuses the rule.

Each of the six Bladestan candidates carried an example pair while it shipped. The tool's fixture findings are empty
or all excused, so the pair is what proved it then; `test/engine/catalog/candidate-examples.test.ts` still checks each
pair below through the engine, so they keep guarding Blade's embedded PHP, and the pairs go back into `pack.json` with
their rules:

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

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `blade-parser-duplicate-condition-expressions`

Port of stillat/blade-parser 2.1.0 DuplicateConditionExpressionsValidator. Reports only the first @elseif of an @if chain whose condition, as written (source text), equals an earlier branch's; read from the compiled view, where the chain is one if statement. A later duplicate in the same chain is not reported, because blade-parser stops at the first. The chain is the witness and the @elseif a capture of it, so the rule asks nothing negative and reads a view with syntax errors elsewhere, as blade-parser does.

### `bladestan-array-duplicate-key`

Port of Bladestan 0.11.7 array.duplicateKey (PHPStan 2.2.16 DuplicateKeysInLiteralArraysRule over the compiled template). Reports, as PHPStan does, at the first explicitly keyed element whose key PHP coerces to the same key as a later explicit key's. A key PHP cannot evaluate (e.g. a method call) equals none. Arrays spanning more than 256 lines are not checked: comparing every pair of keys of a larger literal (generated data tables) exceeds the per-file AST work budget, which would fail every policy on the file.

### `bladestan-if-always-false`

Port of Bladestan 0.11.7 if.alwaysFalse (PHPStan 2.2.16 IfConstantConditionRule over the compiled template). Constant evaluation follows variables through their one reaching binding. Not reported: comparisons, negations, ternaries, isset and empty (PHPStan leaves those to other rules), a condition inside a trait, or an if preceded by an inline @var doc comment (PHPStan reads the variable as the PHPDoc type).

### `bladestan-missing-function-return-type`

Port of Bladestan 0.11.7 missingType.return as PHPStan 2.2.16's MissingFunctionReturnTypehintRule reports it. Only function declarations (not methods or closures). A @return, @phpstan-return or @psalm-return tag in the doc comment counts as a return type.

### `bladestan-random-int-parameters`

Port of Bladestan 0.11.7 argument.type as PHPStan 2.2.16's RandomIntParametersRule reports it over the compiled template. Arguments are bound as PHP binds them (named arguments included); only reported when both $min and $max are constants with $min > $max.

### `bladestan-variable-undefined`

Port of Bladestan 0.11.7 variable.undefined (PHPStan 2.2.16 DefinedVariableRule over the compiled template). A read at the template's top level that the template does not define is view data, which Bladestan reads as "might not be defined" when the render passes none; neither side claims it.
