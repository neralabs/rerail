<?php
// Writes what Forte 1.1.0 parses templates to (`forte_tree.php` documents a case), for the Rust
// port's tests (`native/src/code_rule_dsl/blade/forte/tests.rs`):
//
// - suite.expected.json: every template Forte's own test suite lexes, with the registry it lexed
//   it with (`record_suite.php` records them; with no `--suite-inputs` the templates are read
//   back from the existing file). A case holds its tree's `digest` (`forte_digest`); a template
//   the suite lexed prefix by prefix (`prefixes`) holds `digests`, one per length
//   `forte_prefix_positions` picks. Every input is readable; every expected output is a digest
//   this script reproduces from it.
// - fixtures.expected.json: the Blade fixtures (`../fixtures`) and the tree-sitter-blade
//   grammar-gap repros (`../grammar-gaps/repros`), each with the "default" and "accept-all"
//   registries, every tree spelled out; and `default_registry`, the state of
//   `Directives::withDefaults()` (`forte_registry_state`).
// - corpus/<name>.expected.json: samples of the four pinned corpora (the first 25 templates of
//   at most 6 KiB in SHA-1 order of their path), both registries, trees spelled out.
//
// With `--corpus OUT`, it writes OUT instead: every template of the corpora (selected as the
// Rust benchmarks select them: `*.blade.php`, non-empty, at most 4,000,000 bytes, UTF-8), both
// registries, trees spelled out, the input of the ignored benchmark `benchmark_forte_corpus`.
// With `--full OUT`, it writes the suite's cases with every tree spelled out (prefixes too), for
// reading a difference the digests only report.
//
// Every file holds one case per line. Run offline, not in CI, from this directory:
//   php generate.php <blade-parsers project> <checkout> [--suite-inputs suite.inputs.json]
//       [--corpus OUT | --full OUT]
// <blade-parsers project> is a composer project requiring fortephp/forte:1.1.0 (laravel/framework
// v13.34.0 resolved); <checkout> holds the corpora under .rerail/corpora/blade/{bookstack,
// filament,bagisto} and .rerail/corpora/php/laravel. PHP 8.5.11.
declare(strict_types=1);

ini_set('memory_limit', '8G');
require __DIR__ . '/forte_tree.php';

$args = array_slice($argv, 1);
$option = function (string $name) use (&$args): ?string {
    $at = array_search($name, $args, true);
    if ($at === false) {
        return null;
    }
    $value = $args[$at + 1] ?? null;
    array_splice($args, $at, 2);

    return $value;
};
$suiteInputs = $option('--suite-inputs');
$corpusOut = $option('--corpus');
$fullOut = $option('--full');
[$project, $checkout] = $args + [null, null];
if ($project === null || $checkout === null) {
    fwrite(STDERR, "usage: php generate.php <blade-parsers project> <checkout> [--suite-inputs F] [--corpus OUT | --full OUT]\n");
    exit(1);
}
forte_require($project);

const CORPORA = [
    'bookstack' => ['blade/bookstack', 'https://github.com/BookStackApp/BookStack', 'f585a7d9100a99cba8479c1ef5fd56e05562daea'],
    'filament' => ['blade/filament', 'https://github.com/filamentphp/filament', 'ca4f55bb5e05df56ba1609595c082d44c2511620'],
    'bagisto' => ['blade/bagisto', 'https://github.com/bagisto/bagisto', '635cab68b3229896f854b1b3b3e2dbffcfb6aba8'],
    'laravel' => ['php/laravel', 'https://github.com/laravel/framework', 'e5f3d6265f4b9d3d9efc466de83f79c54e14dd5d'],
];

function header_fields(): array
{
    return [
        'tool' => 'fortephp/forte',
        'version' => FORTE_VERSION,
        'commit' => FORTE_COMMIT,
        'laravel' => \Composer\InstalledVersions::getPrettyVersion('laravel/framework'),
        'php' => PHP_VERSION,
    ];
}

/** Writes `value` as JSON to `path`, one case of `value['cases']` per line. */
function write_json(string $path, array $value): void
{
    @mkdir(dirname($path), 0777, true);
    $cases = $value['cases'] ?? [];
    unset($value['cases']);
    $head = forte_json($value);
    $lines = array_map(fn ($case) => forte_json($case), $cases);
    $json = substr($head, 0, -1) . ($head === '{}' ? '' : ',') . "\"cases\":[\n" . implode(",\n", $lines) . "\n]}\n";
    file_put_contents($path, $json);
    fwrite(STDERR, sprintf("%s: %d cases, %.1f MB\n", $path, count($cases), filesize($path) / 1e6));
}

function read_json(string $path): array
{
    return json_decode((string) file_get_contents($path), true, flags: JSON_THROW_ON_ERROR);
}

/** Templates of a corpus as the benchmarks select them, by path relative to its root. */
function corpus_files(string $root): array
{
    $files = [];
    $iterator = new RecursiveIteratorIterator(new RecursiveCallbackFilterIterator(
        new RecursiveDirectoryIterator($root, FilesystemIterator::SKIP_DOTS),
        fn ($file) => $file->getFilename() !== '.git'
    ));
    foreach ($iterator as $file) {
        if (! str_ends_with($file->getFilename(), '.blade.php')) {
            continue;
        }
        $bytes = (string) file_get_contents($file->getPathname());
        if ($bytes === '' || strlen($bytes) > 4_000_000 || ! mb_check_encoding($bytes, 'UTF-8')) {
            continue;
        }
        $files[substr($file->getPathname(), strlen($root) + 1)] = $bytes;
    }
    ksort($files, SORT_STRING);

    return $files;
}

/** A case per registry for a template, every tree spelled out. */
function both_registries(string $source, array $fields): array
{
    $cases = [];
    foreach (['default', 'accept-all'] as $registry) {
        $cases[] = $fields + forte_source_json($source) + [
            'registry' => $registry,
            'tree' => forte_tree($source, forte_registry($registry, [])),
        ];
    }

    return $cases;
}

$corpusRoot = fn (string $name) => $checkout . '/.rerail/corpora/' . CORPORA[$name][0];
$corpusHeader = fn (string $name) => ['name' => $name, 'repository' => CORPORA[$name][1], 'commit' => CORPORA[$name][2]];

if ($corpusOut !== null) {
    $cases = [];
    $corpora = [];
    foreach (array_keys(CORPORA) as $name) {
        $corpora[] = $corpusHeader($name);
        foreach (corpus_files($corpusRoot($name)) as $path => $bytes) {
            array_push($cases, ...both_registries($bytes, ['corpus' => $name, 'path' => $path]));
        }
    }
    write_json($corpusOut, header_fields() + ['corpora' => $corpora, 'cases' => $cases]);
    exit(0);
}

// The suite.
$suite = $suiteInputs !== null
    ? read_json($suiteInputs)
    : read_json(__DIR__ . '/suite.expected.json');
$registries = $suite['registries'];
$cases = [];
foreach ($suite['cases'] as $input) {
    $source = forte_source_of($input);
    $directives = fn () => forte_registry($input['registry'], $registries);
    $case = forte_source_json($source) + ['registry' => $input['registry']];
    if ($input['prefixes'] ?? false) {
        $case['prefixes'] = true;
    }
    $case['tests'] = $input['tests'];
    if ($input['prefixes'] ?? false) {
        $trees = [];
        foreach (forte_prefix_positions(strlen($source)) as $length) {
            $tree = forte_tree(substr($source, 0, $length), $directives());
            $trees[] = $fullOut !== null ? $tree : forte_digest($tree);
        }
        $case[$fullOut !== null ? 'trees' : 'digests'] = $trees;
    } else {
        $tree = forte_tree($source, $directives());
        $case[$fullOut !== null ? 'tree' : 'digest'] = $fullOut !== null ? $tree : forte_digest($tree);
    }
    $cases[] = $case;
}
$suiteHeader = header_fields() + ['suite' => $suite['suite'], 'registries' => (object) $registries];
if ($fullOut !== null) {
    write_json($fullOut, $suiteHeader + ['cases' => $cases]);
    exit(0);
}
write_json(__DIR__ . '/suite.expected.json', $suiteHeader + ['cases' => $cases]);

// Fixtures and repros.
$cases = [];
foreach (['fixtures', 'grammar-gaps/repros'] as $dir) {
    $paths = glob(__DIR__ . '/../' . $dir . '/*.blade.php');
    sort($paths, SORT_STRING);
    foreach ($paths as $path) {
        $rel = $dir . '/' . basename($path);
        array_push($cases, ...both_registries((string) file_get_contents($path), ['path' => $rel]));
    }
}
write_json(__DIR__ . '/fixtures.expected.json', header_fields() + [
    'default_registry' => forte_registry_state(\Forte\Parser\Directives\Directives::withDefaults()),
    'cases' => $cases,
]);

// Corpus samples.
foreach (array_keys(CORPORA) as $name) {
    $files = array_filter(corpus_files($corpusRoot($name)), fn ($bytes) => strlen($bytes) <= 6144);
    $keys = array_keys($files);
    usort($keys, fn ($a, $b) => strcmp(sha1($a), sha1($b)));
    $cases = [];
    foreach (array_slice($keys, 0, 25) as $path) {
        array_push($cases, ...both_registries($files[$path], ['path' => $path]));
    }
    write_json(__DIR__ . "/corpus/{$name}.expected.json", header_fields() + ['corpus' => $corpusHeader($name), 'cases' => $cases]);
}
