<?php
// Writes printf.expected.json: for PHP printf-family format strings, what PHP 8.5.11's
// `php_formatted_print` (sprintf, printf, vsprintf, fprintf...) makes of each, and how PHPStan
// 2.2.16's `PrintfFormatParser` (the parser `PrintfParametersRule` and
// `PrintfArrayParametersRule`, `argument.sprintf`, read) tells its argument uses.
//
// Each case is [format, runs, uses]:
// - `runs`: sprintf($format, ...$values) with 0..8 and 1000 integer values (3 each, so a `*` width
//   or precision is valid): "ok", or [class, message] of the ValueError or ArgumentCountError it
//   throws.
// - `uses`: PHPStan's argument uses in the order the format consumes them, [index, kind] with the
//   index from 0 after the format and kind "width", "precision" or "value"; null where PHPStan
//   reads the format as invalid.
//
// The cases are hand-written plus seeded random strings over the bytes the grammar gives meaning
// (UTF-8 only: Rerail parses a literal's decoded text). Run offline, not in CI:
//   /opt/homebrew/Cellar/php/8.5.11/bin/php -n generate_printf.php \
//     /tmp/rerail-php-tools/vendor/phpstan/phpstan/phpstan.phar
// Requires PHP 8.5.11 and PHPStan 2.2.16 (composer require phpstan/phpstan:2.2.16).

if (PHP_VERSION !== '8.5.11') {
    fwrite(STDERR, "PHP 8.5.11 required, found " . PHP_VERSION . "\n");
    exit(1);
}
$phar = $argv[1] ?? '';
require "phar://$phar/vendor/autoload.php";
$version = trim((string) shell_exec(escapeshellarg(PHP_BINARY) . ' ' . escapeshellarg($phar) . ' --version'));
if (!str_contains($version, '2.2.16')) {
    fwrite(STDERR, "PHPStan 2.2.16 required, found $version\n");
    exit(1);
}
$parser = new PHPStan\Type\Php\PrintfFormatParser(new PHPStan\Php\PhpVersion(80500));

const RUNS = [0, 1, 2, 3, 4, 5, 6, 8, 1000];

$cases = [
    '',
    'plain',
    '%s',
    '%%',
    '%% %s %%',
    '%',
    'abc %',
    '%s %',
    '%5',
    '%.3',
    '%-',
    '%l',
    '%ls',
    '%ll',
    '%lld',
    '%s %s',
    '%s %d %u %f %F %e %E %g %G %h %H %c %o %x %X %b',
    '%i',
    '%n',
    '%a',
    '%r',
    '%z',
    '%5z',
    '%.5z',
    '%-5z',
    '%1$s',
    '%2$s',
    '%1$s %1$s',
    '%2$s %1$s',
    '%2$s %s',
    '%s %2$s %s',
    '%0$s',
    '%$s',
    '%01$s',
    '%00$s',
    '%1$',
    '%1$5',
    '%1$-05.2f',
    '%10$s',
    '%2147483646$s',
    '%2147483647$s',
    '%99999999999$s',
    '%5%',
    '%-%',
    '%1$%',
    '%.2%',
    '%l%',
    '%5%%s',
    '%5%z',
    '%5%0$s',
    "%5%'",
    "%'",
    "%'x",
    "%'xs",
    "%'*10s",
    "%'%10s",
    "%'%",
    "%-'x10d",
    "%+'010d",
    '%05d',
    '%-5d',
    '%+d',
    '% d',
    '%0-+ 5d',
    '%-+ 0',
    '%*s',
    '%*d %s',
    '%*s %s',
    '%.*f',
    '%*.*f',
    '%*.*f %s',
    '%-*.*f',
    '%*1$s',
    '%*2$s',
    '%*2$.*3$f',
    '%1$*2$.*3$f',
    '%*0$s',
    '%*$s',
    '%.*0$f',
    '%*',
    '%.*',
    '%*%',
    '%*%s',
    '%.*%s',
    '%*l',
    '%*ls',
    '%2147483646$d',
    '%2147483647d',
    '%99999999999d',
    '%.2147483647f',
    '%.99999999999f',
    '%.f',
    '%.s',
    '%.',
    '%5.3s',
    '%-10.3s|',
    '%é',
    '%5é',
    'é%sü',
    "%'és",
    "a\0b",
    "%\0",
    "%5\0",
    '%s%',
    '%%%',
    '%%%s',
    '%%%%',
    '100%',
    '100% sure',
    '%s and 100%',
    '%B',
    '%D',
    '%S',
    '%L',
    '%1',
    '%12',
    '%1$1$s',
    '%1$*s',
    '%$',
];

// Seeded random strings over the bytes the grammar reads; no run of four digits, so no width
// allocates past a few kilobytes.
$alphabet = [
    '%', '%', '%', '%', 's', 's', 'd', 'd', 'u', 'f', 'F', 'e', 'g', 'G', 'h', 'c', 'o', 'x', 'X', 'b',
    'z', 'l', 'i', 'q', '1', '2', '0', '9', '$', '*', '.', '-', '+', ' ', "'", 'é', 'a', '%%',
];
mt_srand(1678, MT_RAND_MT19937);
$seen = array_flip($cases);
while (count($cases) < 5000) {
    $text = '';
    $length = mt_rand(1, 10);
    for ($index = 0; $index < $length; $index++) {
        $text .= $alphabet[mt_rand(0, count($alphabet) - 1)];
    }
    if (preg_match('/\d{4}/', $text) || isset($seen[$text])) {
        continue;
    }
    $seen[$text] = true;
    $cases[] = $text;
}

function run(string $format, int $count): string|array
{
    try {
        sprintf($format, ...array_fill(0, $count, 3));
        return 'ok';
    } catch (ValueError | ArgumentCountError $error) {
        return [get_class($error), $error->getMessage()];
    }
}

$records = [];
foreach ($cases as $format) {
    $runs = array_map(fn (int $count) => run($format, $count), RUNS);
    $uses = $parser->parse($format);
    $records[] = [
        $format,
        $runs,
        $uses === null ? null : array_map(fn (array $use) => [$use['index'], $use['kind']], $uses),
    ];
}

$out = json_encode(
    ['php' => PHP_VERSION, 'phpstan' => $version, 'runs' => RUNS, 'cases' => $records],
    JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE | JSON_THROW_ON_ERROR,
);
file_put_contents(__DIR__ . '/printf.expected.json', $out . "\n");
fwrite(STDERR, count($records) . " cases\n");
