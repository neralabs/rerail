<?php
// PHPStan's tests and configuration, as the port readiness gates read them. The first argument is the Composer autoloader
// the request's op needs: the PHP toolkit runtime's (conformance/recorders/port-readiness/runtimes/php, nikic/php-parser)
// for `tests` and `references`, which read upstream's sources statically; `facts` loads the pinned PHPStan's own phar.
//
// One JSON request on stdin, one JSON response on stdout:
// - {"op": "tests", "tests": [<RuleTestCase file>, ...]}: per test file, `methods`: every test method whose
//   `$this->analyse([files], [errors])` call is made of constant expressions, read statically with nikic/php-parser:
//     method      the test method's name
//     files       the analysed files, as absolute paths (`__DIR__ . '/data/x.php'`, resolved)
//     errors      the expected [message, line] pairs (a tip, the third element, is dropped)
//     properties  the rule options the call runs with: the test class's property defaults, then the
//                 `$this->name = <constant>;` assignments the method makes before the call
//     required    the files the method requires before the call (`require_once __DIR__ . '/data/x-definition.php'`):
//                 symbols the analysed files use, which PHPStan reflects but does not analyse
//   `if` statements on PHP_VERSION_ID or PHP_INT_SIZE are taken as this PHP takes them, and a method that skips itself
//   on this PHP (#[RequiresPhp], markTestSkipped) is left out, as is one that builds its expectations at run time (a
//   data provider, a loop) or analyses through a helper. This is packs/php/upstream/phpstan/read_tests.php's reading.
//   And `left`: every test method left out, with why (`skipped` when it skips itself on this PHP, as upstream's own
//   run does, rather than a method this reading cannot evaluate), so a reading that shrinks says so.
// - {"op": "references", "files": [...]}: per file, the classes its code names (imports, types, `new`, static calls,
//   attributes), fully qualified: what a rule class delegates to.
// - {"op": "facts", "phar": <phpstan.phar>, "package": <installed package directory>, "scratch": <directory>,
//   "levels": [...], "rules": [<class>, ...], "parameters": [<name>, ...]}: the installed release (its version, the
//   phpstan-src commit the phar was built from, its declared license), the files of the phar's src/, and per rule level
//   (`level0` … `level10`, PHPStan's conf/config.level<N>.neon) which of `rules` its container registers as rules and
//   the value it gives each of `parameters` (a top-level parameter, or a feature toggle).

declare(strict_types=1);

use PhpParser\ConstExprEvaluator;
use PhpParser\Node;
use PhpParser\Node\Expr;
use PhpParser\Node\Stmt;
use PhpParser\NodeFinder;
use PhpParser\NodeTraverser;
use PhpParser\NodeVisitor\NameResolver;
use PhpParser\ParserFactory;

final class Skipped extends Exception
{
}

/**
 * The test methods of one RuleTestCase whose analyse() calls read statically (read_tests.php), and every other test
 * method with why it is left out.
 */
function test_methods(string $path): array
{
    $path = realpath($path);
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
    // A property the test class declares with a constant default is an option its methods run with unless they set it.
    $defaults = [];
    foreach ($finder->findInstanceOf($statements, Stmt\Property::class) as $property) {
        foreach ($property->props as $item) {
            if ($item->default === null) continue;
            try {
                $defaults[$item->name->toString()] = $evaluator->evaluateDirectly($item->default);
            } catch (Throwable) {
            }
        }
    }
    $methods = [];
    $left = [];
    // Why a method is left out; `skipped` when it skips itself on this PHP. A message never names this driver's path.
    $leave = static function (string $method, bool $skipped, string $reason) use (&$left): void {
        $left[] = ['method' => $method, 'skipped' => $skipped, 'reason' => str_replace(__DIR__ . '/', '', $reason)];
    };
    foreach ($finder->findInstanceOf($statements, Stmt\ClassMethod::class) as $method) {
        $name = $method->name->toString();
        if (!str_starts_with($name, 'test')) {
            continue;
        }
        if ($method->params !== []) {
            $leave($name, false, 'takes its expectations from a data provider');
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
                $leave($name, true, "requires PHP {$requires}");
                continue;
            }
        }
        $variables = [];
        $properties = $defaults;
        $required = [];
        $found = [];
        $run = static function (array $statements) use (&$run, &$variables, &$properties, &$required, &$found, $evaluator, $name): void {
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
                if ($expr instanceof Expr\Include_) {
                    $file = $evaluator->evaluateDirectly($expr->expr);
                    $required[] = realpath($file) ?: throw new RuntimeException("no file {$file}");
                } elseif ($expr instanceof Expr\Assign && $expr->var instanceof Expr\Variable && is_string($expr->var->name)) {
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
                    $files = array_map(static fn (string $file): string => realpath($file) ?: throw new RuntimeException("no file {$file}"), $argument($expr->args[0]->value));
                    $found[] = [
                        'method' => $name,
                        'files' => $files,
                        'errors' => array_map(static fn (array $error): array => [$error[0], $error[1]], $argument($expr->args[1]->value)),
                        'properties' => (object) $properties,
                        'required' => $required,
                    ];
                }
            }
        };
        try {
            $run($method->stmts ?? []);
        } catch (Skipped) {
            $leave($name, true, 'calls markTestSkipped() on this PHP');
            continue;
        } catch (Throwable $error) {
            $leave($name, false, $error->getMessage());
            continue;
        }
        if ($found === []) {
            $leave($name, false, 'makes no $this->analyse() call of its own (a helper analyses, or nothing does)');
            continue;
        }
        array_push($methods, ...$found);
    }
    return ['methods' => $methods, 'left' => $left];
}

/** The fully qualified classes a file's code names, sorted. */
function references(string $path): array
{
    $statements = (new ParserFactory())->createForNewestSupportedVersion()->parse(file_get_contents($path)) ?? [];
    $statements = (new NodeTraverser(new NameResolver(null, ['replaceNodes' => false])))->traverse($statements);
    $finder = new NodeFinder();
    $classes = [];
    // NameResolver marks a class name with `resolvedName`; a function's or constant's takes `namespacedName` instead.
    foreach ($finder->findInstanceOf($statements, Node\Name::class) as $name) {
        $resolved = $name->getAttribute('resolvedName');
        if ($resolved instanceof Node\Name) $classes[ltrim($resolved->toString(), '\\')] = true;
    }
    foreach ($finder->findInstanceOf($statements, Stmt\Use_::class) as $use) {
        if ($use->type !== Stmt\Use_::TYPE_NORMAL) continue;
        foreach ($use->uses as $item) $classes[ltrim($item->name->toString(), '\\')] = true;
    }
    $classes = array_keys($classes);
    sort($classes, SORT_STRING);
    return $classes;
}

/** The installed release, the phar's src/ files, and per level the ported rules it registers and its parameters. */
function facts(array $request): array
{
    // What PHPStan's own bin/phpstan sets before it loads the phar: its sources are written for every PHP it supports.
    error_reporting(E_ALL & ~E_DEPRECATED);
    $phar = $request['phar'];
    require_once "phar://{$phar}/vendor/autoload.php";
    require_once "phar://{$phar}/preload.php";
    $installed = require "phar://{$phar}/vendor/composer/installed.php";
    $package = json_decode(file_get_contents("{$request['package']}/composer.json"), true, 512, JSON_THROW_ON_ERROR);
    $source = "phar://{$phar}/src/";
    $files = [];
    foreach (new RecursiveIteratorIterator(new RecursiveDirectoryIterator($source, FilesystemIterator::SKIP_DOTS)) as $file) {
        $files[] = substr($file->getPathname(), strlen($source));
    }
    sort($files, SORT_STRING);
    $factory = new PHPStan\DependencyInjection\ContainerFactory($request['scratch']);
    $levels = [];
    foreach ($request['levels'] as $level) {
        $container = $factory->create($request['scratch'] . '/container', [$factory->getConfigDirectory() . "/config.{$level}.neon"], []);
        $registered = array_map(static fn (object $rule): string => get_class($rule), $container->getServicesByTag('phpstan.rules.rule'));
        $parameters = $container->getParameters();
        $values = [];
        foreach ($request['parameters'] as $name) {
            $values[$name] = array_key_exists($name, $parameters) ? $parameters[$name] : ($parameters['featureToggles'][$name] ?? null);
        }
        $levels[$level] = [
            'rules' => array_values(array_intersect($request['rules'], $registered)),
            'parameters' => (object) $values,
        ];
    }
    return [
        'name' => $installed['root']['name'],
        'version' => $installed['root']['pretty_version'],
        'reference' => $installed['root']['reference'],
        'license' => implode(' OR ', (array) ($package['license'] ?? [])),
        'php' => PHP_VERSION,
        'files' => $files,
        'levels' => (object) $levels,
    ];
}

$request = json_decode(stream_get_contents(STDIN), true, 512, JSON_THROW_ON_ERROR);
if ($request['op'] !== 'facts') {
    require $argv[1];
}
$response = match ($request['op']) {
    'tests' => array_map(test_methods(...), array_combine($request['tests'], $request['tests'])),
    'references' => array_map(references(...), array_combine($request['files'], $request['files'])),
    'facts' => facts($request),
    default => throw new RuntimeException("unknown op {$request['op']}"),
};
echo json_encode($response, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE), "\n";
