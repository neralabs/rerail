<?php
// Writes blade.expected.json: for every fixtures/*.blade.php template, what illuminate/view 13.34.0's
// BladeCompiler compiles it to, on PHP 8.5.11 (with ramsey/uuid 4.9.4 installed beside illuminate/view,
// which `@once` and `@pushOnce` need), read three ways:
//   - `regions`: each piece of template PHP Blade compiles (an echo's expression, a directive's
//     arguments, a `@php` block, a `<?php` tag, a component's bound attribute), with its span in the
//     template and the template bytes Blade copies verbatim into the compiled PHP (`copied`);
//   - `phpstan`: PHPStan 2.2.16's `variable.undefined` reports (level max) on the compiled template run
//     as Blade's PhpEngine runs it: inside a function that `extract()`s the view's data, with Blade's
//     `$__env` and `$errors` as parameters. Each report is placed at the template offsets of the
//     variable reads Blade copied onto the reported line (`at`). A fixture PHPStan cannot analyse
//     through (a parse error, an internal error) fails the run;
//   - `values`: what PHP makes of each region that is an expression without variables, as a constant
//     expression (`const`) and at run time (`eval`), each in a fresh `php -n` process, in the
//     encoding of php/value/generate_php.php.
// Spans come from marker comments (`/*T<offset>*/`) put into a copy of the template where Blade
// copies text into PHP: after a directive's `(`, an echo's opening tag, `@php` and `<?php`, and a bound
// attribute's opening quote; and before the `@` of the directives Blade reads as text (`@use`,
// `@inject`, `@break`, `@continue`). Blade compiles the marked copy to the same PHP as the template
// (checked: each region's text without markers is the same), and each region starts at the offset
// its first marker names. `copied` aligns the region's PHP tokens with the compiled PHP's (longest
// common subsequence, whitespace dropped); a quoted string whose contents Blade writes as a name
// (`@use('A\B')` is `use \A\B;`) copies its contents.
// Run offline, not in CI:
//   php generate.php <composer project requiring illuminate/view:13.34.0 and ramsey/uuid:4.9.4> <composer project requiring phpstan/phpstan:2.2.16>
// Region parity over a corpus (no PHPStan, no values), for benchmark_blade_region_parity:
//   php generate.php <illuminate/view project> --corpus <output.json> <root> [<root> ...]
// Definedness over a corpus (PHPStan's `variable.undefined` reports placed as for the fixtures), for
// benchmark_blade_definedness_corpus, with the directives the corpus's application registers
// (directives.php) compiled as it compiles them:
//   php generate.php <illuminate/view project> --definedness <phpstan project> <output.json> <directive set> <root> [<root> ...]

declare(strict_types=1);

use Composer\InstalledVersions;
use Illuminate\Filesystem\Filesystem;
use Illuminate\View\Compilers\BladeCompiler;
use Illuminate\View\Compilers\ComponentTagCompiler;

require $argv[1] . '/vendor/autoload.php';
require __DIR__ . '/directives.php';

/** What the compiler hooks below saw, in the order Blade compiled it. */
final class Seen
{
    /** @var list<array{kind: string, directive: ?string, text: string, compiled: string, whole?: string}> */
    public static array $regions = [];
}

/** Anonymous components by name: no view factory needed. */
final class RerailTags extends ComponentTagCompiler
{
    public function componentClass(string $component)
    {
        return "components.$component";
    }

    protected function getAttributesFromAttributeString(string $attributeString)
    {
        $attributes = parent::getAttributesFromAttributeString($attributeString);
        // The attribute string as Blade rewrote it before reading it: bound attributes are `bind:name=`.
        $rewritten = $this->parseBindAttributes($this->parseComponentTagStyleStatements(
            $this->parseComponentTagClassStatements($this->parseAttributeBag(
                $this->parseShortAttributeSyntax($attributeString)))));
        foreach ($attributes as $name => $value) {
            // A bound attribute written with a value (one written without is `true`, not template PHP).
            if (isset($this->boundAttributes[$name])
                && preg_match('/(^|\s)bind:' . preg_quote((string) $name, '/') . '=/', $rewritten)) {
                Seen::$regions[] = ['kind' => 'attribute', 'directive' => null, 'text' => $value, 'compiled' => $value];
            }
        }
        return $attributes;
    }
}

final class RerailBlade extends BladeCompiler
{
    /** @var array<string, \Closure> The application's directives (directives.php), by name. */
    public static array $directives = [];

    public function __construct()
    {
        parent::__construct(new Filesystem(), sys_get_temp_dir());
        // `@component` pushes onto this static stack and `@endcomponent` does not pop it, so one
        // template's compile would shift the hashes of the next's: each compile starts afresh.
        static::$componentHashStack = [];
        foreach (self::$directives as $name => $handler) {
            $this->directive($name, $handler);
        }
        // Runs last, on the text Blade tokenizes: the `<?php` tags the template spells.
        $this->precompiler(function (string $value): string {
            $open = null;
            $text = '';
            foreach (token_get_all($value) as $token) {
                [$id, $content] = is_array($token) ? $token : [null, $token];
                if ($id === T_OPEN_TAG || $id === T_OPEN_TAG_WITH_ECHO) {
                    [$open, $text] = [$id, ''];
                } elseif ($id === T_CLOSE_TAG && $open !== null) {
                    $this->tag($open, $text);
                    $open = null;
                } elseif ($open !== null) {
                    $text .= $content;
                }
            }
            if ($open !== null) {
                $this->tag($open, $text);
            }
            return $value;
        });
    }

    private function tag(int $open, string $text): void
    {
        // The tag's PHP is copied as it is: it is its own compiled text.
        Seen::$regions[] = ['kind' => $open === T_OPEN_TAG_WITH_ECHO ? 'php_echo' : 'php_tag', 'directive' => null, 'text' => $text, 'compiled' => $text];
    }

    protected function compileComponentTags($value)
    {
        if (! $this->compilesComponentTags) {
            return $value;
        }
        return (new RerailTags($this->classComponentAliases, $this->classComponentNamespaces, $this))->compile($value);
    }

    protected function wrapInEchoHandler($value)
    {
        $wrapped = parent::wrapInEchoHandler($value);
        Seen::$regions[] = ['kind' => 'echo', 'directive' => null, 'text' => (string) $value, 'compiled' => (string) $wrapped];
        return $wrapped;
    }

    protected function storeRawBlock($value)
    {
        if (str_starts_with($value, '<?php') && str_ends_with($value, '?>')) {
            Seen::$regions[] = ['kind' => 'php_block', 'directive' => null, 'text' => substr($value, 5, -2), 'compiled' => $value];
        }
        return parent::storeRawBlock($value);
    }

    protected function compileStatement($match)
    {
        $footer = count($this->footer);
        $compiled = parent::compileStatement($match);
        if (! str_contains($match[1], "@") && isset($match[3])) {
            // `@extends` compiles into the footer.
            $compiled .= implode('', array_slice($this->footer, $footer));
            Seen::$regions[] = [
                'kind' => isset($this->customDirectives[$match[1]]) ? 'custom'
                    : (method_exists($this, 'compile' . ucfirst($match[1])) ? 'directive' : 'text'),
                'directive' => $match[1],
                // `$match[3]` is the arguments with their parentheses; `$match[4]`, as the
                // statement compiler extends it to the closing `)`, may miss inner `)`s.
                'text' => substr($match[3], 1, -1),
                'compiled' => $compiled,
                'whole' => $match[0],
            ];
        }
        return $compiled;
    }
}

/** Directives Blade reads its arguments of as text: a marker before the `@`, not inside. */
const TEXTUAL = ['use', 'inject', 'break', 'continue'];

/**
 * The template with markers at every place Blade may copy PHP from, and the offsets of short bound
 * attributes (`:$name`).
 *
 * @return array{string, list<array{int, string}>}
 */
function mark(string $template): array
{
    $insert = [];
    $short = [];
    $bags = [];
    $length = strlen($template);
    // Blade's `@php ... @endphp` blocks (`storePhpBlocks`), which a `@php (` may open too.
    preg_match_all('/(?<!@)@php(.*?)@endphp/s', $template, $blocks, PREG_OFFSET_CAPTURE);
    $blockStarts = array_flip(array_column($blocks[0], 1));
    $word = fn (int $at) => $at >= 0 && $at < $length && (ctype_alnum($template[$at]) || $template[$at] === '_');
    $after_space = function (int $at) use ($template, $length): int {
        while ($at < $length && ctype_space($template[$at])) {
            $at++;
        }
        return $at;
    };
    for ($at = 0; $at < $length; $at++) {
        $rest = substr($template, $at, 6);
        $escaped = $at > 0 && $template[$at - 1] === '@';
        if (str_starts_with($rest, '{{--')) {
            continue;
        }
        if (isset($bags[$at])) {
            // `{{ $attributes... }}` in a component tag: Blade reads it as a bound attribute, by
            // its text, which a marker would hide.
            $at++;
            continue;
        }
        if (str_starts_with($rest, '{{{') || str_starts_with($rest, '{!!') || str_starts_with($rest, '{{')) {
            $open = str_starts_with($rest, '{{') && ! str_starts_with($rest, '{{{') ? 2 : 3;
            $start = $after_space($at + $open);
            if (! $escaped) {
                $insert[$start] = "/*T$start*/";
            } elseif ($open === 2) {
                $insert[$start] = "/*E$start*/";
            }
            $at += $open - 1;
            continue;
        }
        if (str_starts_with($rest, '<?php') || str_starts_with($rest, '<?=')) {
            $start = $after_space($at + (str_starts_with($rest, '<?=') ? 3 : 5));
            $insert[$start] = "/*T$start*/";
            continue;
        }
        if ($template[$at] === '@' && ! $escaped && ! $word($at - 1) && preg_match('/\G@(\w+(?:::\w+)?)([ \t]*)(\()?/', $template, $m, 0, $at)) {
            $name = $m[1];
            if ($name === 'php' && (! isset($m[3]) || isset($blockStarts[$at]))) {
                $start = $after_space($at + 4);
                $insert[$start] = "/*T$start*/";
            } elseif (isset($m[3])) {
                if (in_array($name, TEXTUAL, true)) {
                    $insert[$at] = "/*B$at*/";
                } else {
                    // An application's directive reads its arguments trimmed: a marker before
                    // their leading whitespace would keep it.
                    $start = isset(RerailBlade::$directives[$name])
                        ? $after_space($at + strlen($m[0]))
                        : $at + strlen($m[0]);
                    $insert[$start] = "/*T$start*/";
                }
            }
            $at += strlen($m[0]) - 1;
            continue;
        }
        if (preg_match('/\G<x[-:][\w\-:.]*/', $template, $m, 0, $at)) {
            // The tag's attributes, up to its unquoted `>` outside `{{ }}`.
            $quote = null;
            for ($end = $at + strlen($m[0]); $end < $length; $end++) {
                $char = $template[$end];
                if ($quote !== null) {
                    $quote = $char === $quote ? null : $quote;
                } elseif ($char === '"' || $char === "'") {
                    $quote = $char;
                } elseif ($char === '{' && substr($template, $end, 2) === '{{' && ($close = strpos($template, '}}', $end)) !== false) {
                    $end = $close + 1;
                } elseif ($char === '>') {
                    break;
                }
            }
            $tag = substr($template, $at, $end - $at);
            // ComponentTagCompiler::parseAttributeBag's pattern.
            preg_match_all('/(?:^|\s+)\{\{\s*(\$attributes(?:[^}]+?(?<!\s))?)\s*\}\}/', $tag, $found, PREG_OFFSET_CAPTURE);
            foreach ($found[0] as $index => [$whole, $offset]) {
                $bags[$at + $offset + strpos($whole, '{{')] = true;
                $short[] = [$at + $found[1][$index][1], $found[1][$index][0]];
            }
            preg_match_all('/\s:(?!:)[\w\-:.]+=(["\'])/', $tag, $bound, PREG_OFFSET_CAPTURE);
            foreach ($bound[1] as [$quoteText, $offset]) {
                $start = $at + $offset + 1;
                $insert[$start] = "/*T$start*/";
            }
            preg_match_all('/\s:\$(\w+)/', $tag, $shorts, PREG_OFFSET_CAPTURE);
            foreach ($shorts[1] as [$name, $offset]) {
                $short[] = [$at + $offset - 1, '$' . $name];
            }
            $at += strlen($m[0]) - 1;
        }
    }
    ksort($insert);
    $marked = '';
    $from = 0;
    foreach ($insert as $offset => $marker) {
        $marked .= substr($template, $from, $offset - $from) . $marker;
        $from = $offset;
    }
    return [$marked . substr($template, $from), $short];
}

function strip(string $text): string
{
    return preg_replace('#/\*[TEB]\d+\*/#', '', $text);
}

/** @return array{string, list<array<string, mixed>>} The compiled PHP and what the hooks saw. */
function compile(string $template): array
{
    Seen::$regions = [];
    $compiled = (new RerailBlade())->compileString($template);
    return [$compiled, Seen::$regions];
}

/**
 * PHP tokens of `$code` (prefixed `<?php ` when it has no tag) other than whitespace, markers, tags
 * and inline HTML, as [text, offset in `$code`, id].
 *
 * @return list<array{string, int, int}>
 */
function tokens(string $code): array
{
    $prefix = str_contains($code, '<?php') || str_contains($code, '<?=') ? '' : '<?php ';
    $out = [];
    $at = -strlen($prefix);
    foreach (token_get_all($prefix . $code) as $token) {
        [$id, $text] = is_array($token) ? $token : [0, $token];
        if (! in_array($id, [T_WHITESPACE, T_OPEN_TAG, T_OPEN_TAG_WITH_ECHO, T_CLOSE_TAG, T_INLINE_HTML], true)
            && ! preg_match('#^/\*[TEB]\d+\*/$#', $text)) {
            $out[] = [$text, $at, $id];
        }
        $at += strlen($text);
    }
    return $out;
}

/**
 * Pairs of (region token index, compiled token index) Blade copies, and for each region token the
 * template range it copies.
 *
 * @param list<array{string, int, int}> $region
 * @param list<array{string, int, int}> $compiled
 * @return list<array{int, int, array{int, int}}>
 */
function align(array $region, array $compiled): array
{
    $copies = function (array $from, array $to): ?array {
        if ($from[0] === $to[0]) {
            return [0, strlen($from[0])];
        }
        // Blade writes the contents of a quoted name, and a name, fully qualified (`@use`).
        if ($from[2] === T_CONSTANT_ENCAPSED_STRING && strlen($from[0]) >= 2) {
            $contents = substr($from[0], 1, -1);
            if ($contents !== '' && ($to[0] === $contents || $to[0] === '\\' . $contents)) {
                return [1, strlen($from[0]) - 1];
            }
        }
        if (in_array($from[2], [T_STRING, T_NAME_QUALIFIED], true) && $to[0] === '\\' . $from[0]) {
            return [0, strlen($from[0])];
        }
        return null;
    };
    [$n, $m] = [count($region), count($compiled)];
    $best = array_fill(0, $n + 1, array_fill(0, $m + 1, 0));
    for ($i = $n - 1; $i >= 0; $i--) {
        for ($j = $m - 1; $j >= 0; $j--) {
            $best[$i][$j] = $copies($region[$i], $compiled[$j]) !== null
                ? $best[$i + 1][$j + 1] + 1
                : max($best[$i + 1][$j], $best[$i][$j + 1]);
        }
    }
    $pairs = [];
    for ([$i, $j] = [0, 0]; $i < $n && $j < $m;) {
        $range = $copies($region[$i], $compiled[$j]);
        if ($range !== null && $best[$i][$j] === $best[$i + 1][$j + 1] + 1) {
            $pairs[] = [$i, $j, $range];
            [$i, $j] = [$i + 1, $j + 1];
        } elseif ($best[$i + 1][$j] >= $best[$i][$j + 1]) {
            $i++;
        } else {
            $j++;
        }
    }
    return $pairs;
}

/**
 * Every region of `$template`: where it is, what Blade copies, and where the copies of its variables
 * land in `$compiled` (the compiled marked template), as [compiled offset of the region's compiled
 * text, compiled token offset, template offset, name].
 *
 * @return array{list<array<string, mixed>>, list<array<string, mixed>>, list<array{string, int, int, string}>}
 */
function regions(string $template, string $marked, array $short, string $compiled, array $seen): array
{
    $located = [];
    $unlocated = [];
    $variables = [];
    $textual = [];
    foreach ($seen as $region) {
        $text = $region['text'];
        $kind = $region['kind'];
        $trimmed = ltrim($text);
        $start = null;
        $search = $region['compiled'];
        if (preg_match('#^/\*E\d+\*/#', $trimmed)) {
            continue; // `@{{ }}` is text.
        }
        if ($kind !== 'attribute' && $kind !== 'echo' && $kind !== 'text' && $region['directive'] !== null
            && in_array($region['directive'], TEXTUAL, true)) {
            // Found by the marker before the `@`, which the compiled statement follows.
            $key = $region['compiled'];
            $textual[$key] ??= 0;
            if (preg_match_all('#/\*B(\d+)\*/' . preg_quote($key, '#') . '#', $compiled, $found) > $textual[$key]) {
                $at = (int) $found[1][$textual[$key]];
                $whole = strip($region['whole']);
                $start = $at + strpos($whole, '(') + 1;
                $search = $found[0][$textual[$key]];
            }
            $textual[$key]++;
            $text = strip($text);
        } elseif (preg_match('#^/\*T(\d+)\*/#', $trimmed, $m)) {
            $start = (int) $m[1];
            $text = strip($trimmed);
        } elseif ($kind === 'attribute'
            && preg_match('#^\\\\Illuminate\\\\Support\\\\Arr::toCss(?:Classes|Styles)\(/\*T(\d+)\*/(.*)\)$#s', $trimmed, $m)) {
            // `@class(...)` and `@style(...)` in a component's tag: a bound attribute whose value
            // calls `Arr` with the arguments, their `"` written `'`, and so the template's bytes
            // where only those differ.
            $start = (int) $m[1];
            $text = substr($template, $start, strlen(strip($m[2])));
        } else {
            $text = strip($text);
            // A short bound attribute (`:$name`) or a tag's attribute bag (`{{ $attributes }}`).
            if ($kind === 'attribute') {
                foreach ($short as $index => [$at, $name]) {
                    if ($name === $text) {
                        $start = $at;
                        unset($short[$index]);
                        break;
                    }
                }
            }
        }
        $stripped = strip($text);
        if ($start === null || substr($template, $start, strlen($stripped)) !== $stripped) {
            // Text Blade wrote itself (a component's call) holds no marker and is not the template's.
            if (str_contains($template, trim($stripped))) {
                $unlocated[] = ['kind' => $kind, 'directive' => $region['directive'], 'text' => $stripped];
            }
            continue;
        }
        $regionTokens = tokens($stripped);
        $compiledTokens = $kind === 'text' ? [] : tokens($region['compiled']);
        $copied = [];
        foreach (align($regionTokens, $compiledTokens) as [$i, $j, [$from, $to]]) {
            $copied[] = [$start + $regionTokens[$i][1] + $from, $start + $regionTokens[$i][1] + $to];
            if ($regionTokens[$i][2] === T_VARIABLE) {
                $variables[] = [$search, $compiledTokens[$j][1] + (strlen($search) - strlen($region['compiled'])), $start + $regionTokens[$i][1], $regionTokens[$i][0]];
            }
        }
        $located[] = [
            'kind' => $kind,
            'directive' => $region['directive'],
            'span' => [$start, $start + strlen($stripped)],
            'text' => $stripped,
            'copied' => merge($copied),
        ];
    }
    usort($located, fn ($a, $b) => $a['span'] <=> $b['span']);
    return [$located, $unlocated, $variables];
}

/** @param list<array{int, int}> $ranges */
function merge(array $ranges): array
{
    sort($ranges);
    $out = [];
    foreach ($ranges as $range) {
        if ($out !== [] && $out[count($out) - 1][1] >= $range[0]) {
            $out[count($out) - 1][1] = max($out[count($out) - 1][1], $range[1]);
        } else {
            $out[] = $range;
        }
    }
    return $out;
}

/**
 * The `use` statements of the compiled template, which PHP allows only in a file's outermost scope
 * (Blade's PhpEngine includes the compiled template as a file of its own, so `@use` and a `@php`
 * block's `use` are legal there), and the compiled template without them, each byte of a statement
 * but its newlines blanked so that every line keeps its number.
 *
 * @return array{list<string>, string}
 */
function hoist(string $compiled): array
{
    $uses = [];
    $body = '';
    $depth = 0;
    $previous = null;
    $statement = null;
    foreach (token_get_all($compiled) as $token) {
        [$id, $text] = is_array($token) ? $token : [null, $token];
        if ($statement === null && $id === T_USE && $depth === 0 && $previous !== ')') {
            $statement = '';
        }
        if ($statement !== null) {
            $statement .= $text;
            $body .= preg_replace('/[^\n]/', ' ', $text);
            if ($text === ';') {
                $uses[] = trim(strip($statement));
                $statement = null;
                $previous = ';';
            }
            continue;
        }
        $body .= $text;
        if ($text === '{' || $id === T_CURLY_OPEN || $id === T_DOLLAR_OPEN_CURLY_BRACES) {
            $depth++;
        } elseif ($text === '}') {
            $depth--;
        }
        if (! in_array($id, [T_WHITESPACE, T_COMMENT, T_DOC_COMMENT, T_INLINE_HTML], true)) {
            $previous = $id === null ? $text : $id;
        }
    }
    return [$uses, $body];
}

/**
 * The compiled template as Blade's PhpEngine runs it, `use` statements hoisted to the file, and
 * the number of lines before the compiled template's first. A template may end inside PHP (a
 * `<?php` it never closes), where the function closes without a tag.
 *
 * @return array{string, int}
 */
function wrap(string $compiled): array
{
    [$uses, $body] = hoist($compiled);
    $open = false;
    foreach (token_get_all($body) as $token) {
        if (is_array($token) && in_array($token[0], [T_OPEN_TAG, T_OPEN_TAG_WITH_ECHO, T_CLOSE_TAG], true)) {
            $open = $token[0] !== T_CLOSE_TAG;
        }
    }
    $header = "<?php\n" . implode("\n", $uses) . "\n"
        . "/** @param array<string, mixed> \$__data */\n"
        . "function __rerail_view(\\Illuminate\\View\\Factory \$__env, \\Illuminate\\Support\\ViewErrorBag \$errors, array \$__data): void\n{\n"
        . "extract(\$__data, EXTR_SKIP);\n?>";
    return [$header . $body . ($open ? "\n}\n" : "<?php\n}\n"), substr_count($header, "\n")];
}

const PATTERNS = [
    '/^Undefined variable: \$(\w+)$/' => 'undefined',
    '/^Variable \$(\w+) might not be defined\.$/' => 'maybe',
    '/^Call to function compact\(\) contains undefined variable \$(\w+)\.$/' => 'compact',
    '/^Call to function compact\(\) contains possibly undefined variable \$(\w+)\.$/' => 'compact_maybe',
];

/**
 * PHPStan's `variable.undefined` reports on each wrapped file. Fails where PHPStan did not analyse a
 * file through: a crash, an error outside the files, or a diagnostic PHPStan does not let a project
 * ignore (a parse error, an internal error), which stops it reading the file's variables. A caller
 * that passes `$unparsed` takes the files PHPStan could not parse there instead, and why.
 *
 * @param array<string, string> $files wrapped file path => fixture name
 * @param array<string, string>|null $unparsed filled with the fixtures PHPStan could not parse, and why
 */
function phpstan(string $tools, array $files, string $directory, ?array &$unparsed = null): array
{
    $config = "$directory/phpstan.neon";
    file_put_contents($config, "parameters:\n  level: max\n  scanDirectories:\n    - '" . $GLOBALS['argv'][1] . "/vendor/illuminate'\n");
    $command = [$tools . '/vendor/bin/phpstan', 'analyse', '-c', $config, '--no-progress', '--error-format=json', '--memory-limit=2G', ...array_keys($files)];
    $process = proc_open($command, [1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes);
    $out = stream_get_contents($pipes[1]);
    $err = stream_get_contents($pipes[2]);
    $status = proc_close($process);
    // 0: no errors, 1: errors reported; anything else is PHPStan failing to run.
    if ($status !== 0 && $status !== 1) {
        throw new RuntimeException("phpstan exited $status: $err");
    }
    $report = json_decode($out, true, flags: JSON_THROW_ON_ERROR);
    if ($report['errors'] !== []) {
        throw new RuntimeException(json_encode($report['errors']));
    }
    $found = [];
    foreach ($report['files'] as $path => $file) {
        $name = $files[realpath($path)] ?? throw new RuntimeException("phpstan reports on $path, no fixture's");
        foreach ($file['messages'] as $message) {
            $identifier = $message['identifier'] ?? '';
            if ($unparsed !== null && str_starts_with($identifier, 'phpstan.parse')) {
                $unparsed[$name] = $message['message'];
                continue;
            }
            if (($message['ignorable'] ?? true) !== true) {
                throw new RuntimeException("$name: line {$message['line']}: PHPStan cannot analyse the compiled template: "
                    . "$identifier {$message['message']}");
            }
            if ($identifier !== 'variable.undefined') {
                continue;
            }
            foreach (PATTERNS as $pattern => $kind) {
                if (preg_match($pattern, $message['message'], $m)) {
                    $found[$name][] = ['line' => $message['line'], 'name' => $m[1], 'kind' => $kind];
                    continue 2;
                }
            }
            throw new RuntimeException("$name: unexpected message {$message['message']}");
        }
    }
    return $found;
}

const ENCODE = <<<'PHP'
function rerail_encode(mixed $value): array {
    return match (true) {
        $value === null => ['null'],
        is_bool($value) => ['bool', $value],
        is_int($value) => ['int', (string) $value],
        is_float($value) => ['float', bin2hex(pack('E', $value))],
        is_string($value) => ['string', bin2hex($value)],
        is_array($value) => ['array', array_map(
            fn ($key) => [rerail_encode($key), rerail_encode($value[$key])],
            array_keys($value),
        )],
        default => ['other', get_debug_type($value)],
    };
}
PHP;

/** The value `$code` echoes as JSON in a fresh `php -n`, or the error PHP raises. */
function run(string $code): array
{
    $process = proc_open(
        [PHP_BINARY, '-n', '-d', 'error_reporting=-1', '-d', 'display_errors=stderr', '-d', 'log_errors=0', '-r', ENCODE . "\n" . $code],
        [1 => ['pipe', 'w'], 2 => ['pipe', 'w']],
        $pipes,
    );
    $out = stream_get_contents($pipes[1]);
    $err = stream_get_contents($pipes[2]);
    proc_close($process);
    $result = json_decode($out, true);
    if (is_array($result) && trim($err) === '') {
        return $result;
    }
    foreach (explode("\n", $err) as $line) {
        if (preg_match('/^(?:PHP )?((?:Parse|Fatal) error|Warning|Deprecated|Notice):\s+(?:Uncaught (\w+): )?(.*?)(?: in Command line code(?: on line \d+|:\d+))?$/', trim($line), $m)) {
            return ['error', ($m[2] !== '' ? "$m[2]: " : '') . $m[3]];
        }
    }
    return ['error', 'no output'];
}

/** Directives whose arguments are one PHP expression Blade evaluates. */
const EXPRESSIONS = ['if', 'elseif', 'unless', 'switch', 'case', 'while', 'checked', 'selected', 'disabled', 'readonly', 'required', 'js', 'bool'];

/**
 * Each region that is an expression without variables: its value in the template's file, after
 * the `use` statements Blade hoists there. `names` marks an expression that reads a named constant
 * or class name, whose value needs PHP name resolution.
 */
function values(array $regions, string $uses): array
{
    $out = [];
    foreach ($regions as $region) {
        $expression = in_array($region['kind'], ['echo', 'attribute', 'php_echo'], true)
            || ($region['kind'] === 'directive' && in_array($region['directive'], EXPRESSIONS, true));
        $source = trim($region['text']);
        $tokens = tokens($source);
        if (! $expression || $source === '' || array_filter($tokens, fn ($token) => $token[2] === T_VARIABLE) !== []) {
            continue;
        }
        $names = false;
        foreach ($tokens as $index => [$text, , $id]) {
            $call = ($tokens[$index + 1][0] ?? null) === '(';
            $names = $names || (in_array($id, [T_STRING, T_NAME_QUALIFIED, T_NAME_FULLY_QUALIFIED, T_NAME_RELATIVE], true)
                && ! $call && ! in_array(strtolower($text), ['true', 'false', 'null'], true));
        }
        $out[] = [
            'span' => $region['span'],
            'src' => $source,
            'names' => $names,
            'const' => run("$uses\nconst RERAIL_X = $source;\necho json_encode(rerail_encode(RERAIL_X));"),
            'eval' => run("$uses\nfunction rerail_f() { return $source; }\necho json_encode(rerail_encode(rerail_f()));"),
        ];
    }
    return $out;
}

/** One template: its regions, and the marked compile PHPStan reads. */
function analyse(string $template): array
{
    [$plain, $plainSeen] = compile($template);
    [$marked, $short] = mark($template);
    [$compiled, $seen] = compile($marked);
    // `@once` and `@pushOnce` name each compile with a fresh UUID; a component's numeric attribute
    // (or `true`) is not sanitized, and with a marker before it Blade no longer sees one.
    $same = fn (string $php) => preg_replace(
        ['/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/', '/\\\\Illuminate\\\\View\\\\Compilers\\\\BladeCompiler::sanitizeComponentAttribute\(([-+.\deE]+|true)\)/'],
        ['uuid', '$1'],
        strip($php),
    );
    $texts = fn (array $seen) => array_map(fn ($region) => [$region['kind'], $region['directive'], $same($region['text'])], $seen);
    if ($texts($seen) !== $texts($plainSeen) || $same($compiled) !== $same($plain)) {
        return ['unstable' => true];
    }
    [$located, $unlocated, $variables] = regions($template, $marked, $short, $compiled, $seen);
    return ['regions' => $located, 'unlocated' => $unlocated, 'variables' => $variables, 'compiled' => $compiled];
}

/**
 * The template offsets of the variable reads Blade copied onto each line of the wrapped file,
 * whose compiled template starts after `$before` lines.
 */
function placed(string $compiled, array $variables, int $before): array
{
    $lines = [];
    foreach ($variables as [$search, $offset, $at, $name]) {
        for ($from = 0; ($found = strpos($compiled, $search, $from)) !== false; $from = $found + 1) {
            $line = $before + substr_count($compiled, "\n", 0, $found + $offset) + 1;
            $lines[$line][substr($name, 1)][] = $at;
        }
    }
    return $lines;
}

function version(string $package): string
{
    return InstalledVersions::getPrettyVersion($package) ?? 'unknown';
}

if (($argv[2] ?? null) === '--show') {
    // One template, for inspection: the marked copy, both compiles, and its regions.
    $template = file_get_contents($argv[3]);
    [$marked, $short] = mark($template);
    [$plain, $plainSeen] = compile($template);
    [$compiled, $seen] = compile($marked);
    echo "== marked\n$marked\n== plain\n$plain\n== compiled\n$compiled\n";
    echo json_encode(['plainSeen' => $plainSeen, 'seen' => $seen, 'analysed' => analyse($template)], JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES), "\n";
    exit(0);
}

/** @return list<string> Every `*.blade.php` under `$roots`, each root's in path order. */
function templates(array $roots): array
{
    $out = [];
    foreach ($roots as $root) {
        $iterator = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($root, FilesystemIterator::SKIP_DOTS));
        $paths = [];
        foreach ($iterator as $file) {
            if (str_ends_with($file->getPathname(), '.blade.php')) {
                $paths[] = $file->getPathname();
            }
        }
        sort($paths);
        array_push($out, ...$paths);
    }
    return $out;
}

if (($argv[2] ?? null) === '--corpus') {
    $out = ['illuminate/view' => version('illuminate/view'), 'php' => PHP_VERSION, 'files' => []];
    foreach (templates(array_slice($argv, 4)) as $path) {
        try {
            $analysed = analyse(file_get_contents($path));
        } catch (Throwable $error) {
            $analysed = ['error' => get_class($error) . ': ' . $error->getMessage()];
        }
        unset($analysed['variables'], $analysed['compiled']);
        $out['files'][realpath($path)] = $analysed;
    }
    file_put_contents($argv[3], json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES) . "\n");
    exit(0);
}

if (($argv[2] ?? null) === '--definedness') {
    [$tools, $output, $set] = [$argv[3], $argv[4], $argv[5]];
    RerailBlade::$directives = corpus_directives($set);
    $directory = sys_get_temp_dir() . '/rerail-blade-definedness-' . getmypid();
    @mkdir($directory);
    $files = [];
    $analysed = [];
    $wrapped = [];
    foreach (templates(array_slice($argv, 6)) as $index => $path) {
        $path = realpath($path);
        try {
            $result = analyse(file_get_contents($path));
        } catch (Throwable $error) {
            $result = ['error' => get_class($error) . ': ' . $error->getMessage()];
        }
        if (! isset($result['regions'])) {
            $files[$path] = $result;
            continue;
        }
        $analysed[$path] = $result;
        $file = "$directory/t$index.php";
        [$text, $before[$path]] = wrap($result['compiled']);
        file_put_contents($file, $text);
        $wrapped[realpath($file)] = $path;
    }
    $reports = [];
    $unparsed = [];
    foreach (array_chunk($wrapped, 250, true) as $chunk) {
        $reports += phpstan($tools, $chunk, $directory, $unparsed);
    }
    foreach ($analysed as $path => $result) {
        if (isset($unparsed[$path])) {
            $files[$path] = ['unparsed' => $unparsed[$path]];
            continue;
        }
        $lines = placed($result['compiled'], $result['variables'], $before[$path]);
        $phpstan = [];
        foreach ($reports[$path] ?? [] as $report) {
            $at = array_values(array_unique($lines[$report['line']][$report['name']] ?? []));
            sort($at);
            $phpstan[] = $report + ['at' => $at];
        }
        usort($phpstan, fn ($a, $b) => [$a['line'], $a['name']] <=> [$b['line'], $b['name']]);
        // The directives core Blade does not compile, or the application compiles in its place:
        // as text (`custom: false`) or as the application compiles them.
        $directives = [];
        foreach ($result['regions'] as $region) {
            if ($region['kind'] === 'custom' || $region['kind'] === 'text') {
                $directives[] = [
                    'directive' => $region['directive'],
                    'span' => $region['span'],
                    'custom' => $region['kind'] === 'custom',
                    'core' => method_exists(BladeCompiler::class, 'compile' . ucfirst($region['directive'])),
                ];
            }
        }
        $files[$path] = ['phpstan' => $phpstan, 'directives' => $directives];
    }
    ksort($files);
    $out = [
        'illuminate/view' => version('illuminate/view'),
        'php' => PHP_VERSION,
        'phpstan' => trim(shell_exec(escapeshellarg("$tools/vendor/bin/phpstan") . ' --version --no-ansi')),
        'directives' => ['set' => $set, 'names' => array_keys(RerailBlade::$directives)],
        'files' => $files,
    ];
    file_put_contents($output, json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES) . "\n");
    array_map('unlink', glob("$directory/*"));
    rmdir($directory);
    exit(0);
}

$tools = $argv[2];
$here = __DIR__;
$directory = sys_get_temp_dir() . '/rerail-blade-oracle-' . getmypid();
@mkdir($directory);
$fixtures = glob("$here/fixtures/*.blade.php");
sort($fixtures);
$analysed = [];
$wrapped = [];
foreach ($fixtures as $path) {
    $name = basename($path);
    $result = analyse(file_get_contents($path));
    if (isset($result['unstable'])) {
        throw new RuntimeException("$name: the marked template compiles differently");
    }
    $analysed[$name] = $result;
    $file = "$directory/" . basename($name, '.blade.php') . '.php';
    [$text, $before[$name]] = wrap($result['compiled']);
    file_put_contents($file, $text);
    $wrapped[realpath($file)] = $name;
}
$reports = phpstan($tools, $wrapped, $directory);
$out = [
    'illuminate/view' => version('illuminate/view'),
    'php' => PHP_VERSION,
    'phpstan' => trim(shell_exec(escapeshellarg("$tools/vendor/bin/phpstan") . ' --version --no-ansi')),
    'fixtures' => [],
];
foreach ($analysed as $name => $result) {
    $lines = placed($result['compiled'], $result['variables'], $before[$name]);
    $phpstan = [];
    foreach ($reports[$name] ?? [] as $report) {
        $at = array_values(array_unique($lines[$report['line']][$report['name']] ?? []));
        sort($at);
        $phpstan[] = $report + ['at' => $at];
    }
    usort($phpstan, fn ($a, $b) => [$a['line'], $a['name']] <=> [$b['line'], $b['name']]);
    $out['fixtures'][$name] = [
        'regions' => array_map(fn ($region) => array_diff_key($region, ['text' => 0]), $result['regions']),
        'unlocated' => $result['unlocated'],
        'phpstan' => $phpstan,
        'values' => values($result['regions'], implode("\n", hoist(strip($result['compiled']))[0])),
    ];
}
file_put_contents("$here/blade.expected.json", json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES) . "\n");
array_map('unlink', glob("$directory/*"));
rmdir($directory);
