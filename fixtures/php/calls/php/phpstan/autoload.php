<?php
// Loads the oracle's PHPStan extension classes (`phpstan analyse --autoload-file`): the
// container needs them before it starts.

require_once __DIR__ . '/CallTargetCollector.php';
require_once __DIR__ . '/CallTargetDump.php';
