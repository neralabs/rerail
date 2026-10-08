<?php

function branches(bool $c, int $n): void
{
    if ($c) {
        $a = 1;
    }
    echo $a;
    if ($c) {
        $b = 1;
    } else {
        $b = 2;
    }
    echo $b;
    if ($c) {
        $d = 1;
    } elseif ($n > 1) {
        $d = 2;
    } else {
        return;
    }
    echo $d;
    if ($c) {
        $e = 1;
    } else {
        throw new \Exception();
    }
    echo $e;
    $c && ($f = 1);
    echo $f;
    $g = $c ? ($h = 1) : 2;
    echo $g, $h;
    $i = $n ?: ($j = 3);
    echo $i, $j;
    $k = null ?? ($l = 1);
    echo $k;
    switch ($n) {
        case 1:
            $s = 1;
            break;
        case 2:
            $s = 2;
            // fall through
        default:
            $s2 = 3;
    }
    echo $s, $s2;
    $m = match ($n) {
        1 => $mv = 1,
        default => 2,
    };
    echo $m, $mv;
    if ($c) {
        $x = 1;
        echo $x;
    }
    if ($n > 0 && ($y = $n)) {
        echo $y;
    }
    echo $y;
}

function exits(bool $c): void
{
    if (!$c) {
        exit(1);
    }
    $a = 1;
    echo $a;
    if ($c) {
        $b = 1;
    } else {
        die();
    }
    echo $b;
}
