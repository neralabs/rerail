<?php
// Reads a Psalm 6.19.1 test class statically with nikic/PHP-Parser 5.9.0 and prints, as JSON, the cases its
// providerValidCodeParse() and providerInvalidCodeParse() return:
//   provider       "valid" or "invalid"
//   name           the case's key
//   code           its PHP source
//   error_message  (invalid cases) the issue Psalm must report, as the test writes it
//   ignored_issues the issues the test runs with ignored; php_version  the PHP version it analyses for
// Only cases made of constant expressions are printed (DIRECTORY_SEPARATOR is "/").
// Called by generate.py:  php read_tests.php <vendor/autoload.php> <SomeTest.php>

declare(strict_types=1);

require $argv[1];

use PhpParser\ConstExprEvaluator;
use PhpParser\Node;
use PhpParser\Node\Expr;
use PhpParser\Node\Stmt;
use PhpParser\NodeFinder;
use PhpParser\ParserFactory;

$statements = (new ParserFactory())->createForNewestSupportedVersion()->parse(file_get_contents($argv[2]));
$evaluator = new ConstExprEvaluator(static function (Expr $expr): mixed {
    if ($expr instanceof Expr\ConstFetch && $expr->name->toString() === 'DIRECTORY_SEPARATOR') {
        return '/';
    }
    throw new RuntimeException('not constant');
});

$cases = [];
foreach ((new NodeFinder())->findInstanceOf($statements, Stmt\ClassMethod::class) as $method) {
    $provider = match ($method->name->toString()) {
        'providerValidCodeParse' => 'valid',
        'providerInvalidCodeParse' => 'invalid',
        default => null,
    };
    if ($provider === null) {
        continue;
    }
    foreach ($method->stmts ?? [] as $statement) {
        if (!$statement instanceof Stmt\Return_ || !$statement->expr instanceof Expr\Array_) {
            continue;
        }
        foreach ($statement->expr->items as $item) {
            try {
                $name = $evaluator->evaluateDirectly($item->key);
                $case = $evaluator->evaluateDirectly($item->value);
            } catch (Throwable) {
                continue;
            }
            $cases[] = [
                'provider' => $provider,
                'name' => $name,
                'code' => $case['code'],
                'error_message' => $case['error_message'] ?? null,
                'ignored_issues' => $case['ignored_issues'] ?? [],
                'php_version' => $case['php_version'] ?? null,
            ];
        }
    }
}
echo json_encode($cases, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE), "\n";
