<?php
// How the value oracles (`generate_php.php`, `generate_corpus.php`) have PHP evaluate an
// expression: a fresh `php -n` process (no php.ini: precision=14, serialize_precision=-1, 64-bit
// integers) printing the value's tagged encoding, its diagnostics read from stderr. Each process
// runs in this directory, so what a shell command case (`` `ls` ``) evaluates to does not depend
// on where the generator runs. Values are type-tagged: ["null"], ["bool", b], ["int", "decimal"],
// ["float", "IEEE-754 binary64 bits, big-endian hex"], ["string", "bytes as hex"], ["array",
// [[key, value], ...]] in order, keys as ["int", ...] or ["string", ...].

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

/** The command running `$code` (without `<?php`) in a fresh PHP with no php.ini. */
function command(string $code): array {
    return [PHP_BINARY, '-n', '-d', 'error_reporting=-1', '-d', 'display_errors=stderr', '-d', 'log_errors=0', '-r', $code];
}

/** The code printing the encoding of the constant expression `$src` (`const X = <src>;`). */
function constant_code(string $src): string {
    return ENCODE . "\nconst X = $src;\necho json_encode(rerail_encode(X));";
}

/** The code printing the encoding of what `$src` evaluates to at run time, in a function. */
function eval_code(string $src): string {
    return ENCODE . "\necho json_encode(rerail_encode((function () { return $src; })()));";
}

/** What a run printed: its decoded result (`null` when it printed none) and its diagnostics. */
function outcome(string $out, string $err): array {
    $lines = array_values(array_filter(array_map('trim', explode("\n", $err)), fn ($line) => $line !== ''));
    $diagnostics = [];
    foreach ($lines as $line) {
        if (preg_match('/^(?:PHP )?((?:Parse|Fatal) error|Warning|Deprecated|Notice):\s+(.*?)(?: in Command line code(?: on line \d+|:\d+))?$/', $line, $m)) {
            $diagnostics[] = [$m[1], $m[2]];
        }
    }
    return [json_decode($out, true), $diagnostics];
}

function run(string $code): array {
    $process = proc_open(command($code), [1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes, __DIR__);
    $out = stream_get_contents($pipes[1]);
    $err = stream_get_contents($pipes[2]);
    fclose($pipes[1]);
    fclose($pipes[2]);
    proc_close($process);
    return outcome($out, $err);
}

function error_of(array $diagnostics): array {
    foreach ($diagnostics as [$level, $message]) {
        if ($level === 'Parse error' || $level === 'Fatal error') {
            return ['error', preg_replace('/^Uncaught (\w+): (.*)$/', '$1: $2', $message)];
        }
    }
    return ['error', 'no output'];
}

/** The diagnostics other than the error that stopped the run, as `Level: message`. */
function warnings_of(array $diagnostics): array {
    return array_values(array_map(
        fn ($d) => "$d[0]: $d[1]",
        array_filter($diagnostics, fn ($d) => !in_array($d[0], ['Parse error', 'Fatal error'], true)),
    ));
}
