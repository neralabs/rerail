<?php
// Writes destructuring.expected.json: for every case in destructuring.cases.json, the value PHP
// 8.5.11 gives each variable the case reads after running its code in a function of its own,
// each case in a fresh `php -n` process (no php.ini: 64-bit integers, precision=14,
// serialize_precision=-1). `values` maps each read variable to its value, or ["undefined"] where
// the function does not define it; `warnings` lists what PHP printed while it ran (an undefined
// array key, a deprecated float key). A case's `unknown` (copied) says why Rerail does not know a
// variable's value. Run offline, not in CI:
//   /opt/homebrew/Cellar/php/8.5.11/bin/php generate_destructuring.php
// Values are type-tagged as value/generate_php.php writes them: ["null"], ["bool", b],
// ["int", "decimal"], ["float", "IEEE-754 binary64 bits, big-endian hex"], ["string", "bytes as
// hex"], ["array", [[key, value], ...]] in order.

declare(strict_types=1);

if (PHP_VERSION !== '8.5.11') {
    throw new RuntimeException('PHP 8.5.11 required, not ' . PHP_VERSION);
}

const ENCODE = <<<'PHP'
function rerail_encode(mixed $value): array {
    return match (true) {
        $value === null => ['null'],
        is_bool($value) => ['bool', $value],
        is_int($value) => ['int', (string) $value],
        is_float($value) => ['float', bin2hex(pack('E', $value))],
        is_string($value) => ['string', bin2hex($value)],
        is_array($value) => ['array', array_map(
            fn ($key) => [rerail_encode($key), rerail_encode($value[$key])],
            array_keys($value),
        )],
        default => ['other', get_debug_type($value)],
    };
}
PHP;

function run(string $code): array
{
    $process = proc_open(
        [PHP_BINARY, '-n', '-d', 'error_reporting=-1', '-d', 'display_errors=stderr', '-d', 'log_errors=0', '-r', $code],
        [1 => ['pipe', 'w'], 2 => ['pipe', 'w']],
        $pipes,
    );
    $out = stream_get_contents($pipes[1]);
    $err = stream_get_contents($pipes[2]);
    fclose($pipes[1]);
    fclose($pipes[2]);
    proc_close($process);
    $warnings = [];
    foreach (explode("\n", $err) as $line) {
        if (preg_match('/^(?:PHP )?((?:Parse|Fatal) error|Warning|Deprecated|Notice):\s+(.*?)(?: in Command line code(?: on line \d+|:\d+))?$/', trim($line), $m)) {
            $warnings[] = "$m[1]: $m[2]";
        }
    }
    return [json_decode($out, true), $warnings];
}

$here = __DIR__;
$cases = json_decode(file_get_contents("$here/destructuring.cases.json"), true, flags: JSON_THROW_ON_ERROR);
$out = ['php' => PHP_VERSION, 'intSize' => PHP_INT_SIZE, 'cases' => []];
foreach ($cases['cases'] as $case) {
    $reads = var_export($case['read'], true);
    $program = ENCODE . "\nfunction t() {\n{$case['code']}\nreturn get_defined_vars();\n}\n"
        . "\$vars = t();\n"
        . "echo json_encode(array_combine($reads, array_map(fn (\$name) => array_key_exists(\$name, \$vars) ? rerail_encode(\$vars[\$name]) : ['undefined'], $reads)));";
    [$values, $warnings] = run($program);
    if (!is_array($values)) {
        fwrite(STDERR, "case failed: {$case['code']}: " . json_encode($warnings) . "\n");
        exit(1);
    }
    $entry = ['code' => $case['code'], 'read' => $case['read'], 'values' => $values, 'warnings' => $warnings];
    if (isset($case['unknown'])) {
        $entry['unknown'] = $case['unknown'];
    }
    $out['cases'][] = $entry;
}
file_put_contents(
    "$here/destructuring.expected.json",
    json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_THROW_ON_ERROR) . "\n",
);
