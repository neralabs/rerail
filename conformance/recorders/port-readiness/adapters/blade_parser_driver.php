<?php
// stillat/blade-parser's validators, as the port readiness gates run them, inside the pinned runtime of
// conformance/recorders/port-readiness/runtimes/blade (laravel/framework and stillat/blade-parser), whose directory is
// the first argument.
//
// One JSON request on stdin, one JSON response on stdout:
// - {"op": "facts", "validators": [...], "checkout": <dir>}: per validator (a class name under
//   Stillat\BladeParser\Validation\Validators), its file in the installed package, the strings its instantiated object
//   holds (DebugDirectiveValidator's debug directive names, read from the parser at construction), the deprecation its
//   docblock declares, its own test at the pinned checkout (tests/Validation/<name less Validator>Test.php), and the
//   options the package's default configuration loads into it when config/validation.php's core_validators lists it,
//   as ValidatorServiceProvider::register builds them (null when the list does not); the installed version and
//   declared license.
// - {"op": "lint", "validators": [{"name", "options"}], "files": [{"path", "source"}]}: per file and validator,
//   Document::fromText(<source>)->addValidator(<validator>)->validate()->getValidationErrors(), each validator on a
//   document of its own, as its test runs it; `options` null constructs the validator as the tests do (its defaults),
//   an object loads those options into it, as the service provider does. Every validation error, with the start line
//   and column of the node it reports (its BladeError position, plus its shift line) and its message; a file the
//   parser or a validator throws on is listed under `errors` with what it threw.
//
// The parser runs on a bare container (no Laravel application booted), as upstream/blade-parser/run.php runs it: it
// knows Blade's core directives and no custom one.

declare(strict_types=1);

require $argv[1] . '/vendor/autoload.php';

use Composer\InstalledVersions;
use Stillat\BladeParser\Document\Document;
use Stillat\BladeParser\Validation\AbstractNodeValidator;
use Stillat\BladeParser\Validation\Validators\Concerns\AcceptsCustomDirectives;
use Stillat\BladeParser\Validation\Validators\Concerns\CanIgnoreDirectives;

const PACKAGE = 'stillat/blade-parser';
const VALIDATORS = 'Stillat\\BladeParser\\Validation\\Validators\\';
/** The directive ValidatorServiceProvider::getIgnoreDirectives adds to the configured global ignore list. */
const PROVIDER_IGNORED = ['livewire'];

function package_directory(): string
{
    return realpath(InstalledVersions::getInstallPath(PACKAGE));
}

/** Every string an object or array holds, keys included, depth first: what a validator's own state names. */
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

/**
 * The options the package's default configuration loads into each validator config/validation.php's core_validators
 * lists, by class, as ValidatorServiceProvider::register builds them: its `options` entry, with the global
 * ignore_directives (and `livewire`, which the provider always adds) merged first into a validator that can ignore
 * directives, and the global custom_directives into one that accepts custom directives. The provider drops
 * DuplicateConditionExpressionsValidator and InvalidPhpDocumentValidator from the list only when the application has
 * PHPStan installed (PhpStanWrapper::canRun); the default configuration is read as a project without it.
 */
function configured(): array
{
    $config = require package_directory() . '/config/validation.php';
    $ignored = array_merge($config['ignore_directives'] ?? [], PROVIDER_IGNORED);
    $custom = $config['custom_directives'] ?? [];
    $configured = [];
    foreach ($config['core_validators'] ?? [] as $class) {
        $options = $config['options'][$class] ?? [];
        $traits = class_uses($class);
        if (isset($traits[CanIgnoreDirectives::class])) $options['ignore_directives'] = array_merge($ignored, $options['ignore_directives'] ?? []);
        if (isset($traits[AcceptsCustomDirectives::class])) $options['custom_directives'] = array_merge($custom, $options['custom_directives'] ?? []);
        $configured[$class] = $options;
    }
    return $configured;
}

function facts(array $request): array
{
    $directory = package_directory();
    $configured = configured();
    $validators = [];
    foreach ($request['validators'] as $name) {
        $class = VALIDATORS . $name;
        if (!preg_match('/^\w+Validator$/', $name) || !class_exists($class) || !is_subclass_of($class, AbstractNodeValidator::class)) continue;
        $reflection = new ReflectionClass($class);
        $found = [];
        strings(new $class(), $found);
        $names = array_keys($found);
        sort($names, SORT_STRING);
        $deprecation = preg_match('/@deprecated\b\s*([^\n*]*)/', (string) $reflection->getDocComment(), $match) === 1 ? trim("Deprecated. {$match[1]}") : null;
        $test = 'tests/Validation/' . substr($name, 0, -strlen('Validator')) . 'Test.php';
        $validators[$name] = [
            'class' => $class,
            'file' => $reflection->getFileName(),
            'deprecation' => $deprecation,
            'test' => is_file("{$request['checkout']}/{$test}") ? $test : null,
            'names' => $names,
            'options' => array_key_exists($class, $configured) ? (object) $configured[$class] : null,
        ];
    }
    $package = json_decode(file_get_contents("{$directory}/composer.json"), true, 512, JSON_THROW_ON_ERROR);
    return [
        'version' => ltrim(InstalledVersions::getPrettyVersion(PACKAGE), 'v'),
        'framework' => ltrim(InstalledVersions::getPrettyVersion('laravel/framework'), 'v'),
        'license' => $package['license'] ?? null,
        'php' => PHP_VERSION,
        'package' => $directory,
        'validators' => (object) $validators,
    ];
}

function first_line(Throwable $error): string
{
    $message = trim(strtok($error->getMessage(), "\n") ?: '');
    return (new ReflectionClass($error))->getShortName() . ($message === '' ? '' : ": {$message}");
}

function lint(array $request): array
{
    $findings = [];
    $errors = [];
    foreach ($request['files'] as $file) {
        $found = [];
        try {
            foreach ($request['validators'] as $validator) {
                $class = VALIDATORS . $validator['name'];
                /** @var AbstractNodeValidator $instance */
                $instance = new $class();
                if ($validator['options'] !== null) $instance->loadConfiguration($validator['options']);
                $found[$validator['name']] = [];
                foreach (Document::fromText($file['source'])->addValidator($instance)->validate()->getValidationErrors() as $error) {
                    $found[$validator['name']][] = [
                        'line' => $error->position->startLine + $error->shiftLine,
                        'column' => $error->position->startColumn,
                        'message' => $error->message,
                    ];
                }
            }
            $findings[$file['path']] = (object) $found;
        } catch (Throwable $error) {
            $errors[$file['path']] = first_line($error);
        }
    }
    return ['findings' => (object) $findings, 'errors' => (object) $errors];
}

$request = json_decode(stream_get_contents(STDIN), true, 512, JSON_THROW_ON_ERROR);
$response = match ($request['op']) {
    'facts' => facts($request),
    'lint' => lint($request),
    default => throw new RuntimeException("unknown op {$request['op']}"),
};
echo json_encode($response, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE), "\n";
