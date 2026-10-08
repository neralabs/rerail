<?php
// `define` called by another name: an alias of the global `define` declares, an alias of another
// function named `define` does not.

namespace App;

use function define as def;
use function Other\define;

def('App\ALIASED', 1);
DEF('App\ALIASED_UPPER', 2);
define('App\SHADOWED', 3);

echo ALIASED, ALIASED_UPPER, SHADOWED;
