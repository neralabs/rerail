<?php
// A namespaced function another file's unqualified call falls back past.

namespace App\Util;

function shared(): int
{
    return 3;
}
