<?php

declare(strict_types=1);

// Bladestan's own rule test harness (tests/Rules/BladeRuleTest.php at 0.11.7: PHPStan's RuleTestCase, level 8, with
// tests/Rules/config/configured_extension.neon), run by generate.py from a Bladestan checkout:
//
//   BLADESTAN_RECORD=<request.json> vendor/bin/phpunit --no-configuration --bootstrap vendor/autoload.php <this file>
//
// The request is `{ "mode": "fixtures" }` or
// `{ "mode": "corpus", "root": <directory>, "stub": <empty template>, "callers": [<file>] }`. `fixtures` analyses
// each fixture of BladeRuleTest::provideData() alone, requires its errors (message, caller line) to be the test's
// expectation, and records them. `corpus` adds the corpus root as a view location and analyses each caller (each
// one `view('<template path, dotted>')`) in turn; one whose analysis throws is recorded as a `rerail.crash` error. A
// view the corpus's application or packages would register (an `@include`, an `<x-...>` component) is the empty
// stub: Bladestan does not compile a template whose component it cannot locate at all, and the stub's content is
// outside every recorded template. Both write
// `{ "errors": [{ caller, line, identifier, message, template, templateLine }] }` to `<request>.out.json`; `template`
// (made absolute: Bladestan names it relative to the view location or namespace holding it) and `templateLine` are
// Bladestan's metadata (`template_file_path`, `template_line`), absent for an error Bladestan reports on the caller
// alone (a template that does not compile, a view it cannot find).

use Bladestan\Rules\BladeRule;
use Bladestan\Tests\Rules\BladeRuleTest;
use Illuminate\Contracts\View\Factory as ViewFactory;
use Illuminate\View\FileViewFinder;
use PHPStan\Analyser\Error;
use PHPStan\Rules\Rule;
use PHPStan\Testing\RuleTestCase;

/**
 * @extends RuleTestCase<Rule>
 */
final class RecordTest extends RuleTestCase
{
    public function testRecord(): void
    {
        $requestPath = (string) getenv('BLADESTAN_RECORD');
        $request = json_decode((string) file_get_contents($requestPath), true, flags: JSON_THROW_ON_ERROR);
        $errors = [];
        if ($request['mode'] === 'fixtures') {
            foreach (BladeRuleTest::provideData() as [$file, $expected]) {
                $gathered = $this->gatherAnalyserErrors([$file]);
                $actual = array_map(static fn (Error $error): string => $error->getLine() . ': ' . $error->getMessage(), $gathered);
                $wanted = array_map(static fn (array $error): string => $error[1] . ': ' . $error[0], $expected);
                sort($actual);
                sort($wanted);
                $this->assertSame($wanted, $actual, "Bladestan's expectation for " . basename($file));
                array_push($errors, ...array_map($this->record(...), $gathered));
            }
        } else {
            $factory = resolve(ViewFactory::class);
            $finder = new StubbingViewFinder($factory->getFinder(), $request['stub']);
            $finder->addLocation($request['root']);
            $factory->setFinder($finder);
            foreach ($request['callers'] as $caller) {
                try {
                    array_push($errors, ...array_map($this->record(...), $this->gatherAnalyserErrors([$caller])));
                } catch (Throwable $throwable) {
                    // Bladestan does not analyse this template: the caller's render throws.
                    $errors[] = ['caller' => $caller, 'line' => null, 'identifier' => 'rerail.crash',
                        'message' => get_class($throwable) . ': ' . $throwable->getMessage(), 'template' => null,
                        'templateLine' => null];
                }
            }
        }
        file_put_contents(
            $requestPath . '.out.json',
            json_encode(['errors' => $errors], JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR)
        );
    }

    /** @return array<string, int|string|null> */
    private function record(Error $error): array
    {
        $metadata = $error->getMetadata();
        $template = $metadata['template_file_path'] ?? null;

        return [
            'caller' => $error->getFilePath(),
            'line' => $error->getLine(),
            'identifier' => $error->getIdentifier(),
            'message' => $error->getMessage(),
            'template' => $template === null ? null : self::absolute($template),
            'templateLine' => $metadata['template_line'] ?? null,
        ];
    }

    /** The template Bladestan names, under the view location, namespace or base path it was named relative to. */
    private static function absolute(string $template): string
    {
        if (str_starts_with($template, '/')) {
            return $template;
        }
        $finder = resolve(ViewFactory::class)->getFinder();
        $roots = array_merge($finder->getPaths(), ...array_values($finder->getHints()));
        $roots[] = base_path();
        $roots = array_unique(array_filter(array_map('realpath', $roots)));
        usort($roots, static fn (string $a, string $b): int => strlen($b) <=> strlen($a));
        foreach ($roots as $root) {
            if (is_file($root . '/' . $template)) {
                return $root . '/' . $template;
            }
        }
        throw new RuntimeException("cannot place template {$template}");
    }

    /** @return list<string> */
    public static function getAdditionalConfigFiles(): array
    {
        return [getcwd() . '/tests/Rules/config/configured_extension.neon'];
    }

    protected function getRule(): Rule
    {
        return self::getContainer()->getByType(BladeRule::class);
    }
}

/** The finder Bladestan resolves views with, where a view it cannot find is the empty stub. */
final class StubbingViewFinder extends FileViewFinder
{
    public function __construct(FileViewFinder $finder, private readonly string $stub)
    {
        parent::__construct($finder->getFilesystem(), $finder->getPaths(), $finder->getExtensions());
        foreach ($finder->getHints() as $namespace => $hints) {
            $this->addNamespace($namespace, $hints);
        }
    }

    public function find($name)
    {
        try {
            return parent::find($name);
        } catch (InvalidArgumentException) {
            return $this->stub;
        }
    }
}
