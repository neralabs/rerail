<?php
// T1 reference counts for the Forte port: per pinned corpus, the templates each reference parser
// reports an error in.
//
//   php t1_reference.php <blade-parsers project> <checkout> <work dir> [OUT]
//
// - `forte`: Forte 1.1.0 `Document::parse` (default registry) reports a diagnostic; `forte_all`:
//   the same with every `@word` a directive (accept-all registry). Forte's diagnostics are its
//   lexer's errors (its tree builder records none) or the exception its depth limits throw.
// - `compiler`: illuminate/view 13.34.0's `BladeCompiler` (core directives only, no application
//   directives; `withoutComponentTags()`, since compiling `<x-…>` tags resolves the application's
//   component classes and views) throws, or `php -l` (PHP 8.5.11) rejects the PHP it compiles the
//   template to.
// Templates are selected as `generate.php --corpus` selects them. Writes JSON to OUT (default
// stdout): per corpus the counts and the paths. <work dir> holds compiled templates.
declare(strict_types=1);

ini_set('memory_limit', '4G');
require __DIR__ . '/forte_tree.php';

[$project, $checkout, $work] = array_slice($argv, 1, 3) + [null, null, null];
$out = $argv[4] ?? null;
if ($project === null || $checkout === null || $work === null) {
    fwrite(STDERR, "usage: php t1_reference.php <blade-parsers project> <checkout> <work dir> [OUT]\n");
    exit(1);
}
forte_require($project);
@mkdir($work . '/compiled', 0777, true);

use Illuminate\Filesystem\Filesystem;
use Illuminate\Foundation\Application;
use Illuminate\View\Compilers\BladeCompiler;

$app = new Application($work);
\Illuminate\Support\Facades\Facade::setFacadeApplication($app);
\Illuminate\Container\Container::setInstance($app);
$app->register(\Illuminate\View\ViewServiceProvider::class);
$app->register(\Illuminate\Filesystem\FilesystemServiceProvider::class);
$app->instance('config', new \Illuminate\Config\Repository(['view' => ['paths' => [], 'compiled' => $work . '/compiled']]));
$app->boot();
$compiler = new BladeCompiler(new Filesystem(), $work . '/compiled');
$compiler->withoutComponentTags();

$corpora = [
    'bookstack' => 'blade/bookstack',
    'filament' => 'blade/filament',
    'bagisto' => 'blade/bagisto',
    'laravel' => 'php/laravel',
];

$report = header_fields_t1();
foreach ($corpora as $name => $dir) {
    $root = $checkout . '/.rerail/corpora/' . $dir;
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

    $forte = $forteAll = $compiled = [];
    foreach ($files as $path => $template) {
        foreach (['default' => &$forte, 'accept-all' => &$forteAll] as $registry => &$list) {
            $tree = forte_tree($template, forte_registry($registry, []));
            if (isset($tree['x']) || $tree['e'] !== []) {
                $list[] = $path;
            }
        }
        unset($list);
        try {
            $php = $compiler->compileString($template);
            $lint = $work . '/lint.php';
            file_put_contents($lint, $php);
            $output = [];
            exec(escapeshellarg(PHP_BINARY) . ' -l ' . escapeshellarg($lint) . ' 2>&1', $output, $code);
            if ($code !== 0) {
                $compiled[] = $path;
            }
        } catch (\Throwable $e) {
            $compiled[] = $path;
        }
    }
    $report['corpora'][$name] = [
        'files' => count($files),
        'forteErrorFiles' => count($forte),
        'forteAcceptAllErrorFiles' => count($forteAll),
        'compilerErrorFiles' => count($compiled),
        'forte' => $forte,
        'forteAcceptAll' => $forteAll,
        'compiler' => $compiled,
    ];
    fwrite(STDERR, sprintf("%s: %d files, forte %d, forte accept-all %d, compiler+php -l %d\n",
        $name, count($files), count($forte), count($forteAll), count($compiled)));
}

function header_fields_t1(): array
{
    return [
        'tools' => [
            ['name' => 'fortephp/forte', 'version' => FORTE_VERSION],
            ['name' => 'illuminate/view (BladeCompiler) + php -l', 'version' => \Composer\InstalledVersions::getPrettyVersion('laravel/framework') . ', PHP ' . PHP_VERSION],
        ],
    ];
}

$json = json_encode($report, FORTE_JSON | JSON_PRETTY_PRINT) . "\n";
if ($out === null) {
    echo $json;
} else {
    file_put_contents($out, $json);
}
