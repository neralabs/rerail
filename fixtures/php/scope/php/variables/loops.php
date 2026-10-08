<?php

function loops(array $items, int $n): void
{
    while ($n > 0) {
        echo $w;
        $w = $n--;
    }
    echo $w;
    foreach ($items as $key => $value) {
        echo $key, $value;
    }
    echo $key, $value;
    foreach ([1, 2] as $one) {
    }
    echo $one;
    foreach ($items as [$p, $q]) {
        echo $p, $q;
    }
    foreach ($items as &$ref) {
        $ref = 1;
    }
    for ($i = 0; $i < $n; $i++) {
        $body = $i;
    }
    echo $i, $body;
    do {
        $once = 1;
    } while ($n--);
    echo $once;
    while (true) {
        $t = 1;
        break;
    }
    echo $t;
    foreach ($items as $item) {
        if ($item) {
            continue;
        }
        $after = $item;
    }
    echo $after;
    $sum = 0;
    foreach ($items as $v) {
        $sum += $v;
    }
    echo $sum;
    $last = null;
    foreach ($items as $v2) {
        $last = $v2;
    }
    echo $last;
}
