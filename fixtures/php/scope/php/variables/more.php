<?php

namespace App\Services;

function nested_functions(int $a): void
{
    function inner(): void
    {
        echo $a;
    }
    echo $a;
}

function switch_and_continue(array $items): void
{
    foreach ($items as $item) {
        switch ($item) {
            case 1:
                $one = 1;
                continue 2;
            case 2:
                $two = 2;
                break;
            default:
                $three = 3;
        }
        echo $two;
        echo $one;
    }
}

function while_true(): int
{
    while (true) {
        $value = compute();
        if ($value > 1) {
            break;
        }
    }
    return $value;
}

function do_while(): void
{
    do {
        $x = compute();
    } while ($x < 1);
    echo $x;
}

function list_with_keys(array $rows): void
{
    foreach ($rows as ['id' => $id, 'name' => $name]) {
        echo $id, $name;
    }
    [, $second] = $rows;
    echo $second;
}

function arrows(int $n): int
{
    $factor = 2;
    $scale = fn(int $x) => $x * $factor;
    $nested = fn() => fn() => $n + $factor + $missing;
    return $scale($n) + $nested()();
}

function static_closure(): callable
{
    return static fn() => $this;
}

function return_in_branch(bool $c): int
{
    if ($c) {
        return 1;
    } else {
        $x = 2;
    }
    return $x;
}

function throw_in_branch(bool $c): int
{
    if (!$c) {
        throw new \InvalidArgumentException();
    }
    $x = 1;
    return $x;
}

function ternary_assignment(?int $n): int
{
    $x = $n ?? ($fallback = 0);
    return $x + $fallback;
}

function reassigned_in_loop(array $items): int
{
    $total = 0;
    foreach ($items as $item) {
        $total = $total + $item;
    }
    return $total;
}

function unset_in_branch(bool $c): void
{
    $x = 1;
    if ($c) {
        unset($x);
    }
    echo $x;
}

function compute(): int
{
    return 1;
}

final class Service
{
    private array $cache = [];

    public function get(string $key): mixed
    {
        if (!isset($this->cache[$key])) {
            $this->cache[$key] = compute();
        }
        return $this->cache[$key];
    }

    public static function make(): self
    {
        return new self();
    }
}
