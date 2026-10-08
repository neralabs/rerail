<?php
// The runtime half of the call-target oracle: runs project/main.php under PHP 8.5.11 with every
// function, method, closure and arrow function instrumented, and prints, as JSON, each call site
// (file and line) with the declarations (file and start line) PHP dispatched it to. A callee
// invoked by one of PHP's functions (call_user_func, call_user_func_array) is credited to that
// function's call site. Line numbers are the source's: the instrumentation adds no line.
// Run offline by generate_php.py:
//   php trace.php <composer vendor/autoload.php with nikic/php-parser 5.9.0> <workspace directory>

declare(strict_types=1);

use PhpParser\Node;
use PhpParser\NodeTraverser;
use PhpParser\NodeVisitorAbstract;
use PhpParser\ParserFactory;
use PhpParser\PhpVersion;

if ($argc !== 3) {
    fwrite(STDERR, "usage: php trace.php <vendor/autoload.php> <workspace>\n");
    exit(2);
}
require $argv[1];
$workspace = realpath($argv[2]);
$copy = sys_get_temp_dir() . '/rerail-calls-trace-' . getmypid();
@mkdir($copy);
$parser = (new ParserFactory())->createForVersion(PhpVersion::fromString('8.5'));

final class Instrument extends NodeVisitorAbstract
{
    /** @var list<array{int, string}> insertions: byte offset, text */
    public array $insertions = [];

    /** @param list<\PhpParser\Token> $tokens */
    public function __construct(private array $tokens, private string $file)
    {
    }

    public function enterNode(Node $node)
    {
        $trace = sprintf('\__rerail_trace(%s, %d)', var_export($this->file, true), $node->getStartLine());
        if ($node instanceof Node\Expr\ArrowFunction) {
            $this->insertions[] = [$node->expr->getStartFilePos(), "($trace) ?: ("];
            $this->insertions[] = [$node->expr->getEndFilePos() + 1, ')'];
            return null;
        }
        if (!($node instanceof Node\Stmt\Function_ || $node instanceof Node\Stmt\ClassMethod
            || $node instanceof Node\Expr\Closure) || $node->getStmts() === null) {
            return null;
        }
        // The body's `{`: the first after the signature.
        $from = $node->getStartTokenPos();
        foreach (array_merge($node->params, $node instanceof Node\Expr\Closure ? $node->uses : [],
            $node->returnType ? [$node->returnType] : []) as $part) {
            $from = max($from, $part->getEndTokenPos());
        }
        for ($i = $from; $i < count($this->tokens); $i++) {
            if ($this->tokens[$i]->text === '{') {
                $this->insertions[] = [$this->tokens[$i]->pos + 1, " $trace;"];
                return null;
            }
        }
        throw new RuntimeException("no body brace in {$this->file} at line {$node->getStartLine()}");
    }
}

foreach (glob($workspace . '/*.php') as $path) {
    $source = file_get_contents($path);
    $statements = $parser->parse($source);
    $visitor = new Instrument($parser->getTokens(), basename($path));
    $traverser = new NodeTraverser();
    $traverser->addVisitor($visitor);
    $traverser->traverse($statements);
    usort($visitor->insertions, static fn (array $a, array $b): int => $b[0] <=> $a[0]);
    foreach ($visitor->insertions as [$at, $text]) {
        $source = substr($source, 0, $at) . $text . substr($source, $at);
    }
    file_put_contents($copy . '/' . basename($path), $source);
}

$prelude = <<<'PHP'
<?php
$GLOBALS['__rerail_calls'] = [];
function __rerail_trace(string $file, int $line): void
{
    $frames = debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS, 3);
    $site = $frames[1] ?? [];
    if (!isset($site['file'])) {
        $site = $frames[2] ?? [];
    }
    if (isset($site['file'])) {
        $GLOBALS['__rerail_calls'][basename($site['file']) . ':' . $site['line']][$file . ':' . $line] = true;
    }
}
register_shutdown_function(static function (): void {
    $calls = [];
    foreach ($GLOBALS['__rerail_calls'] as $site => $callees) {
        [$file, $line] = explode(':', $site);
        $targets = array_map(static fn (string $callee): array => [explode(':', $callee)[0], (int) explode(':', $callee)[1]], array_keys($callees));
        sort($targets);
        $calls[$file][] = ['line' => (int) $line, 'callees' => $targets];
    }
    ksort($calls);
    foreach ($calls as &$sites) {
        usort($sites, static fn (array $a, array $b): int => $a['line'] <=> $b['line']);
    }
    echo json_encode($calls, JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR);
});
PHP;
file_put_contents($copy . '/__prelude.php', $prelude);
passthru(sprintf('%s -d auto_prepend_file=%s %s', escapeshellarg(PHP_BINARY),
    escapeshellarg($copy . '/__prelude.php'), escapeshellarg($copy . '/main.php')), $status);
array_map('unlink', glob($copy . '/*.php'));
rmdir($copy);
exit($status);
