<?php
// Named by-reference arguments of PHP's builtins, `eval` reading a variable, and a by-reference
// capture as a variable's only use.

function named_by_reference_arguments(string $s): void
{
    preg_match(pattern: '/x/', subject: $s, matches: $named);
    echo $named[0];
    preg_match(subject: $s, matches: $reordered, pattern: '/x/');
    echo $reordered[0];
    preg_match('/x/', $s, matches: $mixed);
    echo $mixed[0];
}

function evaluated_read(): void
{
    $read = 1;
    eval('echo $read;');
}

function captured_by_reference(): callable
{
    $captured = 1;
    return function () use (&$captured) {
        return $captured;
    };
}
