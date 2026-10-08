<?php

function globals_and_statics(): void
{
    global $config;
    echo $config;
    static $count = 0;
    $count++;
    echo $count;
    echo $_GET['q'], $_SERVER['x'], $GLOBALS['y'];
}

function by_reference_builtins(string $s, array $list): void
{
    preg_match('/x/', $s, $matches);
    echo $matches[0];
    parse_str($s, $parsed);
    echo $parsed['a'];
    sort($list);
    echo $list[0];
    array_push($stack, 1);
    echo $stack[0];
    echo strlen($undefinedArgument);
}

function try_catch(): void
{
    try {
        $t = risky();
    } catch (\RuntimeException $e) {
        echo $e->getMessage();
        echo $t;
    } catch (\LogicException) {
        echo 'no variable';
    } finally {
        echo 'done';
    }
    echo $t;
    try {
        $u = 1;
        risky();
    } catch (\Throwable $e2) {
        echo $u;
    }
    try {
        $v = risky();
    } finally {
        echo 'cleanup';
    }
    echo $v;
}

function compacting(string $s): array
{
    return compact('s', 'unknown');
}

function variable_variables(): void
{
    $name = 'dyn';
    $$name = 1;
    echo $dyn;
}

function references(): void
{
    $alias = &$source;
    echo $source;
    echo $alias;
}

function guards(): void
{
    if (isset($maybe)) {
        echo $maybe;
    }
    echo empty($empty) ? 1 : $empty;
    echo $nullable ?? 'default';
    echo @$suppressed;
    $defined = 1;
    if (isset($defined)) {
        echo $defined;
    }
    if (!isset($later)) {
        $later = 1;
    }
    echo $later;
}

function extracting(array $data): void
{
    extract($data);
    echo $fromExtract;
}

function evaluating(): void
{
    eval('$e = 1;');
    echo $e;
}

function including(): void
{
    include __DIR__ . '/x.php';
    echo $fromInclude;
}

function defined_vars(): array
{
    $kept = 1;
    return get_defined_vars();
}

function unknown_callee(): void
{
    someone_else($out);
    echo $out;
    $known = 1;
    someone_else($known);
    echo $known;
}

function risky(): int
{
    return 1;
}

$top = 1;
echo $top;
echo $argv[0];
echo $undefinedTop;
