<?php
// Shared by blade_parser_tests.php beside it and the Blade pack's packs/blade/upstream/tlint/read_tests.php, and kept
// here, outside packs/, because the port readiness recorder runs it: a pull request's packs/ is recorded, never run.
// rerail_rewrite() rewrites an upstream test file so that the first argument of every call handing the tool a template
// (`new <Lint>(...)`, `Document::fromText(...)`) passes through rerail_capture() first. Running the rewritten test then
// records each template the test feeds the tool, exactly as the tool receives it, under the running test (and dataset
// row).

declare(strict_types=1);

final class RerailCapture
{
    /** @var list<array{test: string, dataset: ?string, template: string}> */
    public static array $templates = [];

    public static string $test = '';

    public static ?string $dataset = null;
}

function rerail_capture(mixed $template): mixed
{
    if (! is_string($template)) {
        throw new RuntimeException('a captured template is not a string');
    }
    RerailCapture::$templates[] = ['test' => RerailCapture::$test, 'dataset' => RerailCapture::$dataset, 'template' => $template];

    return $template;
}

/**
 * $source with the first argument of each call `new <name>(` ($prefix T_NEW) or `<class>::<name>(` ($prefix
 * T_DOUBLE_COLON) wrapped in \rerail_capture(...). PHP's own tokenizer delimits the argument (strings, heredocs and
 * interpolations are single tokens or balanced), so it ends at the first `,` or `)` outside brackets. Fails unless
 * at least one call is rewritten.
 */
function rerail_rewrite(string $source, int $prefix, string $name): string
{
    $tokens = PhpToken::tokenize($source);
    $insert = [];
    $significant = static function (int $index, int $step) use ($tokens): int {
        do {
            $index += $step;
        } while (isset($tokens[$index]) && $tokens[$index]->isIgnorable());

        return $index;
    };
    foreach ($tokens as $i => $token) {
        if (! $token->is([T_STRING, T_NAME_QUALIFIED, T_NAME_FULLY_QUALIFIED])) {
            continue;
        }
        $parts = explode('\\', $token->text);
        if (end($parts) !== $name || ! ($tokens[$significant($i, -1)] ?? null)?->is($prefix)) {
            continue;
        }
        $open = $significant($i, 1);
        if (($tokens[$open] ?? null)?->text !== '(') {
            continue;
        }
        $depth = 0;
        for ($end = $open + 1; isset($tokens[$end]); $end++) {
            $text = $tokens[$end]->text;
            if ($text === '(' || $text === '[' || $text === '{' || $tokens[$end]->is(T_DOLLAR_OPEN_CURLY_BRACES)) {
                $depth++;
            } elseif ($text === ')' || $text === ']' || $text === '}') {
                if ($depth === 0) {
                    break;
                }
                $depth--;
            } elseif ($text === ',' && $depth === 0) {
                break;
            }
        }
        if ($significant($open, 1) >= $end) {
            throw new RuntimeException("{$name}() on line {$token->line} has no argument to capture");
        }
        $insert[$open + 1] = '\\rerail_capture(';
        $insert[$end] = ')';
    }
    if ($insert === []) {
        throw new RuntimeException("no call of {$name}() to capture");
    }
    $rewritten = '';
    foreach ($tokens as $i => $token) {
        $rewritten .= ($insert[$i] ?? '').$token->text;
    }

    return $rewritten;
}
