<?php
// The code Psalm 6.19.1 does not analyse, from its own reports, read with nikic/PHP-Parser 5.9.0.
// Called by generate_usage_corpus.py's `unanalysed()`:
//   php unanalysed.php <vendor/autoload.php> < {"<file>": [[<byte offset>, <issue type>], ...], ...}
// prints, per file read, its size, its class-like declarations and the functions Psalm stops analysing:
//   {"<file>": {"bytes": <size>, "lines": <line count>,
//               "classes": [[<fq name, lower case>, <parent's, or null>, <start byte>, <end byte>,
//                            <start line>, <end line>], ...],
//               "stopped": [[<start byte>, <end byte>, <start line>, <end line>], ...]}, ...}
// with the end byte exclusive; a file PHP-Parser cannot parse is null.
//
// Psalm analyses a function no further, though it still reports its parameters unused, where its
// analyser of an expression returns false. It reports one of these there: UndefinedVariable,
// PossiblyUndefinedVariable or DuplicateParam at a closure's `use` variable
// (ClosureAnalyzer::analyzeClosureUses; the function stopped is the one the closure is written in,
// the file at its top level), UndefinedClass at the class of a static call, of a static property
// fetch or of an `instanceof` (StaticCallAnalyzer, StaticPropertyFetchAnalyzer,
// InstanceofAnalyzer), NullReference at the name of a method called on null (MethodCallAnalyzer),
// InvalidThrow at a `throw` of what is not Throwable (ThrowAnalyzer).
// `function k(int $q): int { $h = function () use (&$late) { return $late; }; return $q + (int) $h(); }`
// gets UnusedParam for `$q`, which `echo k(5);` prints; so does `$q` in
// `function f(int $q): int { $z = 5; $b = \Nope::make(); return $q; }`, where `$z` and `$b` get no
// UnusedVariable. The same types anywhere else (a read of an undefined variable, `new` of an
// undefined class) stop nothing.

declare(strict_types=1);

require $argv[1];

use PhpParser\Node;
use PhpParser\Node\Expr;
use PhpParser\NodeTraverser;
use PhpParser\NodeVisitor\NameResolver;
use PhpParser\NodeVisitor\ParentConnectingVisitor;
use PhpParser\NodeVisitorAbstract;
use PhpParser\ParserFactory;

final class Reader extends NodeVisitorAbstract
{
    /** @var list<array{string, ?string, int, int, int, int}> */
    public array $classes = [];
    /** @var array<string, array{int, int, int, int}> */
    public array $stopped = [];
    /** @var array<int, list<string>> */
    private array $stops = [];

    /** @param list<array{int, string}> $stops */
    public function __construct(array $stops, private readonly string $source)
    {
        foreach ($stops as [$offset, $type]) {
            $this->stops[$offset][] = $type;
        }
    }

    public function enterNode(Node $node): null
    {
        if ($node instanceof Node\Stmt\ClassLike) {
            $parent = $node instanceof Node\Stmt\Class_ ? $node->extends?->toString() : null;
            $this->classes[] = [
                strtolower($node->namespacedName?->toString() ?? ''), $parent === null ? null : strtolower($parent),
                $node->getStartFilePos(), $node->getEndFilePos() + 1, $node->getStartLine(), $node->getEndLine(),
            ];
        }
        $use = $node instanceof Node\ClosureUse ? $node->var : null;
        $stop = match (true) {
            $use !== null => $this->at($use, ['UndefinedVariable', 'PossiblyUndefinedVariable', 'DuplicateParam']),
            $node instanceof Expr\StaticCall, $node instanceof Expr\StaticPropertyFetch, $node instanceof Expr\Instanceof_
                => $this->at($node->class, ['UndefinedClass']),
            $node instanceof Expr\MethodCall => $this->at($node->name, ['NullReference']),
            $node instanceof Expr\Throw_ => $this->at($node, ['InvalidThrow']),
            default => false,
        };
        if ($stop) {
            // Past a `use` clause's closure, to the function it is written in.
            $function = $this->function($use !== null ? $node->getAttribute('parent') : $node);
            $range = $function === null
                ? [0, strlen($this->source), 1, substr_count($this->source, "\n") + 1]
                : [$function->getStartFilePos(), $function->getEndFilePos() + 1, $function->getStartLine(), $function->getEndLine()];
            $this->stopped[implode(':', $range)] = $range;
        }
        return null;
    }

    /** @param list<string> $types */
    private function at(Node $node, array $types): bool
    {
        return array_intersect($this->stops[$node->getStartFilePos()] ?? [], $types) !== [];
    }

    private function function(Node $node): ?Node\FunctionLike
    {
        for ($node = $node->getAttribute('parent'); $node !== null; $node = $node->getAttribute('parent')) {
            if ($node instanceof Node\FunctionLike) {
                return $node;
            }
        }
        return null;
    }
}

$parser = (new ParserFactory())->createForNewestSupportedVersion();
$out = [];
foreach (json_decode(stream_get_contents(STDIN), true, flags: JSON_THROW_ON_ERROR) as $file => $stops) {
    $source = file_get_contents($file);
    try {
        $statements = $parser->parse($source);
    } catch (PhpParser\Error) {
        $out[$file] = null;
        continue;
    }
    $reader = new Reader($stops, $source);
    $traverser = new NodeTraverser(new ParentConnectingVisitor(), new NameResolver(), $reader);
    $traverser->traverse($statements);
    $out[$file] = ['bytes' => strlen($source), 'lines' => substr_count($source, "\n") + 1,
        'classes' => $reader->classes, 'stopped' => array_values($reader->stopped)];
}
echo json_encode($out, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_INVALID_UTF8_SUBSTITUTE);
