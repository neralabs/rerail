<?php
// Statement and expression decisions: if/elseif/else if, loops, catch, switch and match (once, however many cases),
// &&, || and the ?: conditional, in brace and alternative syntax.

function ifElseAtLimit(int $x): string
{
    if ($x === 1) {
        return 'one';
    } elseif ($x === 2) {
        return 'two';
    } elseif ($x === 3) {
        return 'three';
    } else if ($x === 4) {
        return 'four';
    } elseif ($x === 5) {
        return 'five';
    } else {
        return 'many';
    }
}

function ifElseOverLimit(int $x): string
{
    if ($x === 1) {
        return 'one';
    } elseif ($x === 2) {
        return 'two';
    } elseif ($x === 3) {
        return 'three';
    } else if ($x === 4) {
        return 'four';
    } elseif ($x === 5) {
        return 'five';
    } elseif ($x === 6) {
        return 'six';
    }
    return 'many';
}

function loopsAtLimit(array $rows, array $keys): int
{
    $total = 0;
    for ($index = 0; $index < count($rows); $index++) {
        $total += $index;
    }
    foreach ($keys as $key => $value) {
        $total += strlen($key);
    }
    while ($total > 100) {
        $total -= 1;
    }
    do {
        $total += 1;
    } while ($total < 0);
    foreach ($rows as $row) {
        $total += count($row);
    }
    return $total;
}

function loopsOverLimit(array $rows, array $keys): int
{
    $total = 0;
    for ($index = 0; $index < count($rows); $index++) {
        $total += $index;
    }
    foreach ($keys as $key => $value) {
        $total += strlen($key);
    }
    while ($total > 100) {
        $total -= 1;
    }
    do {
        $total += 1;
    } while ($total < 0);
    foreach ($rows as $row) {
        $total += count($row);
    }
    for ($index = 0; $index < 2; $index++) {
        $total += 2;
    }
    return $total;
}

function catchOverLimit(callable $step): string
{
    try {
        $step();
    } catch (InvalidArgumentException $error) {
        return 'invalid';
    } catch (RuntimeException | LogicException $error) {
        return 'runtime';
    } finally {
        $step();
    }
    try {
        $step();
    } catch (TypeError $error) {
        return 'type';
    }
    try {
        $step();
    } catch (ValueError $error) {
        return 'value';
    }
    try {
        $step();
    } catch (ArithmeticError $error) {
        return 'arithmetic';
    }
    try {
        $step();
    } catch (Error $error) {
        return 'error';
    }
    try {
        $step();
    } catch (Exception $error) {
        return 'exception';
    }
    return 'ok';
}

function switchCountsOnce(string $code): string
{
    switch ($code) {
        case 'a':
            return 'alpha';
        case 'b':
            return 'bravo';
        case 'c':
            return 'charlie';
        case 'd':
            return 'delta';
        case 'e':
            return 'echo';
        case 'f':
            return 'foxtrot';
        default:
            return 'unknown';
    }
}

function switchOverLimit(string $code, int $level): string
{
    switch ($code) {
        case 'a':
            return 'alpha';
        case 'b':
            return 'bravo';
    }
    if ($level > 1) {
        return 'high';
    }
    if ($level > 2) {
        return 'higher';
    }
    if ($level > 3) {
        return 'highest';
    }
    if ($level > 4) {
        return 'top';
    }
    if ($level > 5) {
        return 'peak';
    }
    if ($level > 6) {
        return 'summit';
    }
    return 'low';
}

function nestedSwitches(string $outer, string $inner): string
{
    switch ($outer) {
        case 'x':
            switch ($inner) {
                case 'p':
                    return 'xp';
                case 'q':
                    return 'xq';
            }
            return 'x';
        case 'y':
            return 'y';
    }
    return 'none';
}

function switchDefaultOnly(string $code): string
{
    switch ($code) {
        default:
            return $code;
    }
}

function matchCountsOnce(int $code): string
{
    return match ($code) {
        1 => 'one',
        2 => 'two',
        3 => 'three',
        4, 5 => 'few',
        6 => 'six',
        7 => 'seven',
        default => 'many',
    };
}

function matchOverLimit(int $code, bool $loud, bool $quiet): string
{
    $word = match (true) {
        $code > 10 && $loud => 'big',
        $code > 5 || $quiet => 'medium',
        default => 'small',
    };
    if ($loud) {
        $word = strtoupper($word);
    }
    if ($quiet) {
        $word = strtolower($word);
    }
    return $code > 0 ? $word : '';
}

function logicalAtLimit(bool $a, bool $b, bool $c, bool $d, bool $e, bool $f): bool
{
    return $a && $b || $c && $d || $e && $f;
}

function logicalOverLimit(bool $a, bool $b, bool $c, bool $d, bool $e, bool $f, bool $g): bool
{
    return $a && $b || $c && $d || $e && $f || $g;
}

function ternaryOverLimit(int $x): string
{
    return $x === 1 ? 'one'
        : ($x === 2 ? 'two'
        : ($x === 3 ? 'three'
        : ($x === 4 ? 'four'
        : ($x === 5 ? 'five'
        : ($x === 6 ? 'six' : 'many')))));
}

function alternativeSyntaxOverLimit(array $items, int $level): string
{
    $out = '';
    if ($level > 3):
        $out .= 'high';
    elseif ($level > 2):
        $out .= 'mid';
    else:
        $out .= 'low';
    endif;
    foreach ($items as $item):
        $out .= $item;
    endforeach;
    while (strlen($out) > 10):
        $out = substr($out, 1);
    endwhile;
    for ($index = 0; $index < $level; $index++):
        $out .= '.';
    endfor;
    switch ($level):
        case 1:
            $out .= '!';
            break;
    endswitch;
    return $out;
}
