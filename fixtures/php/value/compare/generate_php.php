<?php
// Writes php.expected.json: what `compare`'s PHP modes read, for every case in php.cases.json,
// as nikic/PHP-Parser 5.9.0 parses the code and PHP 8.5.11 evaluates and looks up what it names.
// Run offline, not in CI, with PHP-Parser installed by Composer:
//   composer require --working-dir=/tmp/rerail-php-tools nikic/php-parser:5.9.0
//   RERAIL_PHP_TOOLS=/tmp/rerail-php-tools /opt/homebrew/Cellar/php/8.5.11/bin/php generate_php.php
//
// A case's node is the innermost PHP-Parser node spanning exactly the `nth` (default 0)
// occurrence of `at` in its source. Names are recorded as hex bytes, `null` for none.
// - `keys` (`compare` text `key`): the name of the property, method, class constant, array key or
//   named argument the node holds or is. A computed name is the value PHP gives its expression
//   as a constant expression (`const X = <expr>;`, each in a fresh `php -n`), converted as PHP
//   converts it where it uses it: through `__get` for a property, `__call` for a method, a
//   dynamic class constant fetch, an array literal's key; a warning is a refusal. An expression
//   PHP does not accept as a constant expression has no name.
// - `members` (`member`): the same, where the computed name's expression is evaluated after the
//   program's statements before the last one (which holds the node) ran, so variables have values.
// - `names` (`name`): `pairs` record whether PHP finds the same symbol for two names written in
//   different case, each decided by running the case's probe (`true` printed means the same; a
//   compile error prints nothing); `strings` the value of a string literal (`const X = <literal>;`).
// - `references` (`reference`, `reference-no-static-keys`): whether two expressions are the
//   same reference, by the definition in `value/php_compare.rs` applied to PHP-Parser's trees, with
//   literals compared by PHP's `===` and keys converted by PHP as above.
// - `structures` (`structure`, `structure-hash`): whether PHP-Parser's trees of two snippets are
//   equal once every attribute is stripped (`equal`), and the same under PHP's array key identity
//   (`hash`): a constant expression without named constants (`const X = <expr>;`) compares by the
//   key PHP coerces its value to, anything else by its tree.

$tools = getenv('RERAIL_PHP_TOOLS') ?: '/tmp/rerail-php-tools';
require "$tools/vendor/autoload.php";

use PhpParser\Node;
use PhpParser\Node\Expr;
use PhpParser\Node\Scalar;
use PhpParser\Node\Stmt;
use PhpParser\NodeFinder;
use PhpParser\NodeTraverser;
use PhpParser\NodeVisitor\ParentConnectingVisitor;
use PhpParser\ParserFactory;
use PhpParser\PhpVersion;

const PARSER_VERSION = '5.9.0';

$installed = \Composer\InstalledVersions::getPrettyVersion('nikic/php-parser');
if (ltrim($installed, 'v') !== PARSER_VERSION) {
    fwrite(STDERR, "nikic/php-parser $installed installed; this oracle pins " . PARSER_VERSION . "\n");
    exit(1);
}

$parser = (new ParserFactory())->createForVersion(PhpVersion::fromString('8.5'));

function parse(string $source): array
{
    global $parser;
    $statements = $parser->parse($source);
    $traverser = new NodeTraverser(new ParentConnectingVisitor());
    return $traverser->traverse($statements);
}

/**
 * The innermost node spanning exactly the `nth` occurrence of `at`, where an occurrence that
 * begins or ends with a word character stands on a word boundary there.
 */
function node_at(array $statements, string $source, string $at, int $nth): Node
{
    $pattern = '/' . (preg_match('/^\w/', $at) ? '(?<!\w)' : '') . preg_quote($at, '/')
        . (preg_match('/\w$/', $at) ? '(?!\w)' : '') . '/';
    preg_match_all($pattern, $source, $matches, PREG_OFFSET_CAPTURE);
    if (!isset($matches[0][$nth])) {
        throw new RuntimeException("no occurrence $nth of `$at` in `$source`");
    }
    $offset = $matches[0][$nth][1];
    $end = $offset + strlen($at) - 1;
    $found = null;
    foreach ((new NodeFinder())->find($statements, fn () => true) as $node) {
        if ($node->getStartFilePos() === $offset && $node->getEndFilePos() === $end) {
            $found = $node;
        }
    }
    if ($found === null) {
        throw new RuntimeException("no node spans `$at` in `$source`");
    }
    return $found;
}

function text_of(Node $node, string $source): string
{
    return substr($source, $node->getStartFilePos(), $node->getEndFilePos() - $node->getStartFilePos() + 1);
}

/** Runs PHP code in a fresh `php -n`; its standard output. */
function run(string $code): string
{
    $process = proc_open(
        [PHP_BINARY, '-n', '-d', 'error_reporting=-1', '-d', 'display_errors=stderr', '-r', $code],
        [1 => ['pipe', 'w'], 2 => ['pipe', 'w']],
        $pipes,
    );
    $out = stream_get_contents($pipes[1]);
    stream_get_contents($pipes[2]);
    fclose($pipes[1]);
    fclose($pipes[2]);
    proc_close($process);
    return $out;
}

/** How PHP converts a computed name, in the process that evaluates it: `$v` holds the value. */
const CONVERT = <<<'PHP'
set_error_handler(function (int $level, string $message) {
    if ($level & (E_WARNING | E_USER_WARNING)) {
        throw new ErrorException($message);
    }
    return true;
});
final class RerailMagic {
    const X = 0;
    public function __get($name) { return $name; }
    public function __call($name, $arguments) { return $name; }
}
function rerail_convert(mixed $v, string $role): ?string {
    try {
        return match ($role) {
            'property' => (new RerailMagic())->{$v},
            'static' => (string) $v,
            'method' => (new RerailMagic())->{$v}(),
            'constant' => (function ($v) {
                try {
                    RerailMagic::{$v};
                } catch (TypeError $e) {
                    throw $e;
                } catch (Error $e) {
                }
                return $v;
            })($v),
            'key' => (string) array_key_first([$v => 0]),
            'expression' => is_array($v) ? throw new TypeError('array') : (string) $v,
        };
    } catch (Throwable $e) {
        return null;
    }
}
PHP;

/** The name a computed key's expression converts to, or null. */
function computed(string $prelude, string $expression, string $role, bool $constant): ?string
{
    $value = $constant ? "const RERAIL_X = $expression;\n\$v = RERAIL_X;" : "\$v = ($expression);";
    $out = run($prelude . "\n" . CONVERT . "\n$value\n\$name = rerail_convert(\$v, '$role');\necho json_encode(\$name === null ? null : bin2hex(\$name));");
    $decoded = json_decode($out, true);
    return is_string($decoded) ? $decoded : null;
}

/** What holds the key a node is, where it stands in a key position: the holder; else null. */
function holder(Node $node): ?Node
{
    $parent = $node->getAttribute('parent');
    if ($parent === null) {
        return null;
    }
    $is = fn ($child) => $child === $node;
    return match (true) {
        $parent instanceof Expr\PropertyFetch,
        $parent instanceof Expr\NullsafePropertyFetch,
        $parent instanceof Expr\MethodCall,
        $parent instanceof Expr\NullsafeMethodCall,
        $parent instanceof Expr\StaticCall,
        $parent instanceof Expr\StaticPropertyFetch,
        $parent instanceof Expr\ClassConstFetch,
        $parent instanceof Node\Arg,
        $parent instanceof Stmt\ClassMethod,
        $parent instanceof Stmt\EnumCase,
        $parent instanceof Node\PropertyItem,
        $parent instanceof Node\Const_ => $is($parent->name) ? $parent : null,
        $parent instanceof Node\ArrayItem => $is($parent->key) ? $parent : null,
        $parent instanceof Expr\ArrayDimFetch => $is($parent->dim) ? $parent : null,
        default => null,
    };
}

/** A key: ['written', name] | ['computed', expr, role] | ['nameless']. */
function key_of_node(Node $node): array
{
    $member = fn ($name, string $role) => $name instanceof Node\Identifier || $name instanceof Node\VarLikeIdentifier
        ? ['written', $name->toString()]
        : ['computed', $name, $role];
    return match (true) {
        $node instanceof Expr\PropertyFetch, $node instanceof Expr\NullsafePropertyFetch => $member($node->name, 'property'),
        $node instanceof Expr\MethodCall, $node instanceof Expr\NullsafeMethodCall, $node instanceof Expr\StaticCall => $member($node->name, 'method'),
        $node instanceof Expr\StaticPropertyFetch => $member($node->name, 'static'),
        $node instanceof Expr\ClassConstFetch => $member($node->name, 'constant'),
        $node instanceof Node\ArrayItem => $node->key === null ? ['nameless'] : ['computed', $node->key, 'key'],
        $node instanceof Expr\ArrayDimFetch => $node->dim === null ? ['nameless'] : ['computed', $node->dim, 'key'],
        $node instanceof Node\Arg => $node->name === null ? ['nameless'] : ['written', $node->name->toString()],
        $node instanceof Stmt\ClassMethod, $node instanceof Stmt\EnumCase, $node instanceof Node\PropertyItem, $node instanceof Node\Const_ => ['written', $node->name->toString()],
        $node instanceof Node\Param => $node->flags !== 0 ? ['written', $node->var->name] : ['nameless'],
        default => ($holder = holder($node)) !== null
            ? key_of_node($holder)
            : ($node instanceof Node\Identifier || $node instanceof Node\VarLikeIdentifier || $node instanceof Node\Name
                ? ['written', $node->toString()]
                : ['computed', $node, 'expression']),
    };
}

/** A key's name as hex, or null. */
function key_name(Node $node, string $source, string $prelude, bool $constant): ?string
{
    $key = key_of_node($node);
    return match ($key[0]) {
        'nameless' => null,
        'written' => bin2hex($key[1]),
        'computed' => computed($prelude, text_of($key[1], $source), $key[2], $constant),
    };
}

$here = __DIR__;
$cases = json_decode(file_get_contents("$here/php.cases.json"), true, flags: JSON_THROW_ON_ERROR);
$out = ['php' => PHP_VERSION, 'phpParser' => PARSER_VERSION];

foreach ($cases['keys'] as $case) {
    $statements = parse($case['src']);
    $node = node_at($statements, $case['src'], $case['at'], $case['nth'] ?? 0);
    $out['keys'][] = $case + ['name' => key_name($node, $case['src'], '', true)];
}

foreach ($cases['members'] as $case) {
    $source = $case['src'];
    $statements = parse($source);
    $node = node_at($statements, $source, $case['at'], $case['nth'] ?? 0);
    $last = end($statements);
    $prelude = substr($source, 0, $last->getStartFilePos());
    $prelude = preg_replace('/^<\?php\s*/', '', $prelude);
    $out['members'][] = $case + ['name' => key_name($node, $source, $prelude, false)];
}

foreach ($cases['names']['pairs'] as $case) {
    $out['names']['pairs'][] = $case + ['same' => trim(run($case['probe'])) === 'true'];
}

foreach ($cases['names']['strings'] as $case) {
    $value = json_decode(run("const RERAIL_X = {$case['at']};\necho json_encode(is_string(RERAIL_X) ? bin2hex(RERAIL_X) : null);"), true);
    $out['names']['strings'][] = $case + ['name' => is_string($value) ? $value : null];
}

// ---- references -----------------------------------------------------------------------------

function expression(string $source): Expr
{
    $statements = parse("<?php $source;");
    if (count($statements) !== 1 || !$statements[0] instanceof Stmt\Expression) {
        throw new RuntimeException("`$source` is not one expression");
    }
    return $statements[0]->expr;
}

function is_literal_constant(Node $node): bool
{
    return $node instanceof Expr\ConstFetch
        && !$node->name->isFullyQualified() && !$node->name->isRelative() && !$node->name->isQualified()
        && in_array(strtolower($node->name->toString()), ['true', 'false', 'null'], true);
}

function shape(Node $node): string
{
    return match (true) {
        $node instanceof Expr\Variable => is_string($node->name) ? 'variable' : 'dynamic',
        is_literal_constant($node) => 'literal',
        $node instanceof Expr\ConstFetch => 'constant',
        $node instanceof Scalar\String_, $node instanceof Scalar\Int_, $node instanceof Scalar\Float_ => 'literal',
        $node instanceof Expr\PropertyFetch, $node instanceof Expr\NullsafePropertyFetch => 'property',
        $node instanceof Expr\StaticPropertyFetch => 'static',
        $node instanceof Expr\ClassConstFetch => 'constant-fetch',
        $node instanceof Expr\ArrayDimFetch => 'element',
        default => 'other',
    };
}

/** A literal's value, as PHP evaluates it. */
function literal_value(Node $node, string $source): mixed
{
    return unserialize(run('echo serialize(' . text_of($node, $source) . ');'));
}

/** A constant's name as PHP looks it up: its namespace folded, its own name as written. */
function constant_name(Node\Name $name): string
{
    $code = $name->toCodeString();
    $at = strrpos($code, '\\');
    return $at === false ? $code : strtolower(substr($code, 0, $at)) . substr($code, $at);
}

/** A written or literal key, converted: what `static_keys` compares. */
function static_key(array $key, string $source): ?string
{
    if ($key[0] === 'written') {
        return bin2hex($key[1]);
    }
    if ($key[0] !== 'computed' || shape($key[1]) !== 'literal') {
        return null;
    }
    return computed('', text_of($key[1], $source), $key[2], true);
}

function same_key(array $left, array $right, bool $static, string $leftSource, string $rightSource): bool
{
    if ($static && ($name = static_key($left, $leftSource)) !== null) {
        return static_key($right, $rightSource) === $name;
    }
    if ($left[0] === 'written' && $right[0] === 'written') {
        return $left[1] === $right[1];
    }
    if ($left[0] === 'computed' && $right[0] === 'computed') {
        return same($left[1], $right[1], $static, $leftSource, $rightSource);
    }
    return false;
}

function same_scope(Node $left, Node $right, bool $static, string $leftSource, string $rightSource): bool
{
    $class = fn (Node $node) => $node instanceof Node\Name ? strtolower($node->toCodeString()) : null;
    [$l, $r] = [$class($left), $class($right)];
    if ($l !== null && $r !== null) {
        return $l === $r;
    }
    if ($l === null && $r === null) {
        return same($left, $right, $static, $leftSource, $rightSource);
    }
    return false;
}

function same(Node $left, Node $right, bool $static, string $leftSource, string $rightSource): bool
{
    $kind = shape($left);
    if ($kind !== shape($right)) {
        return false;
    }
    return match ($kind) {
        'variable' => $left->name === $right->name,
        'dynamic' => same($left->name, $right->name, $static, $leftSource, $rightSource),
        'constant' => constant_name($left->name) === constant_name($right->name),
        'literal' => literal_value($left, $leftSource) === literal_value($right, $rightSource),
        'property' => same($left->var, $right->var, $static, $leftSource, $rightSource)
            && same_key(key_of_node($left), key_of_node($right), $static, $leftSource, $rightSource),
        'static', 'constant-fetch' => same_scope($left->class, $right->class, $static, $leftSource, $rightSource)
            && same_key(key_of_node($left), key_of_node($right), $static, $leftSource, $rightSource),
        'element' => $left->dim !== null && $right->dim !== null
            && same($left->var, $right->var, $static, $leftSource, $rightSource)
            && same_key(key_of_node($left), key_of_node($right), $static, $leftSource, $rightSource),
        default => false,
    };
}

foreach ($cases['references'] as [$left, $right]) {
    $l = expression($left);
    $r = expression($right);
    [$ls, $rs] = ["<?php $left;", "<?php $right;"];
    $out['references'][] = [
        'left' => $left,
        'right' => $right,
        'same' => same($l, $r, true, $ls, $rs),
        'sameNoStaticKeys' => same($l, $r, false, $ls, $rs),
    ];
}

// ---- structures -----------------------------------------------------------------------------

/** A tree without attributes: every node's type and sub-nodes. */
function stripped(mixed $node): mixed
{
    if ($node instanceof Node) {
        $out = ['type' => $node->getType()];
        foreach ($node->getSubNodeNames() as $name) {
            $out[$name] = stripped($node->$name);
        }
        return $out;
    }
    return is_array($node) ? array_map('stripped', $node) : $node;
}

/** The array key a constant expression without named constants is coerced to; null otherwise. */
function folded(Node $node, string $source): ?array
{
    $named = (new NodeFinder())->findFirst($node, fn (Node $n) => ($n instanceof Expr\ConstFetch && !is_literal_constant($n))
        || $n instanceof Expr\ClassConstFetch || $n instanceof Scalar\MagicConst);
    if ($named !== null) {
        return null;
    }
    $text = text_of($node, $source);
    $out = run("const RERAIL_X = $text;\nif (is_array(RERAIL_X)) { echo 'null'; exit; }\n\$k = array_key_first([RERAIL_X => 0]);\necho json_encode([get_debug_type(\$k), is_int(\$k) ? (string) \$k : bin2hex(\$k)]);");
    $key = json_decode($out, true);
    return is_array($key) ? $key : null;
}

foreach ($cases['structures'] as $case) {
    $statement = $case['statement'] ?? false;
    $trees = [];
    $keys = [];
    foreach (['left', 'right'] as $side) {
        $source = $statement ? "<?php {$case[$side]}" : "<?php {$case[$side]};";
        $statements = parse($source);
        if (count($statements) !== 1) {
            throw new RuntimeException("`{$case[$side]}` is not one statement");
        }
        $node = $statement ? $statements[0] : $statements[0]->expr;
        $trees[] = stripped($node);
        $keys[] = $statement ? null : folded($node, $source);
    }
    $equal = $trees[0] === $trees[1];
    $hash = $keys[0] !== null || $keys[1] !== null ? $keys[0] === $keys[1] : $equal;
    $out['structures'][] = $case + ['equal' => $equal, 'hash' => $hash];
}

file_put_contents("$here/php.expected.json", json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE) . "\n");
