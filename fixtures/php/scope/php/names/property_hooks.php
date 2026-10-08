<?php
// A short property hook whose body is a bare name: the body is a constant fetch, the hook's own
// name (`get`, `set`) is no reference.
namespace App\Hooks;

const LIMIT = 3;

final class Hooks
{
    public bool $enabled { get => true; }
    public ?string $label { get => null; }
    public bool $off { get => \false; }
    public int $limit { get => LIMIT; }
    public int $max { get => PHP_INT_MAX; }
    public string $elsewhere { get => DEFINED_ELSEWHERE; }
    public string $qualified { get => Config\DEFAULT_LABEL; }
    public bool $flag { set => true; }
    public int $size {
        get => \count($this->items);
        set(int $value) => max($value, LIMIT);
    }
    public array $items = [];
}
