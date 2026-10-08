<?php
// The phpDoc oracles' recorder, shared by `generate_phpdoc.php` and the corpus samples under
// `literal/{laravel,symfony}/`: how phpstan/phpdoc-parser 2.3.5, as PHPStan 2.2.16
// bundles it, parses a doc comment, the way PHPStan reads one (`PhpDocStringResolver`: the
// comment's whole text through `Lexer::tokenize` and `PhpDocParser::parse`).
//
// A record is [text, tree], tree a node [kind, start, end, props, children]:
// - kind: the node's class without its namespace and `Node` suffix, in snake case
//   (`ParamTagValueNode` -> `param_tag_value`, `DoctrineArray` -> `doctrine_array`);
// - start, end: byte offsets into the text of the first token of the node's `startIndex` and the
//   end of the token of its `endIndex` (the end is before the start for a node of no token);
// - props: its scalar properties by name (strings, booleans, integers, lists of strings), null
//   ones left out; an invalid node's exception is its message (`message`);
// - children: [property, node] for each node-valued property in declaration order, a list of
//   nodes giving one entry per element.
// Where the parser throws (a `LogicException`), the tree is [class, message] instead.

use PHPStan\PhpDocParser\Ast\Node;
use PHPStan\PhpDocParser\Lexer\Lexer;
use PHPStan\PhpDocParser\Parser\ConstExprParser;
use PHPStan\PhpDocParser\Parser\ParserException;
use PHPStan\PhpDocParser\Parser\PhpDocParser;
use PHPStan\PhpDocParser\Parser\TokenIterator;
use PHPStan\PhpDocParser\Parser\TypeParser;
use PHPStan\PhpDocParser\ParserConfig;

const PHPDOC_PARSER_VERSION = '2.3.5';

/** Loads PHPStan's phar and checks the versions the oracle pins; returns PHPStan's version line. */
function phpdoc_require(string $phar): string
{
    if (PHP_VERSION !== '8.5.11') {
        fwrite(STDERR, "PHP 8.5.11 required, found " . PHP_VERSION . "\n");
        exit(1);
    }
    require_once "phar://$phar/vendor/autoload.php";
    $version = trim((string) shell_exec(escapeshellarg(PHP_BINARY) . ' ' . escapeshellarg($phar) . ' --version'));
    if (!str_contains($version, '2.2.16')) {
        fwrite(STDERR, "PHPStan 2.2.16 required, found $version\n");
        exit(1);
    }
    $installed = require "phar://$phar/vendor/composer/installed.php";
    $parser = $installed['versions']['phpstan/phpdoc-parser']['pretty_version'] ?? '';
    if ($parser !== PHPDOC_PARSER_VERSION) {
        fwrite(STDERR, "phpstan/phpdoc-parser " . PHPDOC_PARSER_VERSION . " required, found $parser\n");
        exit(1);
    }
    return $version;
}

function phpdoc_kind(object $node): string
{
    $class = (new ReflectionClass($node))->getShortName();
    if (str_ends_with($class, 'Node')) {
        $class = substr($class, 0, -4);
    }
    return strtolower((string) preg_replace('/(?<=[a-z0-9])(?=[A-Z])/', '_', $class));
}

/** @param list<int> $offsets byte offset of each token, and the text's length last */
function phpdoc_dump(Node $node, array $offsets): array
{
    $start = $node->getAttribute('startIndex');
    $end = $node->getAttribute('endIndex');
    $props = [];
    $children = [];
    foreach ((new ReflectionObject($node))->getProperties() as $property) {
        $name = $property->getName();
        if ($property->isStatic() || $name === 'attributes') {
            continue;
        }
        $value = $property->getValue($node);
        if ($name === 'exceptionArgs') {
            $props['message'] = (new ParserException(...$value))->getMessage();
        } elseif ($value instanceof Node) {
            $children[] = [$name, phpdoc_dump($value, $offsets)];
        } elseif (is_array($value)) {
            foreach ($value as $item) {
                if ($item instanceof Node) {
                    $children[] = [$name, phpdoc_dump($item, $offsets)];
                }
            }
            if ($value !== [] && !($value[array_key_first($value)] instanceof Node)) {
                $props[$name] = array_values($value);
            }
        } elseif ($value !== null) {
            $props[$name] = $value;
        }
    }
    ksort($props);
    return [
        phpdoc_kind($node),
        $start === null ? null : $offsets[$start],
        $end === null ? null : $offsets[$end + 1],
        (object) $props,
        $children,
    ];
}

/** [text, tree] for a doc comment. */
function phpdoc_record(string $text): array
{
    static $lexer, $parser;
    if ($parser === null) {
        $config = new ParserConfig(['lines' => true, 'indexes' => true, 'comments' => true]);
        $lexer = new Lexer($config);
        $constExprParser = new ConstExprParser($config);
        $parser = new PhpDocParser($config, new TypeParser($config, $constExprParser), $constExprParser);
    }
    $tokens = $lexer->tokenize($text);
    $offsets = [0];
    foreach ($tokens as $token) {
        $offsets[] = $offsets[count($offsets) - 1] + strlen($token[0]);
    }
    try {
        $tree = phpdoc_dump($parser->parse(new TokenIterator($tokens)), $offsets);
    } catch (Throwable $error) {
        $tree = [(new ReflectionClass($error))->getShortName(), $error->getMessage()];
    }
    return [$text, $tree];
}

/** Whether PHP lexes `$text` as one doc comment (`T_DOC_COMMENT`). */
function phpdoc_is_doc_comment(string $text): bool
{
    $tokens = token_get_all("<?php " . $text);
    return count($tokens) === 2 && is_array($tokens[1]) && $tokens[1][0] === T_DOC_COMMENT && $tokens[1][1] === $text;
}

/** The doc comments of every PHP file under `$root` (Blade templates excluded), in path order. */
function phpdoc_corpus(string $root): array
{
    $files = [];
    $iterator = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($root, FilesystemIterator::SKIP_DOTS));
    foreach ($iterator as $file) {
        $path = $file->getPathname();
        if (str_ends_with($path, '.php') && !str_ends_with($path, '.blade.php')) {
            $files[] = $path;
        }
    }
    sort($files);
    $docs = [];
    foreach ($files as $path) {
        foreach (@token_get_all((string) file_get_contents($path)) as $token) {
            if (is_array($token) && $token[0] === T_DOC_COMMENT && mb_check_encoding($token[1], 'UTF-8')) {
                $docs[] = $token[1];
            }
        }
    }
    return $docs;
}

/** Writes records as JSON, one record per line. */
function phpdoc_write(string $path, array $header, array $records): void
{
    $flags = JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE | JSON_THROW_ON_ERROR;
    $lines = [];
    foreach ($records as $record) {
        $lines[] = json_encode($record, $flags);
    }
    $header['cases'] = '@CASES@';
    $out = str_replace('"@CASES@"', "[\n" . implode(",\n", $lines) . "\n]", json_encode($header, $flags));
    file_put_contents($path, $out . "\n");
}

/**
 * Writes a corpus sample oracle: the first `$count` distinct doc comments of the repository at
 * `$root` in SHA-1 order (a deterministic sample independent of file order), recorded.
 */
function phpdoc_sample(string $phar, string $root, string $repository, string $out, int $count): void
{
    $version = phpdoc_require($phar);
    $commit = trim((string) shell_exec('git -C ' . escapeshellarg($root) . ' rev-parse HEAD'));
    $docs = array_keys(array_flip(phpdoc_corpus($root)));
    usort($docs, fn(string $a, string $b): int => strcmp(sha1($a), sha1($b)));
    $records = array_map('phpdoc_record', array_slice($docs, 0, $count));
    $header = [
        'php' => PHP_VERSION,
        'phpstan' => $version,
        'phpdocParser' => PHPDOC_PARSER_VERSION,
        'repository' => $repository,
        'commit' => $commit,
        'docComments' => count($docs),
    ];
    phpdoc_write($out, $header, $records);
    fwrite(STDERR, count($records) . " of " . count($docs) . " doc comments\n");
}
