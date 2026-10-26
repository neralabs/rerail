<?php
// How the port readiness gates read Blade templates, run on the pinned interpreter of
// conformance/recorders/port-readiness/runtimes/blade with that runtime's vendor/ (laravel/framework's BladeCompiler and
// stillat/blade-parser), whose directory is the first argument.
//
// One JSON request on stdin, one JSON response on stdout:
// - {"op": "facts"}: the directives BladeCompiler compiles, and the installed versions of laravel/framework and
//   stillat/blade-parser.
// - {"op": "verdicts", "files": [{"path", "source"}]}: per file, the PHP BladeCompiler compiles it to (null when it
//   throws, with what it threw), and stillat/blade-parser's verdict (syntax only). blade.ts lints the compiled PHP with
//   the PHP toolkit's `php -l`.

declare(strict_types=1);

require $argv[1] . '/vendor/autoload.php';

use Composer\InstalledVersions;
use Illuminate\Container\Container;
use Illuminate\Contracts\Foundation\Application;
use Illuminate\Contracts\View\Factory as FactoryContract;
use Illuminate\Events\Dispatcher;
use Illuminate\Filesystem\Filesystem;
use Illuminate\View\Compilers\BladeCompiler;
use Illuminate\View\Engines\EngineResolver;
use Illuminate\View\Factory;
use Illuminate\View\ViewFinderInterface;
use Stillat\BladeParser\Document\Document;

/**
 * The passes compileString runs over the whole template itself rather than through compileStatement, which reads every
 * other `compile<Name>` method as the directive `@<name>`. The passes it runs over each stretch of inline HTML
 * (`$compilers`) and the echo compilers (getEchoMethods) are read off the compiler.
 */
const PASSES = ['', 'String', 'Statement', 'Comments', 'ComponentTags'];

/**
 * A container where every component a tag names exists, as an anonymous component (a view under `components.`): the gates
 * judge a template's syntax, not the application it renders in, and ComponentTagCompiler asks the container for the
 * application's namespace and views to resolve a tag. A tag then compiles as Laravel compiles any anonymous component,
 * its bound attributes into PHP that `php -l` reads.
 */
function application(): void
{
    $container = new Container();
    Container::setInstance($container);
    $views = new class implements ViewFinderInterface {
        public function find($view)
        {
            return $view;
        }

        public function addLocation($location) {}

        public function addNamespace($namespace, $hints) {}

        public function prependNamespace($namespace, $hints) {}

        public function replaceNamespace($namespace, $hints) {}

        public function addExtension($extension) {}

        public function flush() {}
    };
    $container->instance(FactoryContract::class, new Factory(new EngineResolver(), $views, new Dispatcher($container)));
    // ComponentTagCompiler::guessClassName only reads the namespace; no class under it exists, so no tag resolves to a class.
    $container->instance(Application::class, new class {
        public function getNamespace(): string
        {
            return 'App\\';
        }
    });
}

function compiler(): BladeCompiler
{
    return new BladeCompiler(new Filesystem(), sys_get_temp_dir());
}

/**
 * The directives BladeCompiler compiles, as templates spell them: every `compile<Name>` method compileStatement calls for
 * `@<name>` (PHP method names are case-insensitive, so `@includeIf` and `@includeif` both compile, and both spellings are
 * listed), and the blocks storeUncompiledBlocks lifts out before any statement compiles (`@verbatim … @endverbatim`,
 * `@php … @endphp`), read from the patterns of its `store…Blocks` methods.
 */
function directives(): array
{
    $compiler = compiler();
    $class = new ReflectionClass($compiler);
    $passes = [
        ...PASSES,
        ...$class->getProperty('compilers')->getValue($compiler),
        ...array_map(static fn (string $method): string => substr($method, strlen('compile')), $class->getMethod('getEchoMethods')->invoke($compiler)),
    ];
    $names = [];
    foreach ($class->getMethods() as $method) {
        if ($method->isStatic() || !str_starts_with($method->name, 'compile')) continue;
        $name = substr($method->name, strlen('compile'));
        if (!in_array($name, $passes, true)) $names[] = $name;
    }
    $source = file($class->getFileName());
    foreach ($class->getMethods() as $method) {
        if (!preg_match('/^store\w*Blocks$/', $method->name) || $method->getFileName() !== $class->getFileName()) continue;
        $body = implode('', array_slice($source, $method->getStartLine() - 1, $method->getEndLine() - $method->getStartLine() + 1));
        preg_match_all('/@(\w+)/', $body, $matches);
        array_push($names, ...$matches[1]);
    }
    $found = [];
    foreach ($names as $name) {
        $found[lcfirst($name)] = true;
        $found[strtolower($name)] = true;
    }
    $words = array_keys($found);
    sort($words, SORT_STRING);
    return $words;
}

function first_line(Throwable $error): string
{
    $message = trim(strtok($error->getMessage(), "\n") ?: '');
    return (new ReflectionClass($error))->getShortName() . ($message === '' ? '' : ": {$message}");
}

/** BladeCompiler::compileString: the compiled PHP, or what the compiler threw (a template it cannot compile). */
function compiled(string $source): array
{
    try {
        return ['compiled' => compiler()->compileString($source), 'errors' => []];
    } catch (Throwable $error) {
        return ['compiled' => null, 'errors' => [first_line($error)]];
    }
}

/**
 * stillat/blade-parser's document parser: its fatal errors, those it says break compiling the template (an unclosed
 * `<?php` or `@php` at the end of the template is a warning), are syntax errors. It checks nothing after it builds the
 * document, so grammar is null.
 */
function blade_parser_verdict(string $source): array
{
    try {
        $errors = Document::fromText($source)->getErrors()->filter->isFatal()->map->getErrorMessage()->values()->all();
        return ['syntax' => $errors, 'grammar' => null];
    } catch (Throwable $error) {
        return ['syntax' => [first_line($error)], 'grammar' => null];
    }
}

function version(string $package): string
{
    return ltrim(InstalledVersions::getPrettyVersion($package) ?? 'unknown', 'v');
}

application();
$request = json_decode(stream_get_contents(STDIN), true, 512, JSON_THROW_ON_ERROR);
switch ($request['op']) {
    case 'facts':
        $response = ['compiler' => version('laravel/framework'), 'parser' => version('stillat/blade-parser'), 'directives' => directives()];
        break;
    case 'verdicts':
        $parser = 'stillat/blade-parser ' . version('stillat/blade-parser');
        $response = array_map(static fn (array $file): array => [
            ...compiled($file['source']),
            'parser' => ['parser' => $parser, ...blade_parser_verdict($file['source'])],
        ], $request['files']);
        break;
    default:
        fwrite(STDERR, "unknown op {$request['op']}\n");
        exit(1);
}
echo json_encode($response, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_INVALID_UTF8_SUBSTITUTE), "\n";
