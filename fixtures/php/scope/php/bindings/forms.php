<?php
// Every binding form, in the order PHP evaluates them, and the nodes that name each variable.

function destructuring(array $pairs): array
{
    [$a, $b] = [1, 2];
    [, $c] = $pairs;
    ['k' => $k, 'j' => [$j1, $j2]] = $pairs;
    [$a, $a] = [$b, $c];
    foreach ($pairs as $key => [$first, $second]) {
        echo $key, $first, $second;
    }
    foreach ($pairs as [$p => $q]) {
        echo $p, $q;
    }
    return [$a, $k, $j1, $j2];
}

function subscripts(): array
{
    $list = [];
    $list[] = 1;
    $list['k'] = 2;
    $list[0][1] = 3;
    $list[1] += 4;
    $list[2]++;
    $list[3] ??= 5;
    unset($list[0]);
    $object = new stdClass();
    $object->field = $list;
    return $list;
}

function augmented(int $n): int
{
    $total = 0;
    $total += $n;
    $total .= 'x';
    $total ??= 1;
    $total++;
    --$total;
    $other ??= 2;
    return (int) $total + $other;
}

function references(array $items): int
{
    $count = 0;
    $alias = &$count;
    $alias = 3;
    foreach ($items as &$item) {
        $item = 1;
    }
    unset($item);
    preg_match('/a/', 'a', $matches);
    preg_match(subject: 'a', matches: $named, pattern: '/a/');
    sort($items);
    array_push($items, 1);
    $parsed = null;
    parse_str('a=1', $parsed);
    return $count + count($matches) + count($named) + count($parsed);
}

function scopes(): void
{
    global $config, $settings;
    static $calls = 0, $cache;
    $calls++;
    $settings = $config;
    $cache = &$GLOBALS['config'];
    try {
        throw new RuntimeException('x');
    } catch (RuntimeException $error) {
        echo $error->getMessage();
    } catch (LogicException) {
    } finally {
        $calls = 0;
    }
}

function loops(array $items): int
{
    for ($i = 0, $sum = 0; $i < count($items); $i++) {
        $sum += $items[$i];
    }
    $j = 0;
    while ($j < 3) {
        $j++;
    }
    do {
        $k = $j;
    } while ($k > 10);
    switch ($sum) {
        case 1:
            $label = 'one';
            break;
        default:
            $label = 'many';
    }
    $result = match (true) {
        $sum > 1 => $label,
        default => ($fallback = 'none'),
    };
    return $sum + $j + $k + strlen($result) + strlen($fallback ?? '');
}

function closures(int $outer): array
{
    $base = 1;
    $byValue = function (int $x) use ($base, $outer): int {
        $base = $base + $x;
        return $base + $outer;
    };
    $byReference = function () use (&$base, &$later): void {
        $base = 2;
        $later = 3;
    };
    $arrow = fn(int $y) => $y + $base + $outer;
    $arrowLocal = fn() => ($local = $base) + $local;
    $nested = fn($z) => fn() => $z + $base;
    $shadow = fn($base) => $base * 2;
    $static = static fn() => $outer;
    return [$byValue, $byReference, $arrow, $arrowLocal, $nested, $shadow, $static, $later];
}

final class Methods
{
    public function __construct(private int $promoted, int $plain)
    {
        $this->promoted = $plain + $promoted;
    }

    public static function make(string ...$names): string
    {
        $joined = implode(',', $names);
        $made = new class($joined) {
            public function __construct(public string $value)
            {
            }
        };
        return $made->value;
    }

    public function interpolate(array $row): string
    {
        $name = 'n';
        return "{$name} $row[0] {$row['key']}";
    }
}

interface Contract
{
    public function signature(int $unused, string &$out): void;
}

$top = 1;
$top .= 'level';
compact('top');
echo isset($missing) ? $missing : $top;
