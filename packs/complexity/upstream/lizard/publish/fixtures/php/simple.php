<?php
// Only simple functions: nothing here is over the limit.

function add(int $a, int $b): int
{
    return $a + $b;
}

function describe(int $value): string
{
    return $value === 0 ? 'nothing' : (string) $value;
}

function first(array $items): int
{
    foreach ($items as $item) {
        return $item;
    }
    return 0;
}
