<?php
// Records every template Forte 1.1.0's own test suite lexes, with the directive registry it was
// lexed with, into suite.inputs.json: the inputs `generate.php` parses for suite.expected.json.
//
//   php record_suite.php <forte checkout> [<out>]
//
// <forte checkout> is github.com/fortephp/forte at tag v1.1.0 (commit 9c9597d0b7, see
// FORTE_COMMIT) with its dev dependencies installed and laravel/framework pinned:
//   composer install && composer require --dev -W laravel/framework:13.34.0
// The script adds one line to the top of `Lexer::scan()` that hands the source and registry to a
// recorder (auto-prepended), runs the suite (`vendor/bin/pest`, which must pass), and restores
// the file. Every distinct (template, registry) pair is kept with the test files that lexed it.
// Default <out>: suite.inputs.json next to this script.
declare(strict_types=1);

if (getenv('FORTE_RECORD_OUT') !== false) {
    // Auto-prepended into the suite's PHP process.
    require_once __DIR__ . '/forte_tree.php';

    final class ForteSuiteRecorder
    {
        private static $out = null;

        /** @var array<string, true> */
        private static array $seen = [];

        public static function record(string $source, \Forte\Parser\Directives\Directives $directives): void
        {
            self::$out ??= fopen(getenv('FORTE_RECORD_OUT'), 'a');
            $test = '';
            $prefixes = false;
            foreach (debug_backtrace(0, 40) as $frame) {
                // `assertIncrementalParsing` lexes every prefix of its template (as
                // `forte_prefix_positions` picks them): record the template once.
                if (($frame['function'] ?? '') === 'assertIncrementalParsing' && ! $prefixes) {
                    $source = $frame['args'][0];
                    $prefixes = true;
                }
                $file = $frame['file'] ?? '';
                if ($test === '' && preg_match('~/(tests/.+Test\.php)$~', $file, $m)) {
                    $test = $m[1];
                }
            }
            $state = forte_registry_state($directives);
            $name = forte_registry_name($state);
            $key = sha1($name . "\0" . ($prefixes ? 'p' : 's') . "\0" . $test . "\0" . $source);
            if (isset(self::$seen[$key])) {
                return;
            }
            self::$seen[$key] = true;
            fwrite(self::$out, json_encode([
                'source_base64' => base64_encode($source),
                'registry' => $name,
                'prefixes' => $prefixes,
                'state' => $name === 'default' || $name === 'accept-all' ? null : $state,
                'test' => $test,
            ], JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR) . "\n");
        }
    }

    return;
}

ini_set('memory_limit', '8G');

$checkout = realpath($argv[1] ?? '') ?: '';
$out = $argv[2] ?? __DIR__ . '/suite.inputs.json';
if ($checkout === '' || ! is_file($checkout . '/vendor/autoload.php')) {
    fwrite(STDERR, "usage: php record_suite.php <forte checkout with vendor/> [<out>]\n");
    exit(1);
}

require __DIR__ . '/forte_tree.php';
$commit = trim((string) shell_exec('git -C ' . escapeshellarg($checkout) . ' rev-parse HEAD'));
if ($commit !== FORTE_COMMIT) {
    fwrite(STDERR, "{$checkout} is at {$commit}, not v" . FORTE_VERSION . ' (' . FORTE_COMMIT . ")\n");
    exit(1);
}
$installed = json_decode((string) file_get_contents($checkout . '/vendor/composer/installed.json'), true);
$laravel = null;
foreach ($installed['packages'] as $package) {
    if ($package['name'] === 'laravel/framework') {
        $laravel = $package['version'];
    }
}
if ($laravel !== 'v13.34.0' || PHP_VERSION !== '8.5.11') {
    fwrite(STDERR, "needs laravel/framework v13.34.0 (has {$laravel}) and PHP 8.5.11 (runs " . PHP_VERSION . ")\n");
    exit(1);
}

$lexer = $checkout . '/src/Lexer/Lexer.php';
$original = (string) file_get_contents($lexer);
$hook = "    private function scan(): void\n    {\n";
if (substr_count($original, $hook) !== 1) {
    fwrite(STDERR, "Lexer::scan() not found where expected\n");
    exit(1);
}
$raw = $out . '.raw';
@unlink($raw);
file_put_contents($lexer, str_replace(
    $hook,
    $hook . "        \\ForteSuiteRecorder::record(\$this->source, \$this->directives);\n",
    $original
));
try {
    $command = sprintf(
        'cd %s && FORTE_RECORD_OUT=%s %s -d memory_limit=4G -d auto_prepend_file=%s vendor/bin/pest --compact 2>&1',
        escapeshellarg($checkout),
        escapeshellarg($raw),
        escapeshellarg(PHP_BINARY),
        escapeshellarg(__FILE__)
    );
    exec($command, $output, $code);
} finally {
    file_put_contents($lexer, $original);
}
if ($code !== 0) {
    fwrite(STDERR, implode("\n", array_slice($output, -30)) . "\nthe suite failed\n");
    exit(1);
}
$summary = '';
foreach ($output as $line) {
    if (str_contains($line, 'Tests:')) {
        $summary = trim($line);
    }
}

// Distinct (template, registry, prefixes) triples, in first-lexed order, with the tests that
// lexed them. `prefixes`: the test lexed every prefix `forte_prefix_positions` picks.
$cases = [];
$registries = [];
$handle = fopen($raw, 'r');
while (($line = fgets($handle)) !== false) {
    $record = json_decode($line, true, flags: JSON_THROW_ON_ERROR);
    $source = base64_decode($record['source_base64']);
    $name = $record['registry'];
    if ($record['state'] !== null) {
        $registries[$name] = $record['state'];
    }
    $key = $name . "\0" . ($record['prefixes'] ? 'p' : 's') . "\0" . $source;
    if (! isset($cases[$key])) {
        $cases[$key] = forte_source_json($source) + ['registry' => $name];
        if ($record['prefixes']) {
            $cases[$key]['prefixes'] = true;
        }
        $cases[$key]['tests'] = [];
    }
    if ($record['test'] !== '' && ! in_array($record['test'], $cases[$key]['tests'], true)) {
        $cases[$key]['tests'][] = $record['test'];
    }
}
fclose($handle);
unlink($raw);
ksort($registries);

file_put_contents($out, json_encode([
    'tool' => 'fortephp/forte',
    'version' => FORTE_VERSION,
    'commit' => FORTE_COMMIT,
    'suite' => $summary,
    'registries' => (object) $registries,
    'cases' => array_values($cases),
], FORTE_JSON) . "\n");
fwrite(STDERR, count($cases) . ' inputs, ' . count($registries) . " custom registries ({$summary})\n");
