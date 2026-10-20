<?php
// Psalm, as the port readiness gates read it, inside the pinned runtime of
// conformance/recorders/port-readiness/runtimes/psalm-<version>, whose directory is the first argument.
//
// One JSON request on stdin, one JSON response on stdout:
// - {"op": "facts", "issues": [...], "checkout": <dir>, "scratch": <dir>}: per issue type, its class file, its
//   deprecation (an `@deprecated` tag on the class) and whether the recommended configuration reports it as an error
//   (its CodeIssue::ERROR_LEVEL against the configured level); the recommended configuration itself, the psalm.xml
//   `psalm --init` writes for a project whose source is src/ and in which Psalm finds no issue
//   (Config\Creator::getContents at level 1, read by Config in a scratch project of that shape, `scratch`); every case of
//   every test class at the pinned checkout (tests/**/*.php), read statically from its
//   providerValidCodeParse() and providerInvalidCodeParse() as upstream/psalm/read_tests.php reads them, with every
//   item or provider the reading cannot evaluate and why (`left`); the test classes that turn on unused variable
//   detection (`reportUnusedVariables()` in their setUp, as UnusedVariableTest does); the installed version and
//   declared license.
//
// Psalm itself runs as its command line (vendor/bin/psalm), which the adapter calls: this driver reads only the
// installed Composer metadata, the issue classes, Config and Config\Creator, and the tests' source.

declare(strict_types=1);

use Composer\InstalledVersions;
use PhpParser\ConstExprEvaluator;
use PhpParser\Node\Expr;
use PhpParser\Node\Stmt;
use PhpParser\NodeFinder;
use PhpParser\ParserFactory;
use Psalm\Config;
use Psalm\Config\Creator;

require $argv[1] . '/vendor/autoload.php';

const PACKAGE = 'vendor/vimeo/psalm';

/** Every PHP file under a directory, depth first, in byte order of names (the same on every machine). */
function php_files(string $directory): array
{
    $files = [];
    $entries = scandir($directory);
    sort($entries, SORT_STRING);
    foreach ($entries as $entry) {
        if ($entry === '.' || $entry === '..') continue;
        $path = "{$directory}/{$entry}";
        if (is_dir($path)) {
            array_push($files, ...php_files($path));
        } elseif (str_ends_with($entry, '.php')) {
            $files[] = $path;
        }
    }
    return $files;
}

/**
 * The cases a test class's providerValidCodeParse() and providerInvalidCodeParse() return, read statically: only items
 * whose key and value are constant expressions (DIRECTORY_SEPARATOR is "/"), as upstream/psalm/read_tests.php reads them.
 * Every other item, and a provider that returns no array literal, is `left`, with why: [provider, item, reason], the
 * item named by its key where that reads, else by its position (`#<n>`), and null for a whole provider.
 */
function provider_cases(array $statements, ConstExprEvaluator $evaluator): array
{
    $cases = [];
    $left = [];
    foreach ((new NodeFinder())->findInstanceOf($statements, Stmt\ClassMethod::class) as $method) {
        $provider = match ($method->name->toString()) {
            'providerValidCodeParse' => 'valid',
            'providerInvalidCodeParse' => 'invalid',
            default => null,
        };
        if ($provider === null) continue;
        $returned = false;
        foreach ($method->stmts ?? [] as $statement) {
            if (!$statement instanceof Stmt\Return_ || !$statement->expr instanceof Expr\Array_) continue;
            $returned = true;
            foreach ($statement->expr->items as $index => $item) {
                $leave = static function (string $reason) use (&$left, $provider, $item, $index, $evaluator): void {
                    try {
                        $name = $item->key === null ? null : $evaluator->evaluateDirectly($item->key);
                    } catch (Throwable) {
                        $name = null;
                    }
                    $left[] = ['provider' => $provider, 'item' => is_string($name) ? $name : "#{$index}", 'reason' => str_replace(__DIR__ . '/', '', $reason)];
                };
                if ($item->unpack) {
                    $leave('spreads another array into the provider');
                    continue;
                }
                if ($item->key === null) {
                    $leave('has no key');
                    continue;
                }
                try {
                    $name = $evaluator->evaluateDirectly($item->key);
                    $case = $evaluator->evaluateDirectly($item->value);
                } catch (Throwable $error) {
                    $leave($error->getMessage());
                    continue;
                }
                if (!is_string($name)) {
                    $leave('has a key that is no string');
                    continue;
                }
                if (!is_array($case) || !is_string($case['code'] ?? null)) {
                    $leave('is no array with a string `code`');
                    continue;
                }
                $cases[] = [
                    'provider' => $provider,
                    'name' => $name,
                    'code' => $case['code'],
                    'error_message' => $case['error_message'] ?? null,
                    'ignored_issues' => array_values($case['ignored_issues'] ?? []),
                    'php_version' => $case['php_version'] ?? null,
                ];
            }
        }
        if (!$returned) {
            $left[] = ['provider' => $provider, 'item' => null, 'reason' => 'returns no array literal (it yields, or builds the array at run time)'];
        }
    }
    return ['cases' => $cases, 'left' => $left];
}

function facts(array $request): array
{
    global $argv;
    $package = $argv[1] . '/' . PACKAGE;
    $checkout = $request['checkout'];

    // The configuration `psalm --init` writes for a project with src/ and no issue Psalm finds (autoGenerateConfig
    // picks level 1 then), in a scratch project of that shape so that Config reads it as Psalm would.
    $scratch = $request['scratch'];
    if (!is_dir("{$scratch}/src")) mkdir("{$scratch}/src", 0777, true);
    $recommended = Creator::getContents($scratch, null, 1, 'vendor');
    $config = Config::loadFromXML($scratch, $recommended);

    $issues = [];
    foreach ($request['issues'] as $issue) {
        $class = "Psalm\\Issue\\{$issue}";
        if (!preg_match('/^[A-Z][A-Za-z0-9]*$/', $issue) || !class_exists($class)) continue;
        $reflection = new ReflectionClass($class);
        $deprecation = preg_match('/@deprecated\b[ \t]*([^\n]*)/', (string) $reflection->getDocComment(), $match) === 1
            ? trim(rtrim($match[1], '*/ ')) ?: 'deprecated'
            : null;
        $issues[$issue] = [
            'file' => $reflection->getFileName(),
            'deprecation' => $deprecation,
            'recommended' => $config->getReportingLevelForFile($issue, "{$scratch}/src/somefile.php") === Config::REPORT_ERROR,
        ];
    }

    $parser = (new ParserFactory())->createForNewestSupportedVersion();
    $evaluator = new ConstExprEvaluator(static function (Expr $expr): mixed {
        if ($expr instanceof Expr\ConstFetch && $expr->name->toString() === 'DIRECTORY_SEPARATOR') return '/';
        throw new RuntimeException("Expression of type {$expr->getType()} cannot be evaluated");
    });
    $tests = [];
    foreach (php_files("{$checkout}/tests") as $path) {
        $statements = $parser->parse(file_get_contents($path)) ?? [];
        ['cases' => $cases, 'left' => $left] = provider_cases($statements, $evaluator);
        if ($cases === [] && $left === []) continue;
        $setUp = array_filter((new NodeFinder())->findInstanceOf($statements, Stmt\ClassMethod::class), static fn (Stmt\ClassMethod $method): bool => $method->name->toLowerString() === 'setup');
        $unused = array_filter((new NodeFinder())->findInstanceOf($setUp, Expr\MethodCall::class), static fn (Expr\MethodCall $call): bool => $call->name instanceof PhpParser\Node\Identifier && $call->name->toString() === 'reportUnusedVariables');
        $tests[] = [
            'path' => substr($path, strlen($checkout) + 1),
            'unusedVariables' => $unused !== [],
            'cases' => $cases,
            'left' => $left,
        ];
    }

    $manifest = json_decode(file_get_contents("{$package}/composer.json"), true, 512, JSON_THROW_ON_ERROR);
    return [
        'version' => InstalledVersions::getPrettyVersion('vimeo/psalm'),
        'license' => is_array($manifest['license'] ?? null) ? implode(' OR ', $manifest['license']) : ($manifest['license'] ?? null),
        'php' => PHP_VERSION,
        'package' => $package,
        'recommended' => $recommended,
        'issues' => (object) $issues,
        'tests' => $tests,
    ];
}

$request = json_decode(stream_get_contents(STDIN), true, 512, JSON_THROW_ON_ERROR);
$response = match ($request['op']) {
    'facts' => facts($request),
    default => throw new RuntimeException("unknown op {$request['op']}"),
};
echo json_encode($response, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_INVALID_UTF8_SUBSTITUTE), "\n";
