<?php
// Writes flow.expected.json: for every flow/*.php fixture, what PHP 8.5.11 does with each binding
// of a variable when it runs the fixture: its top-level code, then every function named `case_*`
// with every combination of its parameters' inputs (bool: false, true; int: 0 to 3; array: [],
// [1], [2], [1, 1], [1, 2], [2, 1], [2, 2]), each fixture in a fresh `php -n` process. The
// fixture is traced, not analysed: nikic/PHP-Parser 5.9.0 inserts after each statement that
// binds a variable (and where a foreach target, a catch variable, a parameter or a closure's
// `use` variable is bound; a by-reference argument of a function PHP defines by its position or
// the parameter it names; `unset($a[k])` only where `$a` is defined, which is all it changes) a
// probe that records the binding the variable held before (its tag: the start byte of the name
// node, or -1 where it was undefined), and where each body ends normally (a function's after any
// `finally` its `return` runs; the file's before each `return` and at its end) one that records
// which bindings the variables hold. Per binding (by `start`, its name node's first byte):
//   reaching  the bindings some run reached it with (-1: the variable undefined)
//   end       whether some run ended its scope (a `return`, or the end of the body or file) with
//             the variable holding it
//   outer     (in a named function, a method, or a closure written at the top level) the values
//             of: after the binding, the variable is one reference with the global variable of
//             its name (ReflectionReference ids), on the runs that made it
// Fixtures bind only in statements of their own (no binding inside a condition, an operand or an
// arrow function) and make every path a run can take, so a flow that reads every path can agree.
// Run offline, not in CI, from this directory:
//   /opt/homebrew/Cellar/php/8.5.11/bin/php -n generate_flow.php <composer vendor/autoload.php>
// with nikic/php-parser 5.9.0 installed by that Composer project.

declare(strict_types=1);

use PhpParser\Node;
use PhpParser\Node\Expr;
use PhpParser\Node\Name;
use PhpParser\Node\Stmt;
use PhpParser\ParserFactory;
use PhpParser\PhpVersion;
use PhpParser\PrettyPrinter;

if ($argc !== 2) {
    fwrite(STDERR, "usage: php -n generate_flow.php <vendor/autoload.php>\n");
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

/** The probes' recorder, run before the fixture. */
const HARNESS = <<<'PHP'
function __rerail_reach(int $tag, ?int $previous): void
{
    $GLOBALS['__rerail']['reach'][$tag][$previous ?? -1] = true;
}
function __rerail_outer(int $tag, string $name, array $vars): void
{
    $local = array_key_exists($name, $vars) ? ReflectionReference::fromArrayElement($vars, $name)?->getId() : null;
    $global = array_key_exists($name, $GLOBALS) ? ReflectionReference::fromArrayElement($GLOBALS, $name)?->getId() : null;
    $GLOBALS['__rerail']['outer'][$tag][$local !== null && $local === $global ? 'true' : 'false'] = true;
}
function __rerail_end(array $tags): void
{
    foreach ($tags as $tag) {
        $GLOBALS['__rerail']['end'][$tag] = true;
    }
}
function __rerail_inputs(ReflectionFunction $function): array
{
    $runs = [[]];
    foreach ($function->getParameters() as $parameter) {
        $values = match ((string) $parameter->getType()) {
            'bool' => [false, true],
            'int' => [0, 1, 2, 3],
            'array' => [[], [1], [2], [1, 1], [1, 2], [2, 1], [2, 2]],
        };
        $next = [];
        foreach ($runs as $run) {
            foreach ($values as $value) {
                $next[] = [...$run, $value];
            }
        }
        $runs = $next;
    }
    return $runs;
}
function __rerail_run(): void
{
    foreach (get_defined_functions()['user'] as $name) {
        if (!str_starts_with($name, 'case_')) {
            continue;
        }
        foreach (__rerail_inputs(new ReflectionFunction($name)) as $inputs) {
            try {
                $name(...$inputs);
            } catch (Throwable) {
            }
        }
    }
}
PHP;

final class Instrumenter
{
    /** @var array<int, string> each binding's name, by tag */
    public array $names = [];
    /** @var array<int, bool> whether the binding's outer probe runs, by tag */
    public array $outer = [];
    /** Instrumenting the file's own statements, not a function's. */
    private bool $file = false;

    public function __construct(private readonly PhpParser\Parser $parser)
    {
    }

    /** @return list<Stmt> */
    private function code(string $code): array
    {
        return $this->parser->parse("<?php\n$code");
    }

    private static function name(Node $node): ?string
    {
        if (!$node instanceof Expr\Variable || !is_string($node->name)) {
            return null;
        }
        return $node->name === 'this' || in_array($node->name, SUPERGLOBALS, true) ? null : $node->name;
    }

    /** The probes after `$var` is bound; `outer` where the scope's outer probe runs; `defined`:
     * only where the variable is defined after the statement (`unset($a[k])` changes a defined
     * array and defines no variable). */
    private function probe(Expr\Variable $var, bool $outer, bool $defined = false): array
    {
        $name = self::name($var);
        $tag = $var->getStartFilePos();
        $this->names[$tag] = $name;
        $this->outer[$tag] = $outer;
        $code = sprintf("\\__rerail_reach(%d, \$__rerail_t[%s] ?? null);\n\$__rerail_t[%s] = %d;\n", $tag, var_export($name, true), var_export($name, true), $tag);
        if ($outer) {
            $code .= sprintf("\\__rerail_outer(%d, %s, \\get_defined_vars());\n", $tag, var_export($name, true));
        }
        if ($defined) {
            $code = sprintf("if (\\array_key_exists(%s, \\get_defined_vars())) {\n%s}\n", var_export($name, true), $code);
        }
        return $this->code($code);
    }

    /** The variables a statement binds, in order, those an `unset($a[k])` changes where they are
     * defined, and those it unsets. @return array{list<Expr\Variable>, list<Expr\Variable>, list<string>} */
    private function bindings(Stmt $stmt): array
    {
        $bound = [];
        $changed = [];
        $unset = [];
        $target = function (Node $node) use (&$bound, &$target): void {
            if ($node instanceof Expr\Variable && self::name($node) !== null) {
                $bound[] = $node;
            } elseif ($node instanceof Expr\List_) {
                foreach ($node->items as $item) {
                    if ($item !== null) {
                        $target($item->value);
                    }
                }
            } elseif ($node instanceof Expr\ArrayDimFetch) {
                $target($node->var);
            }
        };
        if ($stmt instanceof Stmt\Expression) {
            $expr = $stmt->expr;
            if ($expr instanceof Expr\Assign || $expr instanceof Expr\AssignRef || $expr instanceof Expr\AssignOp
                || $expr instanceof Expr\PreInc || $expr instanceof Expr\PostInc
                || $expr instanceof Expr\PreDec || $expr instanceof Expr\PostDec) {
                $target($expr->var);
            } elseif ($expr instanceof Expr\FuncCall && $expr->name instanceof Name
                && $expr->name->toLowerString() !== 'extract'
                && function_exists($expr->name->toString())
                && ($function = new ReflectionFunction($expr->name->toString()))->isInternal()) {
                // A function PHP defines binds the variables it takes by reference: a named
                // argument the parameter it names, any other the one at its position.
                $parameters = $function->getParameters();
                $byName = [];
                foreach ($parameters as $parameter) {
                    $byName[$parameter->getName()] = $parameter;
                }
                $last = end($parameters);
                $position = 0;
                foreach ($expr->args as $arg) {
                    if (!$arg instanceof Node\Arg || $arg->unpack) {
                        continue;
                    }
                    $parameter = $arg->name !== null
                        ? ($byName[$arg->name->toString()] ?? null)
                        : ($parameters[$position++] ?? ($last !== false && $last->isVariadic() ? $last : null));
                    if ($parameter?->isPassedByReference()) {
                        $target($arg->value);
                    }
                }
            }
        } elseif ($stmt instanceof Stmt\Global_) {
            foreach ($stmt->vars as $var) {
                $target($var);
            }
        } elseif ($stmt instanceof Stmt\Static_) {
            foreach ($stmt->vars as $var) {
                $target($var->var);
            }
        } elseif ($stmt instanceof Stmt\Unset_) {
            foreach ($stmt->vars as $var) {
                if ($var instanceof Expr\ArrayDimFetch) {
                    $before = count($bound);
                    $target($var);
                    array_push($changed, ...array_splice($bound, $before));
                } elseif (($name = self::name($var)) !== null) {
                    $unset[] = $name;
                }
            }
        }
        return [$bound, $changed, $unset];
    }

    private function end(): array
    {
        return $this->code("\\__rerail_end(\$__rerail_t);\n");
    }

    /** Instruments a function-like body: its start binds the parameters (and `use` variables).
     * A function's end is recorded where it ends normally, after any `finally` a `return` runs;
     * the file's, before each `return` and at its end. */
    public function body(array $stmts, array $bound, bool $outer, bool $top, bool $file = false): array
    {
        $start = $this->code("\$__rerail_t = [];\n");
        foreach ($bound as $var) {
            array_push($start, ...$this->probe($var, $outer));
        }
        $enclosing = $this->file;
        $this->file = $file;
        $body = $this->statements($stmts, $outer, $top);
        $this->file = $enclosing;
        if ($file) {
            return [...$start, ...$body, ...$this->end()];
        }
        $guard = $this->code(
            "try {\n} catch (\\Throwable \$__rerail_e) {\n    \$__rerail_thrown = true;\n    throw \$__rerail_e;\n}"
            . " finally {\n    if (!isset(\$__rerail_thrown)) {\n        \\__rerail_end(\$__rerail_t);\n    }\n}\n",
        )[0];
        $guard->stmts = $body;
        return [...$start, $guard];
    }

    /** @param list<Stmt> $stmts */
    private function statements(array $stmts, bool $outer, bool $top): array
    {
        $result = [];
        foreach ($stmts as $stmt) {
            $this->nested($stmt, $outer, $top);
            if ($this->file && $stmt instanceof Stmt\Return_) {
                array_push($result, ...$this->end());
            }
            $result[] = $stmt;
            [$bound, $changed, $unset] = $this->bindings($stmt);
            foreach ($bound as $var) {
                array_push($result, ...$this->probe($var, $outer));
            }
            foreach ($changed as $var) {
                array_push($result, ...$this->probe($var, $outer, true));
            }
            foreach ($unset as $name) {
                array_push($result, ...$this->code(sprintf("unset(\$__rerail_t[%s]);\n", var_export($name, true))));
            }
        }
        return $result;
    }

    /** Instruments the statement lists and function-likes inside a statement. */
    private function nested(Node $node, bool $outer, bool $top): void
    {
        switch (true) {
            case $node instanceof Stmt\Function_:
            case $node instanceof Stmt\ClassMethod:
                if ($node->stmts !== null) {
                    $node->stmts = $this->body($node->stmts, array_map(fn ($param) => $param->var, $node->params), true, false);
                }
                return;
            case $node instanceof Expr\Closure:
                $vars = [...array_map(fn ($param) => $param->var, $node->params), ...array_map(fn ($use) => $use->var, $node->uses)];
                // Only a closure written at the top level has the global scope around it.
                $node->stmts = $this->body($node->stmts, $vars, $top, false);
                foreach ($node->uses as $use) {
                    $this->nested($use, $outer, $top);
                }
                return;
            case $node instanceof Expr\ArrowFunction:
                throw new RuntimeException('flow fixtures have no arrow functions');
            case $node instanceof Stmt\Foreach_:
                $this->nested($node->expr, $outer, $top);
                $targets = [];
                $collect = function (?Node $target) use (&$targets, &$collect): void {
                    if ($target instanceof Expr\Variable && self::name($target) !== null) {
                        $targets[] = $target;
                    } elseif ($target instanceof Expr\List_) {
                        foreach ($target->items as $item) {
                            if ($item !== null) {
                                $collect($item->value);
                            }
                        }
                    }
                };
                $collect($node->keyVar);
                $collect($node->valueVar);
                $probes = [];
                foreach ($targets as $var) {
                    array_push($probes, ...$this->probe($var, $outer));
                }
                $node->stmts = [...$probes, ...$this->statements($node->stmts, $outer, $top)];
                return;
            case $node instanceof Stmt\Catch_:
                $probes = $node->var !== null ? $this->probe($node->var, $outer) : [];
                $node->stmts = [...$probes, ...$this->statements($node->stmts, $outer, $top)];
                return;
        }
        foreach ($node->getSubNodeNames() as $name) {
            $child = $node->$name;
            if ($name === 'stmts' && is_array($child)) {
                $node->$name = $this->statements($child, $outer, $top);
            } elseif ($child instanceof Node) {
                $this->nested($child, $outer, $top);
            } elseif (is_array($child)) {
                foreach ($child as $item) {
                    if ($item instanceof Node) {
                        $this->nested($item, $outer, $top);
                    }
                }
            }
        }
    }
}

$parser = (new ParserFactory())->createForVersion(PhpVersion::fromComponents(8, 5));
$printer = new PrettyPrinter\Standard();
$files = [];
$fixtures = glob(__DIR__ . '/flow/*.php');
sort($fixtures, SORT_STRING);
foreach ($fixtures as $path) {
    $instrumenter = new Instrumenter($parser);
    $statements = $instrumenter->body($parser->parse(file_get_contents($path)), [], false, true, true);
    $program = "<?php\n" . HARNESS . "\nob_start();\n" . $printer->prettyPrint($statements)
        . "\n__rerail_run();\nob_end_clean();\necho json_encode(\$GLOBALS['__rerail'] ?? []);\n";
    $script = tempnam(sys_get_temp_dir(), 'rerail-flow-');
    file_put_contents($script, $program);
    $output = shell_exec(escapeshellarg(PHP_BINARY) . ' -n -d error_reporting=0 ' . escapeshellarg($script));
    unlink($script);
    $trace = json_decode((string) $output, true, flags: JSON_THROW_ON_ERROR);
    $records = [];
    foreach ($instrumenter->names as $tag => $name) {
        if (!isset($trace['reach'][$tag])) {
            throw new RuntimeException(basename($path) . ": the binding of \$$name at byte $tag never ran");
        }
        $reaching = array_map('intval', array_keys($trace['reach'][$tag]));
        sort($reaching);
        $record = [
            'start' => $tag,
            'name' => '$' . $name,
            'reaching' => $reaching,
            'end' => isset($trace['end'][$tag]),
        ];
        if ($instrumenter->outer[$tag]) {
            $outer = array_map(fn ($value) => $value === 'true', array_keys($trace['outer'][$tag]));
            sort($outer);
            $record['outer'] = $outer;
        }
        $records[] = $record;
    }
    usort($records, fn ($a, $b) => $a['start'] <=> $b['start']);
    $files[basename($path)] = $records;
}

file_put_contents(
    __DIR__ . '/flow.expected.json',
    json_encode(
        ['php' => PHP_VERSION, 'phpParser' => $parserVersion, 'files' => $files],
        JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR,
    ) . "\n",
);
