<?php
// Global functions; one of them declared only where it is not yet.

function helper(): int
{
    return 1;
}

if (!function_exists('guarded')) {
    function guarded(): int
    {
        return helper();
    }
}

function run_all(): void
{
    \App\Util\entry();
    \App\Model\entry();
    \App\Callables\entry();
    \App\Anon\entry();
    more_entry();
}
