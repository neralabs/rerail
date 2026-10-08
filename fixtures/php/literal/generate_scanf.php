<?php
// Writes scanf.expected.json: for PHP sscanf/fscanf format strings, what PHP 8.5.11's
// `ValidateFormat` (ext/standard/scanf.c) makes of each. Each case is [format, runs]: `runs` is
// sscanf("\x01", $format, ...$variables) with 0..6 variables passed by reference: "ok", or the
// ValueError message it throws (with no variable the call returns an array of the variables the
// format would assign, recorded as ["ok", count]; an empty input would return -1 instead).
//
// The cases are hand-written plus seeded random strings over the bytes the grammar gives meaning
// (UTF-8 only: Rerail parses a literal's decoded text). Run offline, not in CI:
//   /opt/homebrew/Cellar/php/8.5.11/bin/php -n generate_scanf.php

if (PHP_VERSION !== '8.5.11') {
    fwrite(STDERR, "PHP 8.5.11 required, found " . PHP_VERSION . "\n");
    exit(1);
}

const RUNS = [0, 1, 2, 3, 4, 5, 6];

$cases = [
    '',
    'plain',
    '%d',
    '%%',
    '%% %d %%',
    '%',
    'abc %',
    '%d %',
    '%s %d',
    '%n %d %D %i %o %x %X %u %f %e %E %g %s %c',
    '%a',
    '%z',
    '%F',
    '%G',
    '%b',
    '%p',
    '%*d',
    '%*d %d',
    '%d %*s %d',
    '%*',
    '%*5s',
    '%*5$s',
    '%5s',
    '%5',
    '%ld %Ld %hd',
    '%lld',
    '%l',
    '%5ls',
    '%1$s',
    '%2$s',
    '%1$s %2$s',
    '%2$s %1$s',
    '%1$s %1$s',
    '%2$s %2$s %1$s',
    '%3$s',
    '%1$s %d',
    '%d %1$s',
    '%*d %1$s',
    '%1$s %*d',
    '%0$s',
    '%00$s',
    '%01$s',
    '%255$s',
    '%256$s',
    '%4294967297$s',
    '%99999999999999999999$s',
    '%1$5s',
    '%1$ld',
    '%1$*d',
    '%1$',
    '%1',
    '%[a-z]',
    '%[^a-z]',
    '%[]a]',
    '%[^]a]',
    '%[]',
    '%[^]',
    '%[',
    '%[^',
    '%[a',
    '%5[abc]%d',
    '%*[ ]%s',
    '%1$[abc] %2$d',
    '%c%c',
    'é%sü',
    '%é',
    "a\0%d",
    "%d\0%d",
    "%\0",
    '%s%',
    '%%%',
    '%%%d',
    '100%',
    '%d,%d;%s',
];

$alphabet = [
    '%', '%', '%', '%', 's', 's', 'd', 'd', 'c', 'n', 'D', 'i', 'x', 'f', 'e', 'g', 'u', 'o',
    'z', 'a', 'l', 'h', 'L', '1', '2', '0', '9', '$', '$', '*', '[', ']', '^', ' ', 'é', '%%',
];
mt_srand(1678, MT_RAND_MT19937);
$seen = array_flip($cases);
while (count($cases) < 5000) {
    $text = '';
    $length = mt_rand(1, 10);
    for ($index = 0; $index < $length; $index++) {
        $text .= $alphabet[mt_rand(0, count($alphabet) - 1)];
    }
    if (isset($seen[$text])) {
        continue;
    }
    $seen[$text] = true;
    $cases[] = $text;
}

function run(string $format, int $count): string|array
{
    $variables = array_fill(0, $count, null);
    $references = [];
    foreach ($variables as $key => &$variable) {
        $references[] = &$variable;
    }
    try {
        $result = sscanf("\x01", $format, ...$references);
        return $count === 0 ? ['ok', is_array($result) ? count($result) : -1] : 'ok';
    } catch (ValueError $error) {
        return $error->getMessage();
    }
}

$records = [];
foreach ($cases as $format) {
    $records[] = [$format, array_map(fn (int $count) => run($format, $count), RUNS)];
}

$out = json_encode(
    ['php' => PHP_VERSION, 'runs' => RUNS, 'cases' => $records],
    JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE | JSON_THROW_ON_ERROR,
);
file_put_contents(__DIR__ . '/scanf.expected.json', $out . "\n");
fwrite(STDERR, count($records) . " cases\n");
