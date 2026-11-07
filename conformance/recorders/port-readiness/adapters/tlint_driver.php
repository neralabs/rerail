<?php
// tlint, as the port readiness gates run it, inside the pinned runtime of
// conformance/recorders/port-readiness/runtimes/tlint-<version>, whose directory is the first argument.
//
// One JSON request on stdin, one JSON response on stdout:
// - {"op": "facts", "lints": [...], "presets": [...], "checkout": <dir>}: per lint asked for and per Blade linter
//   (every linter class that uses LintsBladeTemplates), its file in the installed package, whether it is a Blade
//   linter, the strings its class constants and static properties hold and those that are PCRE patterns, and its own
//   test at the pinned checkout (tests/Linting/Linters/<Lint>Test.php): per test method, in declaration order, every
//   template it hands `new <Lint>(...)` and every assertion it makes, each checked against the real linter's output
//   exactly as PHPUnit checks it; per preset, the linters it turns on, in its order; the installed version and
//   declared license.
// - {"op": "lint", "cwd": <dir>, "groups": [{"preset": <name or null>, "lints": [...]}], "files": [...]}: each file
//   (relative to `cwd`) linted as `tlint lint <file>` lints it with the group's preset (LintCommand::lintFile: every
//   linter of the preset that applies to the path, in the preset's order, constructed with the file's contents and its
//   path), or, for a group without one, with its linters alone, as their tests run them; and the lints of the group's
//   linters among the answer, at their node's start line (`tlint lint --json`'s `line`). A file a linter cannot parse
//   (a PhpParser\Error, which tlint reports as the file's only lint) or throws on is listed under `internal` with why.
//
// It reads only tlint's Config, its presets, TLint::lint and the linter tests' templates and assertions.

declare(strict_types=1);

namespace PHPUnit\Framework {
    use LogicException;

    /**
     * PHPUnit's TestCase without PHPUnit, which the runtime does not install: the assertions tlint's linter tests make,
     * each recorded with whether it holds as PHPUnit decides it (assertEquals: ==; assertEmpty: empty()).
     */
    class TestCase
    {
        /** @var list<array{assertion: string, ok: bool}> */
        public static array $assertions = [];

        public function assertEquals(mixed $expected, mixed $actual): void
        {
            self::$assertions[] = ['assertion' => 'assertEquals(' . var_export($expected, true) . ', ' . var_export($actual, true) . ')', 'ok' => $expected == $actual];
        }

        public function assertEmpty(mixed $actual): void
        {
            self::$assertions[] = ['assertion' => 'assertEmpty(' . (is_countable($actual) ? count($actual) . ' item(s)' : var_export($actual, true)) . ')', 'ok' => empty($actual)];
        }

        public function __call(string $method, array $arguments): never
        {
            throw new LogicException("unsupported assertion {$method}()");
        }
    }
}

namespace {
    use Composer\InstalledVersions;
    use PHPUnit\Framework\TestCase;
    use Tighten\TLint\Config;
    use Tighten\TLint\Linters\Concerns\LintsBladeTemplates;
    use Tighten\TLint\TLint;

    require $argv[1] . '/vendor/autoload.php';

    const LINTERS = 'Tighten\\TLint\\Linters\\';

    /** The templates a running test hands the linter, captured as the linter receives them. */
    final class Captured
    {
        /** @var list<string> */
        public static array $templates = [];
    }

    function rerail_capture(mixed $template): mixed
    {
        if (!is_string($template)) throw new RuntimeException('a captured template is not a string');
        Captured::$templates[] = $template;
        return $template;
    }

    /**
     * $source with the first argument of every `new <name>(...)` wrapped in \rerail_capture(...). PHP's own tokenizer
     * delimits the argument (strings, heredocs and interpolations are single tokens or balanced), so it ends at the
     * first `,` or `)` outside brackets. Fails unless at least one call is rewritten.
     */
    function capture_templates(string $source, string $name): string
    {
        $tokens = PhpToken::tokenize($source);
        $significant = static function (int $index, int $step) use ($tokens): int {
            do {
                $index += $step;
            } while (isset($tokens[$index]) && $tokens[$index]->isIgnorable());
            return $index;
        };
        $insert = [];
        foreach ($tokens as $i => $token) {
            if (!$token->is([T_STRING, T_NAME_QUALIFIED, T_NAME_FULLY_QUALIFIED])) continue;
            $parts = explode('\\', $token->text);
            if (end($parts) !== $name || !($tokens[$significant($i, -1)] ?? null)?->is(T_NEW)) continue;
            $open = $significant($i, 1);
            if (($tokens[$open] ?? null)?->text !== '(') continue;
            $depth = 0;
            for ($end = $open + 1; isset($tokens[$end]); $end++) {
                $text = $tokens[$end]->text;
                if ($text === '(' || $text === '[' || $text === '{' || $tokens[$end]->is(T_DOLLAR_OPEN_CURLY_BRACES)) {
                    $depth++;
                } elseif ($text === ')' || $text === ']' || $text === '}') {
                    if ($depth === 0) break;
                    $depth--;
                } elseif ($text === ',' && $depth === 0) {
                    break;
                }
            }
            if ($significant($open, 1) >= $end) throw new RuntimeException("new {$name}() on line {$token->line} has no argument to capture");
            $insert[$open + 1] = '\\rerail_capture(';
            $insert[$end] = ')';
        }
        if ($insert === []) throw new RuntimeException("no new {$name}() to capture");
        $rewritten = '';
        foreach ($tokens as $i => $token) $rewritten .= ($insert[$i] ?? '') . $token->text;
        return $rewritten;
    }

    /**
     * A linter's own test, run with every template it hands the linter captured: per test method (`@test` in its doc
     * comment, a `test` prefix or PHPUnit's Test attribute), in declaration order, its templates and its assertions;
     * `error` is what the method threw, or null.
     */
    function linter_tests(string $testFile, string $lint): array
    {
        $before = get_declared_classes();
        $rewritten = tempnam(sys_get_temp_dir(), 'tlint-test');
        file_put_contents($rewritten, capture_templates(file_get_contents($testFile), $lint));
        try {
            require $rewritten;
        } finally {
            unlink($rewritten);
        }
        $classes = array_values(array_filter(array_diff(get_declared_classes(), $before), static fn (string $class): bool => is_subclass_of($class, TestCase::class)));
        if (count($classes) !== 1) throw new RuntimeException(basename($testFile) . ' declares ' . count($classes) . ' test classes');
        $class = $classes[0];
        $tests = [];
        foreach ((new ReflectionClass($class))->getMethods(ReflectionMethod::IS_PUBLIC) as $method) {
            $isTest = str_contains((string) $method->getDocComment(), '@test') || str_starts_with($method->name, 'test')
                || $method->getAttributes('PHPUnit\Framework\Attributes\Test') !== [];
            if ($method->class !== $class || !$isTest) continue;
            Captured::$templates = [];
            TestCase::$assertions = [];
            $error = null;
            try {
                $method->invoke(new $class());
            } catch (Throwable $thrown) {
                $error = get_class($thrown) . ': ' . $thrown->getMessage();
            }
            $tests[] = ['test' => $method->name, 'templates' => Captured::$templates, 'assertions' => TestCase::$assertions, 'error' => $error];
        }
        return $tests;
    }

    /** The strings a value holds, depth first. */
    function strings(mixed $value, array &$found): void
    {
        if (is_string($value)) {
            $found[$value] = true;
        } elseif (is_array($value)) {
            foreach ($value as $item) strings($item, $found);
        }
    }

    /** The linters a preset turns on, as tlint's Config reads `{"preset": <name>}` from tlint.json: short class names, in order. */
    function preset_linters(string $preset): array
    {
        return array_values(array_map(static fn (string $class): string => substr($class, strlen(LINTERS)), (new Config(['preset' => $preset]))->getLinters()));
    }

    function facts(array $request): array
    {
        $package = realpath(InstalledVersions::getInstallPath('tightenco/tlint'));
        $blade = [];
        foreach (glob("{$package}/src/Linters/*.php") as $file) {
            $lint = basename($file, '.php');
            if (class_exists(LINTERS . $lint) && in_array(LintsBladeTemplates::class, class_uses(LINTERS . $lint), true)) $blade[] = $lint;
        }
        $linters = [];
        foreach (array_unique([...$request['lints'], ...$blade]) as $lint) {
            $class = LINTERS . $lint;
            $file = "{$package}/src/Linters/{$lint}.php";
            if (!is_file($file) || !class_exists($class)) continue;
            $reflection = new ReflectionClass($class);
            $found = [];
            strings($reflection->getConstants(), $found);
            strings($reflection->getStaticProperties(), $found);
            $names = array_keys($found);
            sort($names, SORT_STRING);
            $patterns = [];
            foreach ($reflection->getStaticProperties() as $value) {
                if (is_string($value) && @preg_match($value, '') !== false) $patterns[] = $value;
            }
            $testFile = "{$request['checkout']}/tests/Linting/Linters/{$lint}Test.php";
            $linters[$lint] = [
                'file' => $file,
                'blade' => in_array($lint, $blade, true),
                'names' => $names,
                'patterns' => $patterns,
                'test' => is_file($testFile) ? $testFile : null,
                'tests' => is_file($testFile) ? linter_tests($testFile, $lint) : [],
            ];
        }
        $presets = [];
        foreach ($request['presets'] as $preset) $presets[$preset] = preset_linters($preset);
        $manifest = json_decode(file_get_contents("{$package}/composer.json"), true, 512, JSON_THROW_ON_ERROR);
        return [
            'version' => ltrim(InstalledVersions::getPrettyVersion('tightenco/tlint'), 'v'),
            'license' => $manifest['license'] ?? null,
            'php' => PHP_VERSION,
            'parser' => ltrim(InstalledVersions::getPrettyVersion('nikic/php-parser'), 'v'),
            'package' => $package,
            'linters' => (object) $linters,
            'presets' => (object) $presets,
        ];
    }

    function lint(array $request): array
    {
        chdir($request['cwd']);
        $tlint = new TLint();
        $lints = [];
        $internal = [];
        foreach ($request['files'] as $file) {
            $lints[$file] = [];
            foreach ($request['groups'] as $group) {
                $wanted = array_flip($group['lints']);
                $found = [];
                // A group without a preset runs its linters alone, constructed as their own tests construct them.
                if ($group['preset'] === null) {
                    $classes = array_map(static fn (string $lint): string => LINTERS . $lint, $group['lints']);
                } else {
                    $config = new Config(['preset' => $group['preset']]);
                    $classes = array_filter($config->getLinters(), static fn (string $class): bool => $class::appliesToPath($file, $config->paths));
                }
                foreach ($classes as $class) {
                    try {
                        foreach ($tlint->lint(new $class(file_get_contents($file), $file)) as $result) {
                            $lint = substr(get_class($result->getLinter()), strlen(LINTERS));
                            if (isset($wanted[$lint])) $found[] = ['lint' => $lint, 'line' => $result->getNode()->getStartLine()];
                        }
                    } catch (PhpParser\Error $error) {
                        // LintCommand::lintFile reports the parse error as the file's only lint and lints it no further.
                        $internal[$file] = substr($class, strlen(LINTERS)) . " cannot parse it: {$error->getRawMessage()} on line {$error->getStartLine()}";
                        unset($lints[$file]);
                        continue 3;
                    } catch (Throwable $error) {
                        $internal[$file] = substr($class, strlen(LINTERS)) . ' throws ' . get_class($error) . ": {$error->getMessage()}";
                        unset($lints[$file]);
                        continue 3;
                    }
                }
                array_push($lints[$file], ...$found);
            }
        }
        return ['lints' => (object) $lints, 'internal' => (object) $internal];
    }

    $request = json_decode(stream_get_contents(STDIN), true, 512, JSON_THROW_ON_ERROR);
    $response = match ($request['op']) {
        'facts' => facts($request),
        'lint' => lint($request),
        default => throw new RuntimeException("unknown op {$request['op']}"),
    };
    echo json_encode($response, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_INVALID_UTF8_SUBSTITUTE), "\n";
}
