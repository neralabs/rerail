<?php
// Writes bindings.expected.json: for every variables/*.php and bindings/*.php fixture, each PHP
// variable (a function-like scope's name, as PHP 8.5.11 scopes variables), read off
// nikic/PHP-Parser 5.9.0's syntax tree:
//   name         the variable, `$` included
//   bindings     where it is bound, in the order PHP evaluates them: each the start byte of the
//                name node (`$x`) and the form: assignment (`=`, `=&`, `??=`, a list
//                destructuring's target, a subscript assignment's base `$a[] = v`, `unset($a[k])`,
//                a by-reference argument of a function PHP 8.5.11 defines, by the parameter a
//                named argument names or the position Reflection gives), augmented (`op=`,
//                `++`, `--`), for (a `foreach` key or value, a destructured one too), except (a
//                `catch` variable), parameter (a parameter, a closure's `use` variable), global,
//                static
//   occurrences  the start byte of every `$x` node that names it, in document order
// A function, method or closure has its own variables; a closure's `use` variable is its own
// (bound there) and names the enclosing scope's too; an arrow function's variable is its own when
// the arrow function binds the name (a parameter or any binding in its body), else the enclosing
// scope's, which it copies. `$this`, superglobals, `$$name` and `compact` strings name none.
// Run offline, not in CI, from this directory:
//   /opt/homebrew/Cellar/php/8.5.11/bin/php -n generate_bindings.php <composer vendor/autoload.php>
// with nikic/php-parser 5.9.0 installed by that Composer project.

declare(strict_types=1);

use PhpParser\Node;
use PhpParser\Node\Expr;
use PhpParser\Node\Name;
use PhpParser\Node\Stmt;
use PhpParser\NodeTraverser;
use PhpParser\NodeVisitor\NameResolver;
use PhpParser\ParserFactory;
use PhpParser\PhpVersion;

if ($argc !== 2) {
    fwrite(STDERR, "usage: php -n generate_bindings.php <vendor/autoload.php>\n");
    exit(2);
}
require $argv[1];

const PHP_PARSER = 'v5.9.0';
if (PHP_VERSION !== '8.5.11') {
    throw new RuntimeException('PHP 8.5.11 required, not ' . PHP_VERSION);
}
$parserVersion = Composer\InstalledVersions::getPrettyVersion('nikic/php-parser');
if ($parserVersion !== PHP_PARSER) {
    throw new RuntimeException('nikic/php-parser ' . PHP_PARSER . ' required, not ' . $parserVersion);
}

const SUPERGLOBALS = ['GLOBALS', '_SERVER', '_GET', '_POST', '_FILES', '_COOKIE', '_SESSION', '_REQUEST', '_ENV'];

final class Scope
{
    /** @var array<string, list<array{int, string}>> each name's bindings, in order */
    public array $bindings = [];

    public function __construct(public readonly ?Scope $parent, public readonly bool $arrow)
    {
    }
}

final class Walker
{
    /** @var list<Scope> */
    public array $scopes = [];
    /** @var list<array{Scope, string, int}> each `$x` node: the scope it is written in, its name, its start */
    public array $named = [];

    /** The variable a `$x` node names, or null for `$this`, a superglobal or `$$x`. */
    private static function name(Node $node): ?string
    {
        if (!$node instanceof Expr\Variable || !is_string($node->name)) {
            return null;
        }
        return $node->name === 'this' || in_array($node->name, SUPERGLOBALS, true) ? null : $node->name;
    }

    private function scope(?Scope $parent, bool $arrow): Scope
    {
        $scope = new Scope($parent, $arrow);
        $this->scopes[] = $scope;
        return $scope;
    }

    private function mention(Node $node, Scope $scope): void
    {
        if (($name = self::name($node)) !== null) {
            $this->named[] = [$scope, $name, $node->getStartFilePos()];
        }
    }

    private function bind(Node $node, string $form, Scope $scope): void
    {
        if (($name = self::name($node)) !== null) {
            $scope->bindings[$name][] = [$node->getStartFilePos(), $form];
            $this->named[] = [$scope, $name, $node->getStartFilePos()];
        } elseif ($node instanceof Expr\Variable && $node->name instanceof Expr) {
            $this->walk($node->name, $scope);
        }
    }

    /** @param list<Node>|Node|null $nodes */
    public function walk(array|Node|null $nodes, Scope $scope): void
    {
        if ($nodes === null) {
            return;
        }
        if (is_array($nodes)) {
            foreach ($nodes as $node) {
                if ($node instanceof Node) {
                    $this->walk($node, $scope);
                }
            }
            return;
        }
        $node = $nodes;
        switch (true) {
            case $node instanceof Stmt\Function_:
            case $node instanceof Stmt\ClassMethod:
                $inner = $this->scope(null, false);
                $this->params($node->params, $inner);
                $this->walk($node->stmts, $inner);
                return;
            case $node instanceof Expr\Closure:
                foreach ($node->uses as $use) {
                    $this->mention($use->var, $scope);
                }
                $inner = $this->scope(null, false);
                $this->params($node->params, $inner);
                foreach ($node->uses as $use) {
                    $this->bind($use->var, 'parameter', $inner);
                }
                $this->walk($node->stmts, $inner);
                return;
            case $node instanceof Expr\ArrowFunction:
                $inner = $this->scope($scope, true);
                $this->params($node->params, $inner);
                $this->walk($node->expr, $inner);
                return;
            case $node instanceof Stmt\ClassLike:
                foreach ($node->stmts as $member) {
                    if ($member instanceof Stmt\ClassMethod) {
                        $this->walk($member, $scope);
                    }
                }
                return;
            case $node instanceof Expr\New_ && $node->class instanceof Stmt\Class_:
                $this->walk($node->args, $scope);
                $this->walk($node->class, $scope);
                return;
            case $node instanceof Expr\Variable:
                if ($node->name instanceof Expr) {
                    $this->walk($node->name, $scope);
                } else {
                    $this->mention($node, $scope);
                }
                return;
            case $node instanceof Expr\StaticPropertyFetch:
                $this->walk($node->class, $scope);
                if ($node->name instanceof Expr) {
                    $this->walk($node->name, $scope);
                }
                return;
            case $node instanceof Expr\Assign:
            case $node instanceof Expr\AssignRef:
                $this->walk($node->expr, $scope);
                $this->target($node->var, 'assignment', $scope);
                return;
            case $node instanceof Expr\AssignOp:
                $coalesce = $node instanceof Expr\AssignOp\Coalesce;
                if (self::name($node->var) !== null) {
                    $this->mention($node->var, $scope);
                    $this->walk($node->expr, $scope);
                    $this->bind($node->var, $coalesce ? 'assignment' : 'augmented', $scope);
                } else {
                    $this->base($node->var, $scope);
                    $this->walk($node->expr, $scope);
                }
                return;
            case $node instanceof Expr\PreInc:
            case $node instanceof Expr\PostInc:
            case $node instanceof Expr\PreDec:
            case $node instanceof Expr\PostDec:
                if (self::name($node->var) !== null) {
                    $this->bind($node->var, 'augmented', $scope);
                } else {
                    $this->base($node->var, $scope);
                }
                return;
            case $node instanceof Stmt\For_:
                $this->walk($node->init, $scope);
                $this->walk($node->cond, $scope);
                $this->walk($node->stmts, $scope);
                $this->walk($node->loop, $scope);
                return;
            case $node instanceof Stmt\Foreach_:
                $this->walk($node->expr, $scope);
                if ($node->keyVar !== null) {
                    $this->foreachTarget($node->keyVar, $scope);
                }
                $this->foreachTarget($node->valueVar, $scope);
                $this->walk($node->stmts, $scope);
                return;
            case $node instanceof Stmt\Catch_:
                if ($node->var !== null) {
                    $this->bind($node->var, 'except', $scope);
                }
                $this->walk($node->stmts, $scope);
                return;
            case $node instanceof Stmt\Global_:
                foreach ($node->vars as $var) {
                    $this->bind($var, 'global', $scope);
                }
                return;
            case $node instanceof Stmt\Static_:
                foreach ($node->vars as $var) {
                    $this->walk($var->default, $scope);
                    $this->bind($var->var, 'static', $scope);
                }
                return;
            case $node instanceof Stmt\Unset_:
                foreach ($node->vars as $var) {
                    if ($var instanceof Expr\ArrayDimFetch) {
                        $this->base($var, $scope);
                    } else {
                        $this->walk($var, $scope);
                    }
                }
                return;
            case $node instanceof Expr\FuncCall:
                $this->call($node, $scope);
                return;
        }
        foreach ($node->getSubNodeNames() as $name) {
            $child = $node->$name;
            if ($child instanceof Node || is_array($child)) {
                $this->walk($child, $scope);
            }
        }
    }

    /** @param list<Node\Param> $params */
    private function params(array $params, Scope $scope): void
    {
        foreach ($params as $param) {
            $this->bind($param->var, 'parameter', $scope);
        }
    }

    private function target(Node $node, string $form, Scope $scope): void
    {
        if ($node instanceof Expr\Variable) {
            $this->bind($node, $form, $scope);
        } elseif ($node instanceof Expr\List_) {
            $this->destructure($node, $form, $scope);
        } else {
            $this->base($node, $scope);
        }
    }

    private function destructure(Expr\List_ $list, string $form, Scope $scope): void
    {
        foreach ($list->items as $item) {
            if ($item === null) {
                continue;
            }
            $this->walk($item->key, $scope);
            if ($item->value instanceof Expr\Variable) {
                $this->bind($item->value, $form, $scope);
            } elseif ($item->value instanceof Expr\List_) {
                $this->destructure($item->value, $form, $scope);
            } else {
                // A subscript or property target is an assignment's, as any other target.
                $this->target($item->value, 'assignment', $scope);
            }
        }
    }

    private function foreachTarget(Node $node, Scope $scope): void
    {
        if ($node instanceof Expr\Variable) {
            $this->bind($node, 'for', $scope);
        } elseif ($node instanceof Expr\List_) {
            $this->destructure($node, 'for', $scope);
        } else {
            $this->walk($node, $scope);
        }
    }

    /** `$a[k] = v`: the keys are read, innermost last, and the base variable is bound anew. */
    private function base(Node $node, Scope $scope): void
    {
        if (!$node instanceof Expr\ArrayDimFetch) {
            $this->walk($node, $scope);
            return;
        }
        $this->walk($node->dim, $scope);
        if ($node->var instanceof Expr\Variable && self::name($node->var) !== null) {
            $this->bind($node->var, 'assignment', $scope);
        } else {
            $this->base($node->var, $scope);
        }
    }

    /** A call: a function PHP defines binds the variables it takes by reference, a named
     * argument's by the parameter it names. */
    private function call(Expr\FuncCall $call, Scope $scope): void
    {
        if (!$call->name instanceof Name) {
            $this->walk($call->name, $scope);
        }
        $function = self::builtin($call->name);
        $position = 0;
        foreach ($call->args as $arg) {
            if (!$arg instanceof Node\Arg) {
                continue;
            }
            $referenced = $function !== null && !$arg->unpack
                && self::byReference($function, $arg->name?->toString() ?? $position++);
            if ($referenced && self::name($arg->value) !== null) {
                $this->bind($arg->value, 'assignment', $scope);
            } else {
                $this->walk($arg->value, $scope);
            }
        }
    }

    /** The function PHP 8.5.11 defines that a callee name resolves to, or null. `extract` takes
     * its array by reference only to hand out references to its elements (`EXTR_REFS`); it never
     * assigns it, and defines variables the source does not name, so it binds none here. */
    private static function builtin(Node $name): ?ReflectionFunction
    {
        if (!$name instanceof Name || $name->getLast() === 'extract') {
            return null;
        }
        $resolved = $name->getAttribute('resolvedName') ?? $name->getAttribute('namespacedName');
        $candidates = $resolved instanceof Name && $name->getAttribute('namespacedName') === null
            ? [$resolved->toString()]
            : [$name->getLast()];
        foreach ($candidates as $candidate) {
            if (function_exists($candidate) && (new ReflectionFunction($candidate))->isInternal()) {
                return new ReflectionFunction($candidate);
            }
        }
        return null;
    }

    /** Whether `function` takes the parameter `at` names (a named argument's), or the one at
     * position `at`, by reference. */
    private static function byReference(ReflectionFunction $function, int|string $at): bool
    {
        $parameters = $function->getParameters();
        if (is_string($at)) {
            foreach ($parameters as $parameter) {
                if ($parameter->getName() === $at) {
                    return $parameter->isPassedByReference();
                }
            }
            return false;
        }
        $parameter = $parameters[$at] ?? null;
        if ($parameter === null) {
            $last = end($parameters);
            $parameter = $last !== false && $last->isVariadic() ? $last : null;
        }
        return $parameter !== null && $parameter->isPassedByReference();
    }
}

$parser = (new ParserFactory())->createForVersion(PhpVersion::fromComponents(8, 5));
$files = [];
$fixtures = array_merge(glob(__DIR__ . '/variables/*.php'), glob(__DIR__ . '/bindings/*.php'));
sort($fixtures, SORT_STRING);
foreach ($fixtures as $path) {
    $source = file_get_contents($path);
    $statements = $parser->parse($source);
    (new NodeTraverser(new NameResolver(null, ['replaceNodes' => false])))->traverse($statements);
    $walker = new Walker();
    $top = new Scope(null, false);
    $walker->scopes[] = $top;
    $walker->walk($statements, $top);
    // Each `$x` names the variable of the scope it is written in, or, in an arrow function that
    // never binds the name, the enclosing scope's.
    $variables = new SplObjectStorage();
    foreach ($walker->named as [$scope, $name, $start]) {
        while ($scope->arrow && !isset($scope->bindings[$name]) && $scope->parent !== null) {
            $scope = $scope->parent;
        }
        $names = $variables[$scope] ?? [];
        $names[$name][] = $start;
        $variables[$scope] = $names;
    }
    $records = [];
    foreach ($walker->scopes as $scope) {
        foreach ($scope->bindings as $name => $bindings) {
            $occurrences = array_values(array_unique(($variables[$scope] ?? [])[$name] ?? []));
            sort($occurrences);
            $records[] = [
                'name' => '$' . $name,
                'bindings' => array_map(fn ($binding) => ['start' => $binding[0], 'form' => $binding[1]], $bindings),
                'occurrences' => $occurrences,
            ];
        }
    }
    usort($records, fn ($a, $b) => $a['bindings'][0]['start'] <=> $b['bindings'][0]['start']);
    $files[basename(dirname($path)) . '/' . basename($path)] = $records;
}

file_put_contents(
    __DIR__ . '/bindings.expected.json',
    json_encode(
        ['php' => PHP_VERSION, 'phpParser' => $parserVersion, 'files' => $files],
        JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR,
    ) . "\n",
);
