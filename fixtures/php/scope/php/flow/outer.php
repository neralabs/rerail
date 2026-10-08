<?php
// Bindings that write, or do not write, the global variable of their name: through `global`,
// `$GLOBALS`, a closure's by-reference `use`, and what breaks that reference again.

$g = 0;
$h = 0;
$k = 0;
$m = 0;

function case_global(): void
{
    global $g;
    $g = 1;
    $g++;
    $local = 1;
}

function case_conditional(bool $c): void
{
    if ($c) {
        global $h;
    }
    $h = 2;
}

function case_rebind(): void
{
    global $k;
    $other = 0;
    $k = &$other;
    $k = 3;
}

function case_globals_array(): void
{
    $m = &$GLOBALS['m'];
    $m = 5;
    $n = &$GLOBALS['m'];
    $n = 6;
}

function case_static(): void
{
    static $s = 0;
    $s++;
}

function case_catch(bool $c): void
{
    global $e;
    try {
        if ($c) {
            throw new RuntimeException('caught');
        }
    } catch (RuntimeException $e) {
        $seen = 1;
    }
}

function case_foreach(array $items, array $more): void
{
    global $v, $w;
    foreach ($items as $v) {
        $first = $v;
    }
    foreach ($more as &$w) {
        $second = $w;
    }
}

function case_list(): void
{
    global $l;
    [$l] = [1];
    $l .= 'x';
}

function case_builtin(): void
{
    global $out;
    preg_match('/a/', 'a', $out);
}

function case_unset(): void
{
    global $u;
    $u = 1;
    unset($u);
    $u = 2;
}

function case_parameter(int $g): void
{
    $g = 2;
}

$byReference = function () use (&$shared): void {
    $shared = 1;
    $own = 2;
};
$byReference();
$byValue = function () use ($k): void {
    $k = 9;
};
$byValue();
