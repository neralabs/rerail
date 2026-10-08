<?php
// Writes names.expected.json: for every names/*.php fixture, each name nikic/PHP-Parser 5.9.0's
// NameResolver resolves (the name resolution PHPStan runs), as PHP 8.5.11 reads the file:
//   start, end  byte range (startFilePos, endFilePos + 1)
//   text        the name as written
//   kind        class, function or constant, from the node that holds it (FuncCall: function,
//               ConstFetch: constant, anything else: class)
//   resolved    resolvedName (fully qualified, no leading backslash; a special class name as
//               written, with `special: true`)
//   namespaced  namespacedName, where an unqualified function or constant in a namespace cannot
//               be resolved statically: PHP tries it first and falls back to the global name.
// For those ambiguous names also:
//   declared    whether this file declares the namespaced name: a function declaration, a
//               `const` declaration, or a call of the global `define` (by any name resolving to
//               it, a `use function` alias too) whose name argument is a constant expression
//               (namespace segments case-insensitive, a constant's own name case-sensitive, a
//               leading backslash declaring nothing reachable);
//   builtin     whether the global name is a function or constant of PHP 8.5.11 run as `php -n`
//               (function_exists / defined in a fresh process, which loads no application code).
// Declarations, member names after -> / ?-> / ::, namespace names and names inside `use`
// statements are not resolved and so not recorded there. Each file's `imports` lists instead the
// name every `use` item writes (PHP-Parser's UseItem, of a Use_ or a GroupUse):
//   start, end  the byte range of the item's name (its alias aside)
//   text        the name as written
//   kind        class, function or constant (the item's type, else its statement's)
//   imported    the fully qualified name it imports (a group's prefix joined with the item's
//               name), without a leading backslash.
// Run offline, not in CI, from this directory:
//   /opt/homebrew/Cellar/php/8.5.11/bin/php -n generate_names.php <composer vendor/autoload.php>
// with nikic/php-parser 5.9.0 installed by that Composer project.
//
// With --corpus it records the same per file over whole corpora instead, for
// `benchmark_php_names_corpus` (rerail/native/src/policy_dsl/scope/php_names/tests.rs):
//   /opt/homebrew/Cellar/php/8.5.11/bin/php -n generate_names.php <vendor/autoload.php> \
//       --corpus <output.json> <name>=<path>[,<path>...] ...
// Every `*.php` file under a corpus's paths is read, Blade templates (`*.blade.php`) aside;
// files `php -n -l` rejects are listed in `excluded` (and those PHP-Parser rejects, which should
// be none, in `parserErrors`) and not recorded. Writes {"php", "phpParser", "extensions",
// "corpora": [{"name", "paths", "excluded", "parserErrors", "files": {path: [names]},
// "imports": {path: [use items]}}]}, keyed by absolute path. The T3 input is
//   php -n generate_names.php ROOT/.nera/tools/php/vendor/autoload.php \
//       --corpus ROOT/.nera/t3/names.input.json \
//       laravel=ROOT/.nera/corpora/php/laravel/src,ROOT/.nera/corpora/php/laravel/tests \
//       symfony=ROOT/.nera/corpora/php/symfony/src

declare(strict_types=1);

use PhpParser\ConstExprEvaluator;
use PhpParser\Node;
use PhpParser\Node\Expr;
use PhpParser\Node\Name;
use PhpParser\Node\Name\FullyQualified;
use PhpParser\Node\Stmt;
use PhpParser\NodeTraverser;
use PhpParser\NodeVisitor\NameResolver;
use PhpParser\NodeVisitor\ParentConnectingVisitor;
use PhpParser\NodeVisitorAbstract;
use PhpParser\Parser;
use PhpParser\ParserFactory;
use PhpParser\PhpVersion;

if ($argc !== 2 && ($argc < 5 || $argv[2] !== '--corpus')) {
    fwrite(STDERR, "usage: php -n generate_names.php <vendor/autoload.php> [--corpus <output.json> <name>=<path>[,<path>...] ...]\n");
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

/** A name's key as PHP compares it: namespace segments lowercased; a function's own name too. */
function name_key(string $name, string $kind): string
{
    $name = ltrim($name, '\\');
    $cut = strrpos($name, '\\');
    if ($kind === 'function') {
        return strtolower($name);
    }
    return $cut === false ? $name : strtolower(substr($name, 0, $cut)) . substr($name, $cut);
}

final class Collector extends NodeVisitorAbstract
{
    /** @var list<array{Name, string}> */
    public array $names = [];
    /** @var array<string, true> */
    public array $functions = [];
    /** @var array<string, true> */
    public array $constants = [];
    /** @var list<Expr\FuncCall> */
    public array $defines = [];
    /** @var list<array{Name, string, string}> each use item's name, kind and imported name */
    public array $imports = [];

    public function enterNode(Node $node)
    {
        if ($node instanceof Stmt\Function_) {
            $this->functions[name_key($node->namespacedName->toString(), 'function')] = true;
        } elseif ($node instanceof Stmt\Const_) {
            foreach ($node->consts as $const) {
                $this->constants[name_key($const->namespacedName->toString(), 'constant')] = true;
            }
        } elseif ($node instanceof Expr\FuncCall && $node->name instanceof Name && (
            // NameResolver has already resolved the callee: through a `use function` alias
            // (`def` for `define`), or not at all for an unqualified name that falls back.
            $node->name->getAttribute('resolvedName')?->toLowerString() === 'define'
            || (!$node->name->hasAttribute('resolvedName') && $node->name->toLowerString() === 'define')
        )) {
            $this->defines[] = $node;
        } elseif ($node instanceof Stmt\Use_ || $node instanceof Stmt\GroupUse) {
            foreach ($node->uses as $item) {
                $type = $item->type !== Stmt\Use_::TYPE_UNKNOWN ? $item->type : $node->type;
                $kind = match ($type) {
                    Stmt\Use_::TYPE_FUNCTION => 'function',
                    Stmt\Use_::TYPE_CONSTANT => 'constant',
                    default => 'class',
                };
                $imported = $node instanceof Stmt\GroupUse
                    ? Name::concat($node->prefix, $item->name)->toString()
                    : $item->name->toString();
                $this->imports[] = [$item->name, $kind, $imported];
            }
        }
        if ($node instanceof Name
            && ($node->hasAttribute('resolvedName') || $node->hasAttribute('namespacedName'))) {
            $parent = $node->getAttribute('parent');
            $kind = match (true) {
                $parent instanceof Expr\FuncCall => 'function',
                $parent instanceof Expr\ConstFetch => 'constant',
                default => 'class',
            };
            $this->names[] = [$node, $kind];
        }
        return null;
    }
}

/**
 * One file's names and use items, as the header describes them (`builtin` aside: see
 * `settle_builtins`), and the global names of its ambiguous functions and constants.
 *
 * @return array{list<array<string, mixed>>, list<array<string, mixed>>, array{function: list<string>, constant: list<string>}}
 */
function record_file(Parser $parser, ConstExprEvaluator $evaluator, string $source): array
{
    $statements = $parser->parse($source);
    $collector = new Collector();
    $traverser = new NodeTraverser(
        new ParentConnectingVisitor(),
        new NameResolver(null, ['replaceNodes' => false]),
        $collector,
    );
    $traverser->traverse($statements);

    // Only the global `define` declares: `\define`, or an unqualified `define` that falls back.
    foreach ($collector->defines as $call) {
        $name = $call->name;
        $namespaced = $name->getAttribute('namespacedName');
        $global = $name->getAttribute('resolvedName')?->toLowerString() === 'define'
            || ($namespaced !== null && !isset($collector->functions[name_key($namespaced->toString(), 'function')]));
        // The name argument: the first when it is positional, else the one named constant_name.
        $argument = $call->args[0] ?? null;
        if ($argument instanceof Node\Arg && $argument->name !== null) {
            $argument = null;
            foreach ($call->args as $candidate) {
                if ($candidate instanceof Node\Arg && $candidate->name?->toString() === 'constant_name') {
                    $argument = $candidate;
                }
            }
        }
        if (!$global || !$argument instanceof Node\Arg || $argument->unpack) {
            continue;
        }
        try {
            $value = $evaluator->evaluateDirectly($argument->value);
        } catch (Throwable) {
            continue;
        }
        if (is_string($value) && !str_starts_with($value, '\\')) {
            $collector->constants[name_key($value, 'constant')] = true;
        }
    }

    $names = [];
    $ambiguous = ['function' => [], 'constant' => []];
    foreach ($collector->names as [$name, $kind]) {
        $record = [
            'start' => $name->getStartFilePos(),
            'end' => $name->getEndFilePos() + 1,
            'text' => substr($source, $name->getStartFilePos(), $name->getEndFilePos() + 1 - $name->getStartFilePos()),
            'kind' => $kind,
        ];
        $resolved = $name->getAttribute('resolvedName');
        if ($resolved !== null) {
            $record['resolved'] = $resolved->toString();
            if (!$resolved instanceof FullyQualified) {
                $record['special'] = true;
            }
        } else {
            $namespaced = $name->getAttribute('namespacedName')->toString();
            $record['namespaced'] = $namespaced;
            $record['declared'] = $kind === 'function'
                ? isset($collector->functions[name_key($namespaced, 'function')])
                : isset($collector->constants[name_key($namespaced, 'constant')]);
            $ambiguous[$kind][] = $name->toString();
        }
        $names[] = $record;
    }
    $imports = array_map(fn ($item) => [
        'start' => $item[0]->getStartFilePos(),
        'end' => $item[0]->getEndFilePos() + 1,
        'text' => substr($source, $item[0]->getStartFilePos(), $item[0]->getEndFilePos() + 1 - $item[0]->getStartFilePos()),
        'kind' => $item[1],
        'imported' => $item[2],
    ], $collector->imports);
    return [$names, $imports, $ambiguous];
}

/** Whether each global name is a builtin of `php -n`, asked of a fresh process. */
function builtins(array $functions, array $constants): array
{
    $script = sprintf(
        '<?php echo json_encode(["functions" => array_map("function_exists", %s), "constants" => array_map("defined", %s)]);',
        var_export(array_values($functions), true),
        var_export(array_values($constants), true),
    );
    // The script goes through stdin: a corpus's names outgrow a command line.
    $process = proc_open([PHP_BINARY, '-n'], [['pipe', 'r'], ['pipe', 'w']], $pipes);
    fwrite($pipes[0], $script);
    fclose($pipes[0]);
    $output = stream_get_contents($pipes[1]);
    fclose($pipes[1]);
    proc_close($process);
    $answer = json_decode((string) $output, true, flags: JSON_THROW_ON_ERROR);
    return [
        array_combine(array_values($functions), $answer['functions']) ?: [],
        array_combine(array_values($constants), $answer['constants']) ?: [],
    ];
}

/**
 * Sets `builtin` on every ambiguous name of `$files` (file => names).
 *
 * @param array{function: list<string>, constant: list<string>} $ambiguous
 */
function settle_builtins(array &$files, array $ambiguous): void
{
    [$functions, $constants] = builtins(
        array_unique($ambiguous['function']),
        array_unique($ambiguous['constant']),
    );
    foreach ($files as &$names) {
        foreach ($names as &$record) {
            if (isset($record['namespaced'])) {
                // An ambiguous name is unqualified: its text is the global name.
                $global = $record['text'];
                $record['builtin'] = $record['kind'] === 'function' ? $functions[$global] : $constants[$global];
            }
        }
        unset($record);
    }
    unset($names);
}

/** The files of `$paths`, sorted, that `php -n -l` rejects, linted in parallel. */
function unlintable(array $paths): array
{
    $process = proc_open(
        ['xargs', '-0', '-P', '18', '-n', '1', 'sh', '-c',
            '"$0" -n -l "$1" >/dev/null 2>&1 || printf "%s\0" "$1"', PHP_BINARY],
        [['pipe', 'r'], ['pipe', 'w']],
        $pipes,
    );
    fwrite($pipes[0], implode("\0", $paths) . "\0");
    fclose($pipes[0]);
    $output = stream_get_contents($pipes[1]);
    fclose($pipes[1]);
    proc_close($process);
    $rejected = array_values(array_filter(explode("\0", (string) $output), fn ($path) => $path !== ''));
    sort($rejected, SORT_STRING);
    return $rejected;
}

/** Every `*.php` file under `$root` but Blade templates, sorted. */
function php_files(string $root): array
{
    $paths = [];
    $entries = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($root, FilesystemIterator::SKIP_DOTS));
    foreach ($entries as $entry) {
        $path = $entry->getPathname();
        if ($entry->isFile() && str_ends_with($path, '.php') && !str_ends_with($path, '.blade.php')) {
            $paths[] = $path;
        }
    }
    sort($paths, SORT_STRING);
    return $paths;
}

$parser = (new ParserFactory())->createForVersion(PhpVersion::fromComponents(8, 5));
$evaluator = new ConstExprEvaluator();
$extensions = json_decode(
    (string) shell_exec(escapeshellarg(PHP_BINARY) . ' -n -r ' . escapeshellarg('echo json_encode(get_loaded_extensions());')),
    true,
    flags: JSON_THROW_ON_ERROR,
);
sort($extensions, SORT_STRING);

if ($argc === 2) {
    $files = [];
    $imports = [];
    $ambiguous = ['function' => [], 'constant' => []];
    $fixtures = glob(__DIR__ . '/names/*.php');
    sort($fixtures, SORT_STRING);
    foreach ($fixtures as $path) {
        [$names, $items, $fileAmbiguous] = record_file($parser, $evaluator, file_get_contents($path));
        $files[basename($path)] = $names;
        $imports[basename($path)] = $items;
        $ambiguous = array_merge_recursive($ambiguous, $fileAmbiguous);
    }
    settle_builtins($files, $ambiguous);
    $document = [
        'php' => PHP_VERSION,
        'phpParser' => $parserVersion,
        'extensions' => $extensions,
        'files' => $files,
        'imports' => $imports,
    ];
    file_put_contents(
        __DIR__ . '/names.expected.json',
        json_encode($document, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR) . "\n",
    );
    exit(0);
}

ini_set('memory_limit', '-1');
$corpora = [];
foreach (array_slice($argv, 4) as $argument) {
    [$name, $list] = explode('=', $argument, 2);
    $roots = array_map(fn ($path) => realpath($path) ?: throw new RuntimeException("no $path"), explode(',', $list));
    $paths = array_merge(...array_map('php_files', $roots));
    $excluded = unlintable($paths);
    $skip = array_flip($excluded);
    $files = [];
    $imports = [];
    $parserErrors = [];
    $ambiguous = ['function' => [], 'constant' => []];
    foreach ($paths as $path) {
        if (isset($skip[$path])) {
            continue;
        }
        try {
            [$names, $items, $fileAmbiguous] = record_file($parser, $evaluator, file_get_contents($path));
        } catch (PhpParser\Error $error) {
            $parserErrors[] = ['path' => $path, 'message' => $error->getMessage()];
            continue;
        }
        $files[$path] = $names;
        $imports[$path] = $items;
        $ambiguous['function'] = array_merge($ambiguous['function'], $fileAmbiguous['function']);
        $ambiguous['constant'] = array_merge($ambiguous['constant'], $fileAmbiguous['constant']);
    }
    settle_builtins($files, $ambiguous);
    fwrite(STDERR, sprintf("%s: %d files, %d excluded, %d parser errors\n", $name, count($files), count($excluded), count($parserErrors)));
    $corpora[] = [
        'name' => $name,
        'paths' => $roots,
        'excluded' => $excluded,
        'parserErrors' => $parserErrors,
        'files' => $files,
        'imports' => $imports,
    ];
}
file_put_contents(
    $argv[3],
    json_encode([
        'php' => PHP_VERSION,
        'phpParser' => $parserVersion,
        'extensions' => $extensions,
        'corpora' => $corpora,
    ], JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR) . "\n",
);
