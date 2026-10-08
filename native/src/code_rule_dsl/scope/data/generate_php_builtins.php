<?php
// Writes php_builtins.json: the global names PHP 8.5.11 defines before any application code runs,
// read from a `php -n` process (no ini file, so only the extensions compiled into the binary):
// internal functions (lowercased, as PHP compares them), constants (their canonical spelling,
// compared case-sensitively), and classes, interfaces, traits and enums (lowercased). The
// unqualified function or constant fallback in a namespace reads it (scope/php_names.rs). It
// refuses to run unless PHP's version and loaded extensions are php_runtime.json's, the runtime
// the PHP tables are generated with (scope/php/generate_by_reference.php checks it too). Run
// offline, never in CI, from this directory:
//   /opt/homebrew/Cellar/php/8.5.11/bin/php -n generate_php_builtins.php

declare(strict_types=1);

$runtime = json_decode(
    (string) file_get_contents(__DIR__ . '/php_runtime.json'),
    true,
    flags: JSON_THROW_ON_ERROR,
);
if (php_ini_loaded_file() !== false) {
    throw new RuntimeException('run with php -n: an ini file may load more extensions');
}
if (PHP_VERSION !== $runtime['php']) {
    throw new RuntimeException("PHP {$runtime['php']} required, not " . PHP_VERSION);
}
$extensions = get_loaded_extensions();
sort($extensions, SORT_STRING);
if ($extensions !== $runtime['extensions']) {
    throw new RuntimeException(sprintf(
        'loaded extensions differ from php_runtime.json: missing [%s], extra [%s]',
        implode(', ', array_diff($runtime['extensions'], $extensions)),
        implode(', ', array_diff($extensions, $runtime['extensions'])),
    ));
}

$lower = static function (array $names): array {
    $names = array_values(array_unique(array_map('strtolower', $names)));
    sort($names, SORT_STRING);
    return $names;
};
$constants = array_keys(get_defined_constants());
sort($constants, SORT_STRING);
$data = [
    'php' => PHP_VERSION,
    'extensions' => $extensions,
    'functions' => $lower(get_defined_functions()['internal']),
    'constants' => $constants,
    'classes' => $lower(get_declared_classes()),
    'interfaces' => $lower(get_declared_interfaces()),
    'traits' => $lower(get_declared_traits()),
    'enums' => $lower(array_values(array_filter(get_declared_classes(), 'enum_exists'))),
];
file_put_contents(
    __DIR__ . '/php_builtins.json',
    json_encode($data, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR) . "\n",
);
