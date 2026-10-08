<?php
// Reads a phpstan-src RuleTestCase statically with nikic/PHP-Parser 5.9.0 and prints, as JSON, every test
// method whose `$this->analyse([files], [errors])` call is made of constant expressions:
//   method      the test method's name
//   files       the analysed files, relative to the test's directory (`__DIR__ . '/data/x.php'`)
//   errors      the expected [message, line] pairs (a tip, the third element, is dropped)
//   properties  the `$this->name = <constant>;` assignments made before the call (the rule's options)
//   requires    the #[RequiresPhp] constraint, if any
// `if` statements on PHP_VERSION_ID or PHP_INT_SIZE are taken as this PHP (8.5.11, 64-bit) takes them,
// and a method that skips itself on this PHP is left out, as is one that builds its expectations at run
// time (a data provider, a loop) or analyses through a helper.
// Called by generate.py:  php read_tests.php <vendor/autoload.php> <SomeRuleTest.php>

declare(strict_types=1);

require $argv[1];

use PhpParser\ConstExprEvaluator;
use PhpParser\Node;
use PhpParser\Node\Expr;
use PhpParser\Node\Stmt;
use PhpParser\NodeFinder;
use PhpParser\ParserFactory;

$path = realpath($argv[2]);
$directory = dirname($path);
$statements = (new ParserFactory())->createForNewestSupportedVersion()->parse(file_get_contents($path));
$finder = new NodeFinder();

$evaluator = new ConstExprEvaluator(static function (Expr $expr) use ($directory): mixed {
    if ($expr instanceof Expr\ConstFetch) {
        return match (strtolower($expr->name->toString())) {
            'php_int_size' => PHP_INT_SIZE,
            'php_version_id' => PHP_VERSION_ID,
            default => throw new RuntimeException('not constant'),
        };
    }
    if ($expr instanceof Node\Scalar\MagicConst\Dir) {
        return $directory;
    }
    // A tip (an error's third element) may name a class constant; tips are dropped.
    if ($expr instanceof Expr\ClassConstFetch) {
        return null;
    }
    throw new RuntimeException('not constant');
});

final class Skipped extends Exception
{
}

$methods = [];
foreach ($finder->findInstanceOf($statements, Stmt\ClassMethod::class) as $method) {
    $name = $method->name->toString();
    if (!str_starts_with($name, 'test') || $method->params !== []) {
        continue;
    }
    $requires = null;
    foreach ($method->attrGroups as $group) {
        foreach ($group->attrs as $attribute) {
            if (str_ends_with($attribute->name->toString(), 'RequiresPhp')) {
                $requires = $evaluator->evaluateDirectly($attribute->args[0]->value);
            }
        }
    }
    if ($requires !== null) {
        [$operator, $version] = sscanf($requires, '%[<>=!] %s');
        if (!version_compare(PHP_VERSION, $version, $operator)) {
            continue;
        }
    }
    $variables = [];
    $properties = [];
    $found = [];
    $run = static function (array $statements) use (&$run, &$variables, &$properties, &$found, $evaluator, $directory, $name, $requires): void {
        foreach ($statements as $statement) {
            if ($statement instanceof Stmt\If_) {
                $branch = $statement->stmts;
                if (!$evaluator->evaluateDirectly($statement->cond)) {
                    $branch = $statement->else?->stmts ?? [];
                    if ($statement->elseifs !== []) {
                        throw new RuntimeException('elseif');
                    }
                }
                $run($branch);
                continue;
            }
            if (!$statement instanceof Stmt\Expression) {
                continue;
            }
            $expr = $statement->expr;
            if ($expr instanceof Expr\Assign && $expr->var instanceof Expr\Variable && is_string($expr->var->name)) {
                $variables[$expr->var->name] = $evaluator->evaluateDirectly($expr->expr);
            } elseif ($expr instanceof Expr\Assign && $expr->var instanceof Expr\PropertyFetch
                && $expr->var->var instanceof Expr\Variable && $expr->var->var->name === 'this'
                && $expr->var->name instanceof Node\Identifier) {
                $properties[$expr->var->name->toString()] = $evaluator->evaluateDirectly($expr->expr);
            } elseif ($expr instanceof Expr\MethodCall && $expr->name instanceof Node\Identifier
                && $expr->var instanceof Expr\Variable && $expr->var->name === 'this') {
                $called = $expr->name->toString();
                if ($called === 'markTestSkipped') {
                    throw new Skipped();
                }
                if ($called !== 'analyse') {
                    continue;
                }
                $argument = static fn (Expr $value): mixed => $value instanceof Expr\Variable && is_string($value->name)
                    ? ($variables[$value->name] ?? throw new RuntimeException('unknown variable'))
                    : $evaluator->evaluateDirectly($value);
                $files = $argument($expr->args[0]->value);
                $errors = $argument($expr->args[1]->value);
                $found[] = [
                    'method' => $name,
                    'files' => array_map(static fn (string $file): string => substr($file, strlen($directory) + 1), $files),
                    'errors' => array_map(static fn (array $error): array => [$error[0], $error[1]], $errors),
                    'properties' => (object) $properties,
                    'requires' => $requires,
                ];
            }
        }
    };
    try {
        $run($method->stmts ?? []);
    } catch (Throwable) {
        continue;
    }
    array_push($methods, ...$found);
}
echo json_encode($methods, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE), "\n";
