<?php
// The directives applications register beyond core Blade (`Blade::directive`), by corpus, for
// `generate.php --definedness`: each compiles as the application compiles it, so PHPStan reads the
// template as the application runs it. Each closure is an excerpt of the named upstream source
// (rerail-fixtures/blade/licenses/<project>/UPSTREAM.md); where the upstream closure reads the
// running application (a view path, a key generator), the excerpt writes a fixed value instead,
// which changes no variable the template reads.

declare(strict_types=1);

/** Livewire 3.8.7's directives (livewire/livewire, src/). */
function livewire_directives(): array
{
    return [
        // Mechanisms/RenderComponent.php, `RenderComponent::livewire` (the deterministic key fixed).
        'livewire' => function (string $expression): string {
            $key = null;
            $expression = preg_replace_callback('/,\s*?key\(([\s\S]*)\)/', function ($match) use (&$key) {
                $key = trim($match[1]) ?: $key;
                return '';
            }, $expression);
            $key ??= 'null';
            return <<<EOT
<?php
\$__split = function (\$name, \$params = []) {
    return [\$name, \$params];
};
[\$__name, \$__params] = \$__split($expression);

\$__key = $key;

\$__key ??= \Livewire\Features\SupportCompiledWireKeys\SupportCompiledWireKeys::generateKey('lw-0', \$__key);

\$__html = app('livewire')->mount(\$__name, \$__params, \$__key);

echo \$__html;

unset(\$__html);
unset(\$__key);
unset(\$__name);
unset(\$__params);
unset(\$__split);
if (isset(\$__slots)) unset(\$__slots);
?>
EOT;
        },
        // Mechanisms/ExtendBlade/ExtendBlade.php.
        'this' => fn () => "window.Livewire.find('{{ \$_instance->getId() }}')",
        // Mechanisms/FrontendAssets/FrontendAssets.php.
        'livewireScripts' => fn ($expression) => '{!! \Livewire\Mechanisms\FrontendAssets\FrontendAssets::scripts(' . $expression . ') !!}',
        'livewireScriptConfig' => fn ($expression) => '{!! \Livewire\Mechanisms\FrontendAssets\FrontendAssets::scriptConfig(' . $expression . ') !!}',
        'livewireStyles' => fn ($expression) => '{!! \Livewire\Mechanisms\FrontendAssets\FrontendAssets::styles(' . $expression . ') !!}',
        // Features/SupportScriptsAndAssets/SupportScriptsAndAssets.php (the compile-time key fixed).
        'script' => fn () => "<?php\n    \$__scriptKey = 'script-0';\n    ob_start();\n?>",
        'endscript' => fn () => "<?php\n    \$__output = ob_get_clean();\n\n    \Livewire\store(\$this)->push('scripts', \$__output, \$__scriptKey)\n?>",
        // Features/SupportEntangle/SupportEntangle.php.
        'entangle' => fn ($expression) => "<?php if ((object) ({$expression}) instanceof \Livewire\WireDirective) : ?>window.Livewire.find('{{ \$__livewire->getId() }}').entangle('{{ {$expression}->value() }}'){{ {$expression}->hasModifier('live') ? '.live' : '' }}<?php else : ?>window.Livewire.find('{{ \$__livewire->getId() }}').entangle('{{ {$expression} }}')<?php endif; ?>",
        // Features/SupportNavigate/SupportNavigate.php.
        'persist' => fn ($expression) => '<?php app("livewire")->forceAssetInjection(); ?><div x-persist="<?php echo e(' . $expression . '); ?>">',
        'endpersist' => fn () => '</div>',
        // Features/SupportTeleporting/SupportTeleporting.php.
        'teleport' => fn ($expression) => '<template x-teleport="<?php echo e(' . $expression . '); ?>">',
        'endteleport' => fn () => '</template>',
    ];
}

/**
 * The directives each corpus's application registers, by corpus name.
 *
 * @return array<string, \Closure>
 */
function corpus_directives(string $corpus): array
{
    return match ($corpus) {
        'core' => [],
        // BookStackApp/BookStack: app/App/Providers/ViewTweaksServiceProvider.php (`icon`) and
        // ThemeServiceProvider.php (`include`, which replaces Blade's own).
        'bookstack' => [
            'icon' => fn ($expression) => "<?php echo (new \BookStack\Util\SvgIcon($expression))->toHtml(); ?>",
            'include' => fn ($expression) => "<?php echo \$__themeViews->handleViewInclude({$expression}, array_diff_key(get_defined_vars(), ['__data' => 1, '__path' => 1])); ?>",
        ],
        // filamentphp/filament: packages/support/src/SupportServiceProvider.php, on Livewire.
        'filament' => livewire_directives() + [
            'capture' => function (string $expression): string {
                [$name, $arguments] = str_contains($expression, ',')
                    ? array_map('trim', explode(',', $expression, 2))
                    : [$expression, ''];
                return "\n                <?php {$name} = (function (\$args) {\n                    return function ({$arguments}) use (\$args) {\n                        extract(\$args, EXTR_SKIP);\n                        ob_start(); ?>\n            ";
            },
            'endcapture' => fn () => "\n                <?php return new \Illuminate\Support\HtmlString(ob_get_clean()); };\n                    })(get_defined_vars()); ?>\n            ",
            'captureSlots' => fn (string $expression) => "<?php \$slotContents = get_defined_vars(); \$slots = collect({$expression})->mapWithKeys(fn (string \$slot): array => [\$slot => \$slotContents[\$slot] ?? null])->all(); unset(\$slotContents) ?>",
            'filamentScripts' => fn (string $expression) => "<?php echo \Filament\Support\Facades\FilamentAsset::renderScripts({$expression}) ?>",
            'filamentStyles' => fn (string $expression) => "<?php echo \Filament\Support\Facades\FilamentAsset::renderStyles({$expression}) ?>",
        ],
        // bagisto/bagisto: packages/Webkul/Theme/src/Providers/ThemeServiceProvider.php.
        'bagisto' => [
            'bagistoVite' => fn ($expression) => "<?php echo themes()->setBagistoVite({$expression})->toHtml(); ?>",
        ],
    };
}
