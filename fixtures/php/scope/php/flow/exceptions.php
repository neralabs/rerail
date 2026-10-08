<?php
// Paths through `try`, `catch` and `finally`. A flow lets every statement of a `try` body throw;
// a run throws where the fixture does, so the flow's paths here hold the runs' and more.

function case_catch(bool $c): void
{
    $state = 'start';
    try {
        $state = 'try';
        if ($c) {
            throw new RuntimeException('x');
        }
        $state = 'done';
    } catch (RuntimeException $error) {
        $state = 'caught';
    }
    $state .= '!';
}

function case_finally(bool $c, bool $d): void
{
    $step = 0;
    try {
        $step = 1;
        if ($c) {
            throw new LogicException('x');
        }
        $step = 2;
    } catch (LogicException $error) {
        if ($d) {
            throw $error;
        }
        $step = 3;
    } finally {
        $cleanup = $step;
    }
    $step = 4;
}

function case_break_finally(bool $c): void
{
    $x = 1;
    while (true) {
        try {
            if ($c) {
                break;
            }
            $x = 2;
            break;
        } finally {
            $x = 3;
        }
    }
    $x = 4;
}

function case_continue_finally(array $items): void
{
    foreach ($items as $item) {
        try {
            continue;
        } finally {
            $last = $item;
        }
    }
    $last = 0;
}

function case_return_finally(bool $c): void
{
    $r = 1;
    try {
        if ($c) {
            return;
        }
        $r = 2;
    } finally {
        $r = 3;
    }
    $r = 4;
}
