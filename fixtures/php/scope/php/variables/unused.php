<?php

function foreach_values(array $items): int
{
    $total = 0;
    foreach ($items as $key => $value) {
        $total += $key;
    }
    foreach ($items as $item) {
        $total++;
    }
    foreach ($items as $k => $v) {
        $total += $v;
    }
    foreach ($items as [$first, $second]) {
        $total += $first;
    }
    foreach ($items as &$reference) {
        $total++;
    }
    unset($reference);
    foreach ($items as $index => &$slot) {
        $total += $index;
    }
    foreach ($items as $unusedKey => ['a' => $a, 'b' => $b]) {
        $total += $b;
    }
    return $total;
}

function closure_parameters(): int
{
    $one = function ($ignored) {
        return 1;
    };
    $trailing = function ($used, $unusedTrailing) {
        return $used;
    };
    $leading = function ($leadingUnused, $used) {
        return $used;
    };
    $skipped = function ($beforeSkipped, $_skipped) {
        return 1;
    };
    $arrow = fn($x) => 1;
    $arrowUsed = fn($x) => $x;
    return $one(1) + $trailing(1, 2) + $leading(1, 2) + $skipped(1, 2) + $arrow(1) + $arrowUsed(1);
}

function trailing_parameter(int $used, int $trailingUnused): int
{
    return $used;
}

function leading_parameter(int $leadingUnused, int $used): int
{
    return $used;
}

function variadic_after(int $leadingUnused, int ...$rest): int
{
    return count($rest);
}

function ignored_names(int $used): int
{
    $_scratch = $used;
    $result = $used + 1;
    return $result;
}
