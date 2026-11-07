<?php
// PHP_CodeSniffer, as the port readiness gates run it, inside the pinned runtime of
// conformance/recorders/port-readiness/runtimes/phpcs-<version>, whose directory is the first argument.
//
// One JSON request on stdin, one JSON response on stdout:
// - {"op": "facts", "sniffs": [...], "standards": [...], "checkout": <dir>}: per sniff code, its class, its file in the
//   installed package, the strings its instantiated object holds, and its own unit test at the pinned checkout (the
//   test files, as upstream's AbstractSniffTestCase::getTestFiles lists them, and each file's expected errors and
//   warnings per line, from the test's getErrorList and getWarningList); per standard, the sniffs it turns on and the
//   properties and excluded messages it sets for each; the installed version and declared license.
// - {"op": "lint", "groups": [{"standard": <name or null>, "sniffs": [...]}], "files": [{"path", "test"?}]}: each
//   group's sniffs run over every file, as upstream's own test harness runs a sniff (tests/Standards/
//   AbstractSniffTestCase.php: a Config, a Ruleset of the sniffs, a LocalFile processed; `test` names the unit test
//   whose setCliValues applies to the file): a group with a standard runs the sniffs with that standard's
//   configuration of them (`--standard=<name> --sniffs=<codes>`), one without runs them alone with their default
//   properties, as the unit tests do. Every message, errors and warnings, with its line, column and source; a file
//   PHP_CodeSniffer could not analyse (UNANALYSED) is listed under `internal` with why.
//
// It reads only PHP_CodeSniffer's Config, Ruleset, LocalFile and the unit tests' expectation methods.

declare(strict_types=1);

namespace PHP_CodeSniffer\Tests\Standards {
    /**
     * Upstream's sniff test base class without PHPUnit, which the runtime does not install: what the gates call of a
     * unit test is its expectation lists, its test file list and its CLI values, and these defaults are upstream's own.
     */
    abstract class AbstractSniffTestCase
    {
        private array $ignoreExtensions = ['php' => 'php', 'fixed' => 'fixed', 'bak' => 'bak', 'orig' => 'orig'];

        protected function getTestFiles(string $testFileBase)
        {
            $testFiles = [];
            foreach (new \DirectoryIterator(dirname($testFileBase)) as $file) {
                $path = $file->getPathname();
                if (substr($path, 0, strlen($testFileBase)) === $testFileBase && isset($this->ignoreExtensions[$file->getExtension()]) === false) {
                    $testFiles[] = $path;
                }
            }
            sort($testFiles, SORT_NATURAL);
            return $testFiles;
        }

        protected function shouldSkipTest()
        {
            return false;
        }

        public function setCliValues(string $testFile, \PHP_CodeSniffer\Config $config)
        {
        }
    }
}

namespace {
    use PHP_CodeSniffer\Config;
    use PHP_CodeSniffer\Files\LocalFile;
    use PHP_CodeSniffer\Ruleset;

    require $argv[1] . '/vendor/autoload.php';
    require $argv[1] . '/vendor/squizlabs/php_codesniffer/autoload.php';

    // What PHP_CodeSniffer's Runner defines before it builds a Config: no fixer, no verbose output.
    define('PHP_CODESNIFFER_CBF', false);
    define('PHP_CODESNIFFER_VERBOSITY', 0);
    // The tokens PHP_CodeSniffer adds to PHP's (T_TRUE, T_OPEN_PARENTHESIS, …) are defined when its Tokens class loads,
    // which its Runner and Config do before any sniff is built.
    class_exists(PHP_CodeSniffer\Util\Tokens::class);

    /**
     * The internal messages after which PHP_CodeSniffer's answer on a file is not its whole answer: its tokenizer gave up
     * (File::parse ignores the file), a sniff threw (Runner), or the file could not be read (LocalFile). Its other
     * internal messages (mixed line endings, no PHP code found) are warnings on a file it analysed all the same.
     */
    const UNANALYSED = ['Internal.Tokenizer.Exception', 'Internal.Exception', 'Internal.LocalFile'];

    const PACKAGE = 'vendor/squizlabs/php_codesniffer';

    function package_directory(): string
    {
        global $argv;
        return $argv[1] . '/' . PACKAGE;
    }

    /** A Config of the given standard and sniffs, read from no CodeSniffer.conf and no phpcs.xml but the standard named. */
    function config(string $standard, array $sniffs): Config
    {
        $arguments = ['-q', '--no-cache', '--standard=' . $standard];
        if ($sniffs !== []) $arguments[] = '--sniffs=' . implode(',', $sniffs);
        return new Config($arguments, false);
    }

    /** A ruleset file of the sniffs alone, each with its default properties, as a unit test writes its own. */
    function alone(array $sniffs, string $directory): string
    {
        $file = tempnam($directory, 'ruleset') . '.xml';
        $rules = implode('', array_map(static fn (string $code): string => '    <rule ref="' . htmlspecialchars($code, ENT_XML1) . "\"/>\n", $sniffs));
        file_put_contents($file, "<?xml version=\"1.0\"?>\n<ruleset name=\"RerailPortReadiness\">\n{$rules}</ruleset>\n");
        return $file;
    }

    /** Every string an object or array holds, keys included, depth first: what a sniff's own state names. */
    function strings(mixed $value, array &$found, int $depth = 0): void
    {
        if ($depth > 8) return;
        if (is_string($value)) {
            $found[$value] = true;
        } elseif (is_array($value)) {
            foreach ($value as $key => $item) {
                if (is_string($key)) $found[$key] = true;
                strings($item, $found, $depth + 1);
            }
        } elseif (is_object($value)) {
            for ($class = new ReflectionObject($value); $class !== false; $class = $class->getParentClass()) {
                foreach ($class->getProperties() as $property) {
                    if ($property->isStatic() || !$property->isInitialized($value)) continue;
                    strings($property->getValue($value), $found, $depth + 1);
                }
            }
        }
    }

    /** The unit test class a test file of upstream's checkout belongs to, loaded, or null when the checkout has none. */
    function unit_test(string $testClassFile): ?object
    {
        if (!is_file($testClassFile)) return null;
        $before = get_declared_classes();
        require_once $testClassFile;
        $declared = array_values(array_filter(array_diff(get_declared_classes(), $before), static fn (string $class): bool => str_ends_with($class, 'UnitTest')));
        if ($declared === []) {
            $declared = array_values(array_filter(get_declared_classes(), static fn (string $class): bool => (new ReflectionClass($class))->getFileName() === realpath($testClassFile)));
        }
        return $declared === [] ? null : (new ReflectionClass($declared[0]))->newInstanceWithoutConstructor();
    }

    function call(object $object, string $method, mixed ...$arguments): mixed
    {
        $reflection = new ReflectionMethod($object, $method);
        return $reflection->invoke($object, ...$arguments);
    }

    function facts(array $request): array
    {
        $directory = package_directory();
        $sniffs = [];
        foreach ($request['sniffs'] as $code) {
            $parts = explode('.', $code);
            if (count($parts) !== 3) continue;
            [$standard, $category, $name] = $parts;
            $class = "PHP_CodeSniffer\\Standards\\{$standard}\\Sniffs\\{$category}\\{$name}Sniff";
            $file = "{$directory}/src/Standards/{$standard}/Sniffs/{$category}/{$name}Sniff.php";
            if (!is_file($file) || !class_exists($class)) continue;
            $found = [];
            $sniff = new $class();
            strings($sniff, $found);
            // A sniff upstream deprecates implements DeprecatedSniff, whose message PHP_CodeSniffer prints when it runs it.
            $deprecation = $sniff instanceof PHP_CodeSniffer\Sniffs\DeprecatedSniff
                ? trim("Deprecated since {$sniff->getDeprecationVersion()}; removed in {$sniff->getRemovalVersion()}. {$sniff->getDeprecationMessage()}")
                : null;
            $testClassFile = "{$request['checkout']}/src/Standards/{$standard}/Tests/{$category}/{$name}UnitTest.php";
            $test = unit_test($testClassFile);
            $tests = [];
            $skipped = false;
            if ($test !== null) {
                $skipped = call($test, 'shouldSkipTest') === true;
                foreach (call($test, 'getTestFiles', substr($testClassFile, 0, -3)) as $path) {
                    $base = basename($path);
                    $tests[] = [
                        'path' => $path,
                        'errors' => (object) call($test, 'getErrorList', $base),
                        'warnings' => (object) call($test, 'getWarningList', $base),
                    ];
                }
            }
            $names = array_keys($found);
            sort($names, SORT_STRING);
            $sniffs[$code] = ['class' => $class, 'file' => $file, 'deprecation' => $deprecation, 'testClass' => $test === null ? null : $testClassFile, 'skipped' => $skipped, 'tests' => $tests, 'names' => $names];
        }
        $standards = [];
        foreach ($request['standards'] as $standard) {
            $ruleset = new Ruleset(config($standard, []));
            $on = [];
            foreach (array_keys($sniffs) as $code) {
                if (!isset($ruleset->sniffCodes[$code])) continue;
                $properties = [];
                foreach ($ruleset->ruleset[$code]['properties'] ?? [] as $property => $setting) $properties[$property] = $setting['value'] ?? $setting;
                $excluded = [];
                foreach ($ruleset->ruleset as $ref => $settings) {
                    if (str_starts_with((string) $ref, "{$code}.") && ($settings['severity'] ?? null) === 0) $excluded[] = (string) $ref;
                }
                sort($excluded, SORT_STRING);
                $on[$code] = ['properties' => (object) $properties, 'excluded' => $excluded];
            }
            $standards[$standard] = (object) $on;
        }
        $package = json_decode(file_get_contents("{$directory}/composer.json"), true, 512, JSON_THROW_ON_ERROR);
        return [
            'version' => Config::VERSION,
            'license' => $package['license'] ?? null,
            'php' => PHP_VERSION,
            'iconv' => function_exists('iconv_strlen'),
            'package' => $directory,
            'sniffs' => (object) $sniffs,
            'standards' => (object) $standards,
        ];
    }

    function lint(array $request): array
    {
        $temporary = sys_get_temp_dir();
        $messages = [];
        $internal = [];
        foreach ($request['files'] as $file) $messages[$file['path']] = [];
        foreach ($request['groups'] as $group) {
            if ($group['sniffs'] === []) continue;
            $ruleset = $group['standard'] ?? alone($group['sniffs'], $temporary);
            try {
                foreach ($request['files'] as $file) {
                    // A fresh Config and Ruleset per file, as each unit test builds its own: a `phpcs:set` in one file
                    // changes the sniff's properties for the rest of the run.
                    $config = config($ruleset, $group['sniffs']);
                    $test = isset($file['test']) ? unit_test($file['test']) : null;
                    if ($test !== null) call($test, 'setCliValues', basename($file['path']), $config);
                    $sniffed = new LocalFile($file['path'], new Ruleset($config), $config);
                    try {
                        $sniffed->process();
                    } catch (Throwable $error) {
                        // PHP_CodeSniffer's Runner reports a sniff that throws as this message on the file, and goes on.
                        $internal[$file['path']] = 'Internal.Exception: ' . $error->getMessage();
                        continue;
                    }
                    foreach (['error' => $sniffed->getErrors(), 'warning' => $sniffed->getWarnings()] as $type => $found) {
                        foreach ($found as $line => $columns) {
                            foreach ($columns as $column => $list) {
                                foreach ($list as $message) {
                                    if (in_array($message['source'], UNANALYSED, true)) {
                                        $internal[$file['path']] = "{$message['source']}: {$message['message']}";
                                        continue;
                                    }
                                    $messages[$file['path']][] = ['source' => $message['source'], 'line' => $line, 'column' => $column, 'type' => $type];
                                }
                            }
                        }
                    }
                }
            } finally {
                if ($group['standard'] === null) {
                    unlink($ruleset);
                    @unlink(substr($ruleset, 0, -4));
                }
            }
        }
        return ['messages' => (object) $messages, 'internal' => (object) $internal];
    }

    $request = json_decode(stream_get_contents(STDIN), true, 512, JSON_THROW_ON_ERROR);
    $response = match ($request['op']) {
        'facts' => facts($request),
        'lint' => lint($request),
        default => throw new RuntimeException("unknown op {$request['op']}"),
    };
    echo json_encode($response, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_INVALID_UTF8_SUBSTITUTE), "\n";
}
