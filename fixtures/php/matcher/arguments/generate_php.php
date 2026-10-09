<?php
// Writes php.expected.json: for every case in php.cases.json, how PHP 8.5.11 binds the call's
// arguments to the case's signature. Each case runs in a fresh `php -n` process as
//   const RERAIL_ABSENT = "\0absent";
//   function f($a = RERAIL_ABSENT, $b = RERAIL_ABSENT, ..., ...$rerail_rest) { return [$a, $b, ...]; }
//   <prelude>; echo json_encode(f(<call>));
// or, with `callee: "new class"`, passes the arguments to an anonymous class's constructor of
// those parameters, `new class(<call>) { function __construct(...) {...} }`.
// The trailing variadic stands for parameters a policy's signature does not list, so a named
// argument outside the signature is not an error. `slots` holds each slot's bound value as the
// value oracle encodes values (`["absent"]` where the default was used), or `error` the error PHP
// raises (at compile time or while binding). `unknown` (copied from the case) names each slot
// Rerail may not know, with why. Run offline, not in CI:
//   /opt/homebrew/Cellar/php/8.5.11/bin/php generate_php.php

const ENCODE = <<<'PHP'
function rerail_encode(mixed $value): array {
    return match (true) {
        $value === "\0absent" => ['absent'],
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

function run(string $code): array {
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
    $result = json_decode($out, true);
    if (is_array($result)) {
        return ['slots' => $result];
    }
    foreach (explode("\n", $err) as $line) {
        if (preg_match('/^(?:PHP )?(?:Parse|Fatal) error:\s+(?:Uncaught (\w+): )?(.*?)(?: in Command line code(?: on line \d+|:\d+))?$/', trim($line), $m)) {
            return ['error' => ($m[1] !== '' ? "$m[1]: " : '') . $m[2]];
        }
    }
    return ['error' => 'no output'];
}

$cases = json_decode(file_get_contents(__DIR__ . '/php.cases.json'), true, flags: JSON_THROW_ON_ERROR);
$out = ['php' => PHP_VERSION, 'cases' => []];
foreach ($cases['cases'] as $case) {
    $params = implode(', ', array_map(fn ($name) => "\$$name = RERAIL_ABSENT", $case['signature']));
    $values = implode(', ', array_map(fn ($name) => "\$$name", $case['signature']));
    $prelude = $case['prelude'] ?? '';
    $code = ENCODE . "\nconst RERAIL_ABSENT = \"\\0absent\";\n" . match ($case['callee'] ?? null) {
        null => "function f($params, ...\$rerail_rest) { return array_map('rerail_encode', [$values]); }\n"
            . "$prelude\necho json_encode(f({$case['call']}));",
        'new class' => "$prelude\necho json_encode((new class({$case['call']}) {\n"
            . "    public array \$slots;\n"
            . "    public function __construct($params, ...\$rerail_rest) { \$this->slots = array_map('rerail_encode', [$values]); }\n"
            . "})->slots);",
    };
    $out['cases'][] = $case + run($code);
}
file_put_contents(
    __DIR__ . '/php.expected.json',
    json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_THROW_ON_ERROR) . "\n",
);
