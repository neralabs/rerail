<?php
// Writes php.expected.json: for every expression in php.cases.json, what PHP 8.5.11 makes of it as
// a constant expression (`const X = <src>;`), each in a fresh `php -n` process (no php.ini:
// precision=14, serialize_precision=-1, 64-bit integers; `evaluate.php`). `const` is the value, or
// the error PHP raises (a parse error, "Constant expression contains invalid operations", an Error
// thrown while evaluating); `warnings` lists the diagnostics PHP printed while it evaluated. Where
// PHP refuses the constant expression, `eval` is what the expression evaluates to at run time in a
// function with no variables (a value or an error). For every pair of `pairs.operands`,
// `pairs.results` records PHP's `===`, `==` and `<=>`. Each process runs in this directory, so what
// a shell command case (`` `ls` ``) evaluates to does not depend on where the generator runs. Run
// offline, not in CI:
//   /opt/homebrew/Cellar/php/8.5.11/bin/php generate_php.php
// Values are tagged as `evaluate.php` encodes them.

require __DIR__ . '/evaluate.php';

$here = __DIR__;
$cases = json_decode(file_get_contents("$here/php.cases.json"), true, flags: JSON_THROW_ON_ERROR);
$out = ['php' => PHP_VERSION, 'intSize' => PHP_INT_SIZE, 'cases' => [], 'pairs' => []];
foreach ($cases['expressions'] as $case) {
    $src = is_array($case) ? $case['src'] : $case;
    [$result, $diagnostics] = run(constant_code($src));
    $entry = ['src' => $src, 'const' => $result ?? error_of($diagnostics), 'warnings' => warnings_of($diagnostics)];
    if ($entry['const'][0] === 'error') {
        [$evaluated, $evalDiagnostics] = run(eval_code($src));
        $entry['eval'] = $evaluated ?? error_of($evalDiagnostics);
    }
    if (is_array($case) && isset($case['unknown'])) {
        $entry['unknown'] = $case['unknown'];
    }
    $out['cases'][] = $entry;
}
$operands = $cases['pairs'];
$left = implode(', ', $operands);
$code = ENCODE . "\nconst V = [$left];\n\$r = [];\nforeach (V as \$a) { foreach (V as \$b) { \$r[] = [\$a === \$b, \$a == \$b, \$a <=> \$b]; } }\necho json_encode(\$r);";
[$results, $diagnostics] = run($code);
if (!is_array($results)) {
    fwrite(STDERR, "pairs failed: " . json_encode($diagnostics) . "\n");
    exit(1);
}
$out['pairs'] = ['operands' => $operands, 'results' => array_chunk($results, count($operands))];
file_put_contents(
    "$here/php.expected.json",
    json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_THROW_ON_ERROR) . "\n",
);
