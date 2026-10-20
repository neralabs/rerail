<?php
// Bladestan, as the port readiness gates run it, inside the pinned runtime of
// conformance/recorders/port-readiness/runtimes/bladestan-<version>, whose directory is the first argument. The working
// directory is a scratch directory of the adapter's, and TMPDIR a directory in it: upstream's test application
// (Orchestra Testbench's skeleton, copied, and Bladestan's testbench.yaml) is set up there, and PHPStan's container and
// Bladestan's compiled templates are written there, so nothing is written into the runtime or the checkout.
//
// One JSON request on stdin, one JSON response on stdout:
// - {"op": "levels", "rules": [<PHPStan rule class>]}: per rule class, the lowest PHPStan level (0 to 10) whose
//   configuration (conf/config.level<N>.neon) registers it, or null.
// - {"op": "facts", "checkout": <dir>, "rules": [<PHPStan rule class>], "packages": [<Composer package>]}: each
//   package's installed version, PHPStan's own, the Bladestan package's directory and declared license, the phar's PHP
//   sources (src/…), each rule class's file and whether the template rules registry Bladestan runs over a compiled
//   template holds it; `tests`: every fixture of Bladestan's own rule test (tests/Rules/BladeRuleTest.php's
//   provideData), analysed as the test analyses it, with the test's expectation; `templates`: every template of the
//   test's application (tests/skeleton/resources), rendered alone with no view data (`view('<name>')`, from a caller
//   of its own under callers/), with view null for one no view name addresses.
// - {"op": "corpus", "checkout": <dir>, "root": <dir>, "stub": <file>, "templates": [<file under root>]}: each
//   template rendered alone, the root its only view location, and a view it cannot find the empty stub.
// An error is {identifier, message, line, template, templateLine}: `template` (absolute) and `templateLine` are
// Bladestan's metadata (template_file_path, template_line), null for an error Bladestan reports on the caller alone (a
// template that does not compile, a view it cannot find) or at a template it cannot place. A render that throws is
// {crash: <message>}.
//
// It runs Bladestan's own BladeRuleTest (PHPStan's RuleTestCase: level 8 with tests/Rules/config/configured_extension.neon)
// through RuleTestCase::gatherAnalyserErrors, as the test's analyse() does, without PHPUnit's runner.

declare(strict_types=1);

use Bladestan\Tests\Rules\BladeRuleTest;
use Bladestan\TemplateCompiler\PHPStan\FileAnalyserProvider;
use Composer\Autoload\ClassLoader;
use Illuminate\Contracts\View\Factory as ViewFactory;
use Illuminate\View\FileViewFinder;
use PHPStan\Analyser\Error;
use PHPStan\DependencyInjection\ContainerFactory;

define('RUNTIME_VENDOR', $argv[1] . '/vendor');
/** @var ClassLoader $loader */
$loader = require RUNTIME_VENDOR . '/autoload.php';
$request = json_decode(stream_get_contents(STDIN), true, 512, JSON_THROW_ON_ERROR);

/** The finder a corpus template is rendered with: its root the only view location, and a view it cannot find the empty stub. */
final class StubbingViewFinder extends FileViewFinder
{
    public function __construct(FileViewFinder $finder, string $root, private readonly string $stub)
    {
        parent::__construct($finder->getFilesystem(), [$root], $finder->getExtensions());
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

/** The classes, interfaces, traits and enums a PHP file declares, fully qualified, as Composer's classmap reads them. */
function declared_classes(string $path): array
{
    $tokens = PhpToken::tokenize((string) file_get_contents($path));
    $namespace = '';
    $classes = [];
    foreach ($tokens as $index => $token) {
        if ($token->is(T_NAMESPACE)) {
            $name = '';
            for ($next = $index + 1; isset($tokens[$next]) && ! in_array($tokens[$next]->text, [';', '{'], true); $next++) {
                if ($tokens[$next]->is([T_STRING, T_NAME_QUALIFIED])) {
                    $name .= $tokens[$next]->text;
                }
            }
            $namespace = $name === '' ? '' : $name . '\\';
        } elseif ($token->is([T_CLASS, T_INTERFACE, T_TRAIT, T_ENUM])) {
            $previous = $index - 1;
            while (isset($tokens[$previous]) && $tokens[$previous]->isIgnorable()) {
                $previous--;
            }
            // `Foo::class` and `new class` name no declaration.
            if (isset($tokens[$previous]) && $tokens[$previous]->is([T_DOUBLE_COLON, T_NEW])) {
                continue;
            }
            $next = $index + 1;
            while (isset($tokens[$next]) && $tokens[$next]->isIgnorable()) {
                $next++;
            }
            if (isset($tokens[$next]) && $tokens[$next]->is(T_STRING)) {
                $classes[] = $namespace . $tokens[$next]->text;
            }
        }
    }

    return $classes;
}

/** The checkout's autoload-dev (its tests, the test application's classes, the fixtures' classmap), as Composer installs it in development. */
function register_development_autoload(ClassLoader $loader, string $checkout): void
{
    $manifest = json_decode((string) file_get_contents($checkout . '/composer.json'), true, 512, JSON_THROW_ON_ERROR);
    $development = $manifest['autoload-dev'] ?? [];
    foreach ($development['psr-4'] ?? [] as $namespace => $directories) {
        $loader->addPsr4($namespace, array_map(static fn (string $directory): string => $checkout . '/' . $directory, (array) $directories));
    }
    $map = [];
    foreach ($development['classmap'] ?? [] as $directory) {
        $files = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($checkout . '/' . $directory, FilesystemIterator::SKIP_DOTS));
        foreach ($files as $file) {
            if ($file->isFile() && $file->getExtension() === 'php') {
                foreach (declared_classes($file->getPathname()) as $class) {
                    $map[$class] = $file->getPathname();
                }
            }
        }
    }
    $loader->addClassMap($map);
}

/** Upstream's test application in the working directory: Testbench's skeleton, copied, and Bladestan's testbench.yaml with that skeleton as its base path. */
function test_application(string $checkout): void
{
    $skeleton = realpath(RUNTIME_VENDOR . '/orchestra/testbench-core/laravel');
    $files = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($skeleton, FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::SELF_FIRST);
    foreach ($files as $file) {
        $target = 'laravel/' . substr($file->getPathname(), strlen($skeleton) + 1);
        if ($file->isDir()) {
            mkdir($target, 0777, true);
        } else {
            copy($file->getPathname(), $target);
        }
    }
    // Testbench links the skeleton's vendor/ to the working directory's, as a package's own checkout has it.
    symlink(RUNTIME_VENDOR, 'vendor');
    file_put_contents('testbench.yaml', "laravel: ./laravel\n" . file_get_contents($checkout . '/testbench.yaml'));
}

/** Bladestan's own rule test, set up as PHPUnit sets up each test. */
function rule_test(): BladeRuleTest
{
    $test = new BladeRuleTest('testRule');
    (fn () => $this->setUp())->call($test);

    return $test;
}

/**
 * The template Bladestan names, under the view location, namespace or base path it was named relative to (RecordTest.php's),
 * resolved; null for a template it cannot place (TemplateErrorsFactory names `''` for a compiled line before every mapped one).
 */
function absolute_template(string $template): ?string
{
    if (str_starts_with($template, '/')) {
        return realpath($template) ?: $template;
    }
    if ($template === '') {
        return null;
    }
    $finder = resolve(ViewFactory::class)->getFinder();
    $roots = array_merge($finder->getPaths(), ...array_values($finder->getHints()));
    $roots[] = base_path();
    $roots = array_unique(array_filter(array_map('realpath', $roots)));
    usort($roots, static fn (string $a, string $b): int => strlen($b) <=> strlen($a));
    foreach ($roots as $root) {
        if (is_file($root . '/' . $template)) {
            return realpath($root . '/' . $template);
        }
    }

    return null;
}

function error_record(Error $error): array
{
    $metadata = $error->getMetadata();
    $template = $metadata['template_file_path'] ?? null;

    return [
        'identifier' => $error->getIdentifier(),
        'message' => $error->getMessage(),
        'line' => $error->getLine(),
        'template' => $template === null ? null : absolute_template((string) $template),
        'templateLine' => $metadata['template_line'] ?? null,
    ];
}

/** Bladestan's errors rendering one view alone, with no view data, from a caller of its own (callers/c<n>.php). */
function render(BladeRuleTest $test, string $view, int $index): array
{
    is_dir('callers') || mkdir('callers');
    $caller = getcwd() . "/callers/c{$index}.php";
    file_put_contents($caller, "<?php\n\nview(" . var_export($view, true) . ");\n");
    try {
        return ['errors' => array_map('error_record', $test->gatherAnalyserErrors([$caller]))];
    } catch (Throwable $throwable) {
        return ['crash' => get_class($throwable) . ': ' . $throwable->getMessage()];
    }
}

/** The view name a template is rendered by: its path under a view location, dotted, or under a namespace's, `<namespace>::`. */
function view_name(string $template): ?string
{
    $finder = resolve(ViewFactory::class)->getFinder();
    $roots = array_map(static fn (string $path): array => [realpath($path), ''], $finder->getPaths());
    foreach ($finder->getHints() as $namespace => $paths) {
        foreach ($paths as $path) {
            $roots[] = [realpath($path), $namespace . '::'];
        }
    }
    foreach ($roots as [$root, $prefix]) {
        if ($root !== false && str_starts_with($template, $root . '/')) {
            $view = substr($template, strlen($root) + 1, -strlen('.blade.php'));
            // A view name's dots are its directories: a template whose name holds another cannot be addressed.
            return str_contains($view, '.') ? null : $prefix . str_replace('/', '.', $view);
        }
    }

    return null;
}

function levels(array $request): array
{
    $factory = new ContainerFactory(getcwd());
    $lowest = array_fill_keys($request['rules'], null);
    for ($level = 10; $level >= 0; $level--) {
        $container = $factory->create(getcwd() . '/tmp', [$factory->getConfigDirectory() . "/config.level{$level}.neon"], []);
        $registered = array_map('get_class', $container->getServicesByTag('phpstan.rules.rule'));
        foreach ($request['rules'] as $rule) {
            if (in_array($rule, $registered, true)) {
                $lowest[$rule] = $level;
            }
        }
    }

    return $lowest;
}

function facts(array $request): array
{
    $test = rule_test();
    $container = BladeRuleTest::getContainer();
    $registry = $container->getByType(FileAnalyserProvider::class)->getRules();
    $rules = [];
    foreach ($request['rules'] as $rule) {
        // A rule's node type is a constant of its class; the registry answers by node type.
        $node = (new ReflectionClass($rule))->newInstanceWithoutConstructor()->getNodeType();
        $held = array_filter($registry->getRules($node), static fn (object $instance): bool => $instance instanceof $rule);
        // The rule's source within the phar (src/Rules/…), as PHPStan's own classes load from it.
        $file = (string) (new ReflectionClass($rule))->getFileName();
        $rules[$rule] = ['file' => preg_replace('#^phar://.*?\.phar/#', '', $file), 'registered' => $held !== []];
    }
    $tests = [];
    foreach (BladeRuleTest::provideData() as [$file, $expected]) {
        $tests[] = [
            'file' => $file,
            'expected' => array_map(static fn (array $error): array => ['message' => $error[0], 'line' => $error[1]], $expected),
            'errors' => array_map('error_record', $test->gatherAnalyserErrors([$file])),
        ];
    }
    $templates = [];
    $resources = realpath($request['checkout'] . '/tests/skeleton/resources');
    $files = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($resources, FilesystemIterator::SKIP_DOTS));
    $paths = [];
    foreach ($files as $file) {
        if ($file->isFile() && str_ends_with($file->getFilename(), '.blade.php')) {
            $paths[] = $file->getPathname();
        }
    }
    sort($paths, SORT_STRING);
    foreach ($paths as $index => $path) {
        $view = view_name($path);
        $templates[] = ['path' => $path, 'view' => $view]
            + ($view === null ? [] : render($test, $view, $index));
    }
    $installed = json_decode((string) file_get_contents(RUNTIME_VENDOR . '/composer/installed.json'), true, 512, JSON_THROW_ON_ERROR)['packages'];
    $versions = array_column($installed, 'version', 'name');
    $package = realpath(RUNTIME_VENDOR . '/tomasvotruba/bladestan');
    $manifest = json_decode((string) file_get_contents($package . '/composer.json'), true, 512, JSON_THROW_ON_ERROR);
    $phar = realpath(RUNTIME_VENDOR . '/phpstan/phpstan/phpstan.phar');
    $sources = [];
    foreach (new RecursiveIteratorIterator(new Phar($phar)) as $file) {
        $inner = substr($file->getPathname(), strlen('phar://' . $phar . '/'));
        if (str_starts_with($inner, 'src/') && str_ends_with($inner, '.php')) {
            $sources[] = $inner;
        }
    }
    sort($sources, SORT_STRING);

    return [
        'versions' => array_combine(
            $request['packages'],
            array_map(static fn (string $name): ?string => isset($versions[$name]) ? ltrim($versions[$name], 'v') : null, $request['packages'])
        ),
        'phpstan' => PHPStan\Internal\ComposerHelper::getPhpStanVersion(),
        'php' => PHP_VERSION,
        'package' => $package,
        'phar' => $phar,
        'pharSources' => $sources,
        'license' => is_array($manifest['license'] ?? null) ? implode(' OR ', $manifest['license']) : ($manifest['license'] ?? null),
        'rules' => $rules,
        'tests' => $tests,
        'templates' => $templates,
    ];
}

function corpus(array $request): array
{
    $test = rule_test();
    $factory = resolve(ViewFactory::class);
    $factory->setFinder(new StubbingViewFinder($factory->getFinder(), $request['root'], $request['stub']));
    $templates = [];
    foreach ($request['templates'] as $index => $path) {
        $view = view_name($path);
        $templates[] = ['path' => $path, 'view' => $view]
            + ($view === null ? [] : render($test, $view, $index));
    }

    return ['templates' => $templates];
}

if ($request['op'] === 'levels') {
    $response = levels($request);
} else {
    register_development_autoload($loader, $request['checkout']);
    test_application($request['checkout']);
    $response = match ($request['op']) {
        'facts' => facts($request),
        'corpus' => corpus($request),
    };
}
echo json_encode($response, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_INVALID_UTF8_SUBSTITUTE), "\n";
