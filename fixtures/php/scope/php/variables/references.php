<?php

function by_reference_use(array $items): int
{
    $seen = [];
    $count = 0;
    $f = function (int $item) use (&$seen, &$count): bool {
        $count++;
        return in_array($item, $seen, true);
    };
    return count(array_filter($items, $f));
}

function reference_assignment(): int
{
    $value = 1;
    $alias = &$value;
    return $alias;
}

function arguments_read(int $first, int $second): array
{
    return func_get_args();
}

function argument_read(int $only): mixed
{
    return func_get_arg(0);
}

function evaluated(): void
{
    $code = 1;
    eval('echo $code;');
}

function object_offset_set(): void
{
    $object = new ArrayObject();
    $object['key'] = 1;
}

function array_offset_set(): void
{
    $array = [];
    $array['key'] = 1;
}

function coalesce_element(?array $input): void
{
    $copy = $input;
    $copy['key'] ??= 1;
}

function append_element(): void
{
    $copy = [];
    $copy['key'] .= 'x';
}

function self_updates(): int
{
    $text = 'a';
    $text .= 'b';
    $count = 0;
    $count++;
    $kept = 0;
    $kept++;
    return $kept;
}
