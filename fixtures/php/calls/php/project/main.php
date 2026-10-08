<?php
// Runs every fixture's calls (the runtime trace's entry point).

require __DIR__ . '/global.php';
require __DIR__ . '/other.php';
require __DIR__ . '/util.php';
require __DIR__ . '/model.php';
require __DIR__ . '/callables.php';
require __DIR__ . '/anon.php';
require __DIR__ . '/more.php';

run_all();
