<?php
// Records every constant expression in PHP corpora, with what PHP 8.5.11 evaluates it to, for
// `benchmark_php_values_corpus` (rerail/native/src/policy_dsl/value/php/tests.rs). Offline, not in
// CI:
//   /opt/homebrew/Cellar/php/8.5.11/bin/php generate_corpus.php \
//       <composer autoload.php with nikic/php-parser 5.9.0> <output.json> \
//       <name>=<checkout>:<path>[,<path>...] ...
// e.g. `laravel=.nera/corpora/php/laravel:src,tests symfony=.nera/corpora/php/symfony:src`.
//
// nikic/php-parser 5.9.0 finds the initializers: a namespace-level `const X = ...`, a class,
// interface, trait or enum `const`, and an enum case's value. Files PHP cannot parse (`php -l`) and
// Blade templates are excluded, as tape parity excludes them. An initializer is evaluable in
// isolation when it is built from literals alone (`true`, `false`, `null` the only names): PHP
// evaluates it as `generate_php.php` does, `const X = <src>;` in a fresh `php -n` process
// (`evaluate.php`), and `const` records the tagged value, or the error PHP raises. Any other
// initializer reads its file's context (a named or class constant, a magic constant, `new`, a
// closure): `unevaluable` lists why. For such an array literal, PHP evaluates it with each
// context-reading element value replaced by `null`: where every key and spread is evaluable,
// `length` is that array's element count, which is the literal's (it does not depend on the values
// replaced); otherwise, with each such key replaced by `0` and spread by `[]`, `nonempty` is whether
// that array has an element, which implies the literal has one. Writes {"phpParser", "php",
// "intSize", "corpora": [{"name", "root", "paths", "excluded", "records": {file: [{"at" (byte
// offset of the constant's name), "line", "kind" (const | classConst | enumCase), "src", "const" +
// "warnings" | "unevaluable" + "length" | "nonempty"?}]}}]}.

use PhpParser\Node;
use PhpParser\Node\Expr;
use PhpParser\Node\Scalar;
use PhpParser\NodeFinder;
use PhpParser\ParserFactory;

require __DIR__ . '/evaluate.php';

/** Runs every command, `$width` at a time: each one's [stdout, stderr, exit code], by key. */
function pool(array $commands, int $width): array {
    $results = [];
    $running = [];
    $queue = $commands;
    while ($queue || $running) {
        while ($queue && count($running) < $width) {
            $key = array_key_first($queue);
            $command = $queue[$key];
            unset($queue[$key]);
            $process = proc_open($command, [0 => ['file', '/dev/null', 'r'], 1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes);
            stream_set_blocking($pipes[1], false);
            stream_set_blocking($pipes[2], false);
            $running[$key] = [$process, $pipes, '', ''];
        }
        $read = [];
        foreach ($running as $job) {
            if (!feof($job[1][1])) {
                $read[] = $job[1][1];
            }
            if (!feof($job[1][2])) {
                $read[] = $job[1][2];
            }
        }
        if ($read) {
            $write = $except = null;
            stream_select($read, $write, $except, 1);
        }
        foreach ($running as $key => &$job) {
            $job[2] .= stream_get_contents($job[1][1]);
            $job[3] .= stream_get_contents($job[1][2]);
            if (feof($job[1][1]) && feof($job[1][2])) {
                fclose($job[1][1]);
                fclose($job[1][2]);
                $results[$key] = [$job[2], $job[3], proc_close($job[0])];
                unset($running[$key]);
            }
        }
        unset($job);
    }
    return $results;
}

function php_files(string $root): array {
    $files = [];
    $entries = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($root, FilesystemIterator::SKIP_DOTS));
    foreach ($entries as $entry) {
        $path = $entry->getPathname();
        if (str_ends_with($path, '.php') && !str_ends_with($path, '.blade.php')) {
            $files[] = $path;
        }
    }
    sort($files, SORT_STRING);
    return $files;
}

/** Why `$expr` reads its file's context: nothing where it is built from literals alone. */
function blockers(Expr $expr): array {
    $reasons = [];
    foreach ((new NodeFinder())->find($expr, fn (Node $node) => $node instanceof Expr || $node instanceof Scalar) as $node) {
        $reason = match (true) {
            $node instanceof Scalar\MagicConst => 'magic constant',
            $node instanceof Scalar\InterpolatedString => 'interpolation',
            $node instanceof Scalar => null,
            $node instanceof Expr\ConstFetch => in_array($node->name->toLowerString(), ['true', 'false', 'null'], true)
                ? null : 'named constant',
            $node instanceof Expr\ClassConstFetch => 'class constant',
            $node instanceof Expr\Array_, $node instanceof Expr\BinaryOp, $node instanceof Expr\UnaryMinus,
            $node instanceof Expr\UnaryPlus, $node instanceof Expr\BitwiseNot, $node instanceof Expr\BooleanNot,
            $node instanceof Expr\Ternary, $node instanceof Expr\ArrayDimFetch, $node instanceof Expr\Cast => null,
            default => lcfirst(rtrim((new ReflectionClass($node))->getShortName(), '_')),
        };
        if ($reason !== null) {
            $reasons[$reason] = true;
        }
    }
    return array_keys($reasons);
}

/** An array literal's source with each element value that reads context replaced by `null`, each
 *  such key by `0` and each such spread by `[]`, and whether it is exact: whether no key or spread
 *  was replaced, so that its element count is the literal's. Otherwise its count is non-zero only
 *  where the literal's is (an element it keeps is one of the literal's, or a duplicate key's). */
function shape(Expr $expr, string $code): ?array {
    if (!$expr instanceof Expr\Array_) {
        return null;
    }
    $base = $expr->getStartFilePos();
    $src = substr($code, $base, $expr->getEndFilePos() - $base + 1);
    $replaced = [];
    $exact = true;
    foreach ($expr->items as $item) {
        if ($item->key !== null && blockers($item->key)) {
            $exact = false;
            $replaced[] = [$item->key->getStartFilePos() - $base, $item->key->getEndFilePos() - $base + 1, '0'];
        }
        if (blockers($item->value)) {
            $exact = $exact && !$item->unpack;
            $replaced[] = [$item->value->getStartFilePos() - $base, $item->value->getEndFilePos() - $base + 1, $item->unpack ? '[]' : 'null'];
        }
    }
    foreach (array_reverse($replaced) as [$start, $end, $by]) {
        $src = substr($src, 0, $start) . $by . substr($src, $end);
    }
    return [$src, $exact];
}

[$autoload, $output] = [$argv[1], $argv[2]];
require $autoload;
$parser = (new ParserFactory())->createForHostVersion();
$width = (int) (shell_exec('sysctl -n hw.ncpu') ?: 8);
$corpora = [];
$evaluate = [];
foreach (array_slice($argv, 3) as $argument) {
    [$name, $spec] = explode('=', $argument, 2);
    [$checkout, $paths] = explode(':', $spec, 2);
    $root = realpath($checkout);
    $roots = array_map(fn ($path) => realpath("$root/$path"), explode(',', $paths));
    $files = array_merge(...array_map('php_files', $roots));
    $lint = pool(array_map(fn ($file) => ['php', '-l', $file], $files), $width);
    $excluded = [];
    $records = [];
    foreach ($files as $index => $file) {
        if ($lint[$index][2] !== 0) {
            $excluded[] = $file;
            continue;
        }
        $code = file_get_contents($file);
        $ast = $parser->parse($code);
        $found = [];
        foreach ((new NodeFinder())->find($ast, fn (Node $node) => $node instanceof Node\Stmt\Const_
            || $node instanceof Node\Stmt\ClassConst || ($node instanceof Node\Stmt\EnumCase && $node->expr !== null)) as $statement) {
            $kind = match (true) {
                $statement instanceof Node\Stmt\Const_ => 'const',
                $statement instanceof Node\Stmt\ClassConst => 'classConst',
                default => 'enumCase',
            };
            $elements = $statement instanceof Node\Stmt\EnumCase ? [[$statement->name, $statement->expr]]
                : array_map(fn ($const) => [$const->name, $const->value], $statement->consts);
            foreach ($elements as [$constant, $expr]) {
                $start = $expr->getStartFilePos();
                $record = [
                    'at' => $constant->getStartFilePos(),
                    'line' => $constant->getStartLine(),
                    'kind' => $kind,
                    'src' => substr($code, $start, $expr->getEndFilePos() - $start + 1),
                ];
                $reasons = blockers($expr);
                if ($reasons) {
                    $record['unevaluable'] = $reasons;
                    $shape = shape($expr, $code);
                    if ($shape !== null) {
                        $record['shape'] = $shape;
                        $evaluate[$shape[0]] = true;
                    }
                } else {
                    $evaluate[$record['src']] = true;
                }
                $found[] = $record;
            }
        }
        usort($found, fn ($a, $b) => $a['at'] <=> $b['at']);
        if ($found) {
            $records[$file] = $found;
        }
    }
    $corpora[] = ['name' => $name, 'root' => $root, 'paths' => $roots, 'excluded' => $excluded, 'records' => $records];
}
$sources = array_keys($evaluate);
$runs = pool(array_map(fn ($src) => command(constant_code((string) $src)), $sources), $width);
$values = [];
foreach ($sources as $index => $src) {
    [$result, $diagnostics] = outcome($runs[$index][0], $runs[$index][1]);
    $values[$src] = [$result ?? error_of($diagnostics), warnings_of($diagnostics)];
}
foreach ($corpora as &$corpus) {
    foreach ($corpus['records'] as &$found) {
        foreach ($found as &$record) {
            if (isset($record['shape'])) {
                [$src, $exact] = $record['shape'];
                [$value] = $values[$src];
                if ($value[0] === 'array') {
                    $record[$exact ? 'length' : 'nonempty'] = $exact ? count($value[1]) : $value[1] !== [];
                }
                unset($record['shape']);
            } elseif (!isset($record['unevaluable'])) {
                [$record['const'], $record['warnings']] = $values[$record['src']];
            }
        }
    }
}
unset($corpus, $found, $record);
file_put_contents($output, json_encode([
    'phpParser' => Composer\InstalledVersions::getPrettyVersion('nikic/php-parser'),
    'php' => PHP_VERSION,
    'intSize' => PHP_INT_SIZE,
    'corpora' => $corpora,
], JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE | JSON_THROW_ON_ERROR) . "\n");
