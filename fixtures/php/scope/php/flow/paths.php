<?php
// Paths a run takes through branches, loops and switches, each run's inputs deciding every
// condition, so every path a flow reads is one some run takes.

function case_straight(): void
{
    $a = 1;
    $a = 2;
    $b = $a;
    $a .= 'x';
    $b++;
    unset($a);
    $a = 3;
}

function case_branches(bool $c, bool $d, bool $e): void
{
    $x = 1;
    if ($c) {
        $x = 2;
    } elseif ($d) {
        $x = 3;
    } else {
        $y = 0;
    }
    $x = 4;
    if ($e) {
        $y = 1;
    }
    $y = 2;
    if ($c) {
        return;
    }
    $z = 5;
}

function case_loops(array $items, bool $c): void
{
    $last = null;
    foreach ($items as $item) {
        if ($item === 2) {
            $skip = true;
            continue;
        }
        $last = $item;
        if ($c) {
            break;
        }
    }
    $last = 'done';
    $i = 0;
    while ($i < count($items)) {
        $i++;
    }
    $total = 0;
    foreach ($items as $key => $value) {
        $total += $value;
        $previous = $key;
    }
    do {
        $round = 1;
        $total--;
    } while ($total > 0);
}

function case_nested_loops(array $items, array $more): void
{
    $found = null;
    foreach ($items as $item) {
        foreach ($more as $other) {
            if ($other === 2) {
                continue 2;
            }
            if ($item === 2) {
                break 2;
            }
            $found = $other;
        }
        $found = $item;
    }
}

function case_switch(int $n): void
{
    $label = 'none';
    switch ($n) {
        case 0:
            $label = 'zero';
            break;
        case 1:
            $label = 'one';
        case 2:
            $label .= '!';
            break;
        default:
            $other = 'default';
    }
    $label = strtoupper($label);
}

function case_arrays(array $items): void
{
    $list = [];
    foreach ($items as $item) {
        $list[] = $item;
    }
    $list['count'] = count($list);
    [$first, $second] = [$list[0] ?? null, $list[1] ?? null];
    $matches = [];
    preg_match('/[0-9]/', implode('', $items), $matches);
    sort($list);
}

function case_closure(bool $c): void
{
    $base = 1;
    $add = function (int $x) use ($base): int {
        $sum = $base;
        $sum += $x;
        if ($x > 1) {
            return $sum;
        }
        $sum = 0;
        return $sum;
    };
    $add($c ? 1 : 2);
    $base = 2;
}

final class Counter
{
    public function count(array $items, bool $reset): int
    {
        $n = 0;
        foreach ($items as $item) {
            $n++;
        }
        if ($reset) {
            $n = 0;
        }
        return $n;
    }
}

function case_method(array $items, bool $reset): void
{
    $counter = new Counter();
    $counter->count($items, $reset);
}

$top = 1;
$top = 2;
$after = $top;

function case_unset_subscript(bool $c): void
{
    if ($c) {
        $a = ['k' => 1];
    }
    unset($a['k']);
    $a = 2;
}
