<?php

declare(strict_types=1);

const GLOBAL_TWO = '%s %s';
define('DEFINED_TWO', '%s %s');
define(constant_name: 'NAMED_ONE', value: '%s');
define('DEFINED_FROM', GLOBAL_TWO . ' %s');

if (!defined('MAYBE')) {
    define('MAYBE', '%s %s');
}

function defines(int $a, int $b): void
{
    echo sprintf(GLOBAL_TWO, $a);
    echo sprintf(\GLOBAL_TWO, $a, $b, $a);
    echo sprintf(DEFINED_TWO, $a);
    echo sprintf(NAMED_ONE, $a, $b);
    echo sprintf(DEFINED_FROM, $a, $b);
    echo sprintf(MAYBE, $a);
}

$a = 1;
echo sprintf(GLOBAL_TWO, $a);
