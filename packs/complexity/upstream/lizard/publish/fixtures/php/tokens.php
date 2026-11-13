<?php
// Where lizard's PHP reader and Rerail differ on purpose (README, "Deviations"); each is a listed deviation.

// A nullable type's ? is no conditional: lizard counts each of the three (4), Rerail none (1).
function nullableTypes(?int $count, ?string $label): ?string
{
    return $label;
}

// The keyword operators and, or are && and || at a lower precedence: lizard counts neither (1), Rerail both (3).
function keywordLogic(bool $ready, bool $forced, bool $quiet): bool
{
    $go = $ready and $forced or $quiet;
    return $go;
}

// A method named match is no match expression: lizard counts the name (2), Rerail nothing (1).
function routes(object $router): object
{
    $router->match(['GET', 'HEAD'], '/', 'home');
    return $router;
}

// A closure inside a function is scored on its own: lizard adds its decisions to the function around it (8 and none),
// Rerail scores the function 2 and the closure 7.
function withClosure(array $items, bool $strict): array
{
    $keep = function (int $item) use ($strict): bool {
        if ($item > 100) {
            return false;
        }
        if ($item < 0) {
            return false;
        }
        if ($item % 2 === 0) {
            return true;
        }
        if ($strict && $item % 3 === 0) {
            return false;
        }
        return $item > 10 || $item === 7;
    };
    return $strict ? array_filter($items, $keep) : $items;
}
