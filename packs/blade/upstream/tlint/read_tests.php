<?php
// Runs one tlint 9.7.0 linter test (tests/Linting/Linters/<Lint>Test.php) with every template it hands the linter
// captured (../blade_capture.php: the argument of `new <Lint>(...)`), and
// prints, as JSON, per test method in declaration
// order: its templates and its assertions ({"equals": <expected>} or {"empty": true}). PHPUnit's TestCase is
// stubbed by the assertions these tests make; each is checked against the real linter's output exactly as PHPUnit
// checks it (assertEquals: ==), and a failed or unsupported assertion, or a test asserting nothing, fails the run.
// Called by generate.py:  php read_tests.php <composer project with tightenco/tlint 9.7.0> <Lint>Test.php <Lint>

declare(strict_types=1);

namespace PHPUnit\Framework {
    use LogicException;
    use RuntimeException;

    class TestCase
    {
        /** @var list<array<string, mixed>> */
        public static array $assertions = [];

        public function assertEquals(mixed $expected, mixed $actual): void
        {
            if ($expected != $actual) {
                throw new RuntimeException('assertEquals: expected '.var_export($expected, true).', got '.var_export($actual, true));
            }
            self::$assertions[] = ['equals' => $expected];
        }

        public function assertEmpty(mixed $actual): void
        {
            if (! empty($actual)) {
                throw new RuntimeException('assertEmpty: got '.count($actual).' lints');
            }
            self::$assertions[] = ['empty' => true];
        }

        public function __call(string $method, array $arguments): never
        {
            throw new LogicException("unsupported assertion {$method}()");
        }
    }
}

namespace {
    use PHPUnit\Framework\TestCase;

    require $argv[1].'/vendor/autoload.php';
    require __DIR__.'/../blade_capture.php';

    [, , $testFile, $lint] = $argv;
    $before = get_declared_classes();
    $rewritten = tempnam(sys_get_temp_dir(), 'tlint-test');
    file_put_contents($rewritten, rerail_rewrite(file_get_contents($testFile), T_NEW, $lint));
    try {
        require $rewritten;
    } finally {
        unlink($rewritten);
    }
    $classes = array_values(array_filter(array_diff(get_declared_classes(), $before),
        static fn (string $class): bool => is_subclass_of($class, TestCase::class)));
    if (count($classes) !== 1) {
        throw new RuntimeException("{$testFile} declares ".count($classes).' test classes');
    }
    $class = $classes[0];

    $tests = [];
    foreach ((new ReflectionClass($class))->getMethods(ReflectionMethod::IS_PUBLIC) as $method) {
        $isTest = str_contains((string) $method->getDocComment(), '@test') || str_starts_with($method->name, 'test')
            || $method->getAttributes('PHPUnit\Framework\Attributes\Test') !== [];
        if ($method->class !== $class || ! $isTest) {
            continue;
        }
        RerailCapture::$test = $method->name;
        RerailCapture::$templates = [];
        TestCase::$assertions = [];
        $method->invoke(new $class);
        if (TestCase::$assertions === []) {
            throw new RuntimeException("{$class}::{$method->name} asserts nothing");
        }
        $tests[] = [
            'test' => $method->name,
            'templates' => array_column(RerailCapture::$templates, 'template'),
            'assertions' => TestCase::$assertions,
        ];
    }
    echo json_encode($tests, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE), "\n";
}
