<?php
// Prints, as JSON, what a PHP_CodeSniffer 4.0.4 sniff test expects of one of its test files:
// {"errors": {line: count}, "warnings": {line: count}}, from the test class's getErrorList() and
// getWarningList() (which receive the test file's name and may depend on the running PHP).
// Called by generate.py:  php error_lists.php <SomeUnitTest.php> <SomeUnitTest.inc>

declare(strict_types=1);

namespace PHP_CodeSniffer\Tests\Standards {
    abstract class AbstractSniffTestCase
    {
    }
}

namespace {
    require $argv[1];
    $declared = array_filter(get_declared_classes(), static fn (string $class): bool => str_ends_with($class, 'UnitTest'));
    $test = (new ReflectionClass(end($declared)))->newInstanceWithoutConstructor();
    echo json_encode([
        'errors' => (object) $test->getErrorList($argv[2]),
        'warnings' => (object) $test->getWarningList($argv[2]),
    ]), "\n";
}
