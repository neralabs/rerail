<?php
// Runs one stillat/blade-parser 2.1.0 validator test (tests/Validation/<X>Test.php, a Pest file) with every template
// it hands Document::fromText() captured (../blade_capture.php), and prints, as JSON, per test and dataset row in Pest's
// order: the templates and the expectations it made ({"count": n} for toHaveCount, {"be": value} for toBe). Pest is
// stubbed by what these tests use (uses(), test()->with(), expect()->toHaveCount()/toBe(), and expect()->extend()
// in tests/Pest.php, loaded first for its dataset functions); every expectation is checked against the validator's
// real output as Pest checks it (toHaveCount: count() ===, toBe: ===), and a failed or unsupported expectation, or a
// test row expecting nothing, fails the run. The tests run on a bare container (no Laravel application), so the
// parser knows the core directives and no custom one, and validators are constructed with their defaults.
// Called by the Blade pack's generator (packs/blade/upstream/blade-parser/generate.py):
//   php read_tests.php <composer project with stillat/blade-parser 2.1.0> <checkout> <X>Test.php

declare(strict_types=1);

require $argv[1].'/vendor/autoload.php';
require __DIR__.'/../blade_capture.php';

final class RerailExpectation
{
    /** @var list<array<string, mixed>> */
    public static array $log = [];

    public function __construct(private readonly mixed $value) {}

    public function toHaveCount(int $count): self
    {
        if (count($this->value) !== $count) {
            throw new RuntimeException("toHaveCount: expected {$count}, got ".count($this->value));
        }
        self::$log[] = ['count' => $count];

        return $this;
    }

    public function toBe(mixed $expected): self
    {
        if ($this->value !== $expected) {
            throw new RuntimeException('toBe: expected '.var_export($expected, true).', got '.var_export($this->value, true));
        }
        self::$log[] = ['be' => $expected];

        return $this;
    }

    // tests/Pest.php: expect()->extend('toBeOne', ...), an expectation these tests do not use.
    public function extend(string $name, Closure $extension): void {}

    public function __call(string $method, array $arguments): never
    {
        throw new LogicException("unsupported expectation {$method}()");
    }
}

final class RerailTest
{
    /** @var list<RerailTest> */
    public static array $tests = [];

    public ?array $dataset = null;

    public function __construct(public readonly string $name, public readonly Closure $body)
    {
        self::$tests[] = $this;
    }

    public function with(array $dataset): self
    {
        $this->dataset = $dataset;

        return $this;
    }

    public function __call(string $method, array $arguments): never
    {
        throw new LogicException("unsupported test modifier {$method}()");
    }
}

function uses(string ...$classes): void {}

function test(string $description, Closure $body): RerailTest
{
    return new RerailTest($description, $body);
}

function it(string $description, Closure $body): RerailTest
{
    return new RerailTest("it {$description}", $body);
}

function expect(mixed $value = null): RerailExpectation
{
    return new RerailExpectation($value);
}

[, , $checkout, $testFile] = $argv;
require $checkout.'/tests/Pest.php';
$rewritten = tempnam(sys_get_temp_dir(), 'blade-parser-test');
file_put_contents($rewritten, rerail_rewrite(file_get_contents($testFile), T_DOUBLE_COLON, 'fromText'));
try {
    require $rewritten;
} finally {
    unlink($rewritten);
}

$runs = [];
foreach (RerailTest::$tests as $test) {
    $rows = $test->dataset ?? [[]];
    foreach ($rows as $key => $row) {
        $arguments = is_array($row) ? $row : [$row];
        $label = $test->dataset === null ? null
            : (is_string($key) ? $key : '('.implode(', ', array_map(static fn ($v) => var_export($v, true), $arguments)).')');
        RerailCapture::$test = $test->name;
        RerailCapture::$dataset = $label;
        RerailCapture::$templates = [];
        RerailExpectation::$log = [];
        ($test->body)(...$arguments);
        if (RerailExpectation::$log === []) {
            throw new RuntimeException("{$test->name} {$label} expects nothing");
        }
        $runs[] = [
            'test' => $test->name,
            'dataset' => $label,
            'templates' => array_column(RerailCapture::$templates, 'template'),
            'expectations' => RerailExpectation::$log,
        ];
    }
}
echo json_encode($runs, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE), "\n";
