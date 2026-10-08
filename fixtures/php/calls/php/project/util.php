<?php
// Function calls: declared here, the namespace fallback decided by the workspace, builtins,
// fully qualified, relative and aliased names.

namespace App\Util;

use function App\Model\make as build;

function local(): int
{
    return 2;
}

function entry(): void
{
    local();
    helper();
    shared();
    strlen('x');
    \strlen('y');
    \helper();
    \App\Util\local();
    namespace\local();
    build();
    guarded();
    LOCAL();
}
