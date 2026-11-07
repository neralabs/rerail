<?php
// How the port readiness gates read PHP, run on the pinned interpreter of conformance/recorders/port-readiness/runtimes/php
// with that runtime's vendor/ (nikic/php-parser), whose directory is the first argument.
//
// One JSON request on stdin, one JSON response on stdout:
// - {"op": "facts"}: PHP's keywords as the interpreter's tokenizer reads them, and PHP-Parser's version.
// - {"op": "words", "texts": [...]}: per text, every word of every token but numbers, as the TypeScript adapters scan.
// - {"op": "literals", "files": [...]}: per file, the names its code checks (identifier-shaped and segmented string
//   literals, quoted words in strings, words of the regexes it passes to preg_*), those regexes, its `use` imports and
//   the classes it declares with what they extend.
// - {"op": "verdicts", "files": [{"path", "source"}], "ini": [...]}: per file, `php -l` on this interpreter with the
//   gates' php.ini settings (parse errors are syntax, compile errors grammar) and PHP-Parser's errors (syntax only).

declare(strict_types=1);

require $argv[1] . '/vendor/autoload.php';

use PhpParser\Node;
use PhpParser\NodeFinder;
use PhpParser\ParserFactory;

const WORD = '/[A-Za-z_][A-Za-z0-9_]*/';
const IDENTIFIER = '/^[A-Za-z_][A-Za-z0-9_]*$/';
// A qualified name, a static member or a path that upstream matches by prefix or suffix: `Foo\Bar`, `Foo::bar`, `/var/tmp`, `.run`.
const SEGMENTED = '/^(?:\\\\|[.\/])?[A-Za-z_][A-Za-z0-9_]*(?:(?:\\\\|::|[.\/])[A-Za-z_][A-Za-z0-9_]*)*$/';
const QUOTED = '/["\'`]([A-Za-z_][A-Za-z0-9_]*)["\'`]/';
const NUMBER_TOKENS = [T_LNUMBER, T_DNUMBER];

/**
 * PHP's reserved words that the tokenizer reads as plain names: the PHP manual's "other reserved words", the relative
 * class names, and `this`, the pseudo-variable the compiler reserves (`$this` cannot be assigned, unset or declared as
 * a parameter: "Cannot re-assign $this").
 */
const RESERVED = ['bool', 'false', 'float', 'int', 'iterable', 'mixed', 'never', 'null', 'numeric', 'object', 'resource', 'string', 'true', 'void', 'enum', 'self', 'parent', 'this'];
/** Keywords whose token name is not their spelling: the word operators and the magic constants. */
const SPELLED = ['and', 'or', 'xor', '__halt_compiler', '__class__', '__dir__', '__file__', '__function__', '__line__', '__method__', '__namespace__', '__property__', '__trait__'];

function keywords(): array
{
    $candidates = [...RESERVED, ...SPELLED];
    foreach (array_keys(get_defined_constants(true)['tokenizer']) as $name) {
        if (str_starts_with($name, 'T_')) $candidates[] = strtolower(substr($name, 2));
    }
    $found = [];
    foreach (array_unique($candidates) as $word) {
        if (!preg_match(IDENTIFIER, $word)) continue;
        $tokens = token_get_all("<?php {$word} ");
        // A keyword is a word the tokenizer reads as a token of its own rather than as a name.
        $keyword = is_array($tokens[1] ?? null) && $tokens[1][1] === $word && $tokens[1][0] !== T_STRING;
        if ($keyword || in_array($word, RESERVED, true)) {
            // PHP reads keywords in any case: list the two spellings code uses.
            $found[$word] = true;
            $found[strtoupper($word)] = true;
        }
    }
    $words = array_keys($found);
    sort($words, SORT_STRING);
    return $words;
}

function words(string $text): array
{
    $found = [];
    foreach (token_get_all($text) as $token) {
        if (!is_array($token)) continue;
        if (in_array($token[0], NUMBER_TOKENS, true)) continue;
        preg_match_all(WORD, $token[1], $matches);
        foreach ($matches[0] as $word) $found[] = $word;
    }
    return $found;
}

/** The fully qualified name of a resolved name node, without its leading `\`. */
function qualified(Node\Name $name): string
{
    $resolved = $name->getAttribute('resolvedName');
    return ltrim(($resolved instanceof Node\Name ? $resolved : $name)->toString(), '\\');
}

function literals(string $path): array
{
    $parser = (new ParserFactory())->createForHostVersion();
    $statements = $parser->parse(file_get_contents($path)) ?? [];
    $traverser = new PhpParser\NodeTraverser(new PhpParser\NodeVisitor\NameResolver(null, ['replaceNodes' => false]));
    $statements = $traverser->traverse($statements);
    $finder = new NodeFinder();
    $names = [];
    $regexes = [];
    $imports = [];
    $classes = [];
    foreach ($finder->findInstanceOf($statements, Node\Scalar\String_::class) as $string) {
        $text = $string->value;
        if (preg_match(IDENTIFIER, $text)) {
            $names[$text] = true;
        } elseif (preg_match(SEGMENTED, $text)) {
            foreach (preg_split('/\\\\|::|[.\/]/', $text, -1, PREG_SPLIT_NO_EMPTY) as $segment) $names[$segment] = true;
        }
        preg_match_all(QUOTED, $text, $quoted);
        foreach ($quoted[1] as $word) $names[$word] = true;
    }
    foreach ($finder->findInstanceOf($statements, Node\Expr\FuncCall::class) as $call) {
        if (!$call->name instanceof Node\Name || !str_starts_with(strtolower($call->name->getLast()), 'preg_')) continue;
        $first = $call->args[0] ?? null;
        if ($first instanceof Node\Arg && $first->value instanceof Node\Scalar\String_) $regexes[] = $first->value->value;
    }
    foreach ($finder->findInstanceOf($statements, Node\Stmt\Use_::class) as $use) {
        foreach ($use->uses as $item) $imports[ltrim($item->name->toString(), '\\')] = true;
    }
    foreach ($finder->findInstanceOf($statements, Node\Stmt\GroupUse::class) as $use) {
        foreach ($use->uses as $item) $imports[ltrim($use->prefix->toString() . '\\' . $item->name->toString(), '\\')] = true;
    }
    foreach ($finder->findInstanceOf($statements, Node\Stmt\Class_::class) as $class) {
        if ($class->namespacedName === null) continue;
        $classes[] = ['name' => $class->namespacedName->toString(), 'extends' => $class->extends === null ? null : qualified($class->extends)];
    }
    foreach ($regexes as $pattern) {
        // A pattern's words, less its escapes (`\d`, `\s`) and its delimiter and modifiers.
        preg_match_all(WORD, preg_replace('/\\\\[A-Za-z]/', ' ', $pattern), $matches);
        foreach ($matches[0] as $word) $names[$word] = true;
    }
    $names = array_keys($names);
    sort($names, SORT_STRING);
    $imports = array_keys($imports);
    sort($imports, SORT_STRING);
    return ['names' => $names, 'regexes' => $regexes, 'imports' => $imports, 'classes' => $classes];
}

/** `php -l` on this interpreter: `Parse error` lines are syntax errors, any other error is a compile (grammar) error. */
function lint_verdict(string $source, array $ini, string $directory): array
{
    $file = tempnam($directory, 'lint');
    file_put_contents($file, $source);
    try {
        $process = proc_open([PHP_BINARY, ...$ini, '-d', 'display_errors=stdout', '-d', 'log_errors=0', '-d', 'html_errors=0', '-l', $file], [1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes);
        // The temporary file's name is no part of a verdict: the recording must read the same on every machine.
        $output = str_replace([realpath($file), $file], '<file>', stream_get_contents($pipes[1]) . stream_get_contents($pipes[2]));
        fclose($pipes[1]);
        fclose($pipes[2]);
        $status = proc_close($process);
    } finally {
        unlink($file);
    }
    $syntax = [];
    $grammar = [];
    foreach (preg_split('/\R/', $output) as $line) {
        if (!preg_match('/^(?:PHP )?(Parse error|Fatal error|Warning|Deprecated|Notice|Compile Error|Compile Warning):\s+(.*?) in \S+ on line (\d+)$/', trim($line), $match)) continue;
        if ($match[1] === 'Deprecated' || $match[1] === 'Notice' || $match[1] === 'Warning' || $match[1] === 'Compile Warning') continue;
        $message = "{$match[1]}: {$match[2]} (line {$match[3]})";
        if ($match[1] === 'Parse error') $syntax[] = $message;
        else $grammar[] = $message;
    }
    if ($status !== 0 && $syntax === [] && $grammar === []) $grammar[] = 'php -l failed: ' . trim(str_replace($file, '<file>', $output));
    return ['syntax' => $syntax, 'grammar' => $grammar];
}

/** PHP-Parser for the running PHP version: whatever it reports while building a tree is a syntax error. */
function php_parser_verdict(string $source): array
{
    $parser = (new ParserFactory())->createForHostVersion();
    try {
        $parser->parse($source);
        return ['syntax' => [], 'grammar' => null];
    } catch (PhpParser\Error $error) {
        return ['syntax' => [$error->getMessage()], 'grammar' => null];
    }
}

function parser_version(): string
{
    return ltrim(Composer\InstalledVersions::getPrettyVersion('nikic/php-parser') ?? 'unknown', 'v');
}

$request = json_decode(stream_get_contents(STDIN), true, 512, JSON_THROW_ON_ERROR);
switch ($request['op']) {
    case 'facts':
        $response = ['keywords' => keywords(), 'parser' => parser_version()];
        break;
    case 'words':
        $response = array_map('words', $request['texts']);
        break;
    case 'literals':
        $response = array_map('literals', $request['files']);
        break;
    case 'verdicts':
        $directory = sys_get_temp_dir();
        $parser = 'nikic/php-parser ' . parser_version();
        $response = array_map(static fn (array $file): array => [
            ['parser' => 'php -l ' . PHP_VERSION, ...lint_verdict($file['source'], $request['ini'], $directory)],
            ['parser' => $parser, ...php_parser_verdict($file['source'])],
        ], $request['files']);
        break;
    default:
        fwrite(STDERR, "unknown op {$request['op']}\n");
        exit(1);
}
echo json_encode($response, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_INVALID_UTF8_SUBSTITUTE), "\n";
