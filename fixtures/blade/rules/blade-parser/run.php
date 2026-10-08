<?php
// Runs stillat/blade-parser 2.1.0 node validators over templates as its tests run them (tests/Validation): per file
// and validator, Document::fromText(<contents>)->addValidator(new <Validator>)->validate()->getValidationErrors(),
// each validator constructed with its defaults (no custom or ignored directives) on a bare container (no Laravel
// application, so the parser knows the core directives and no custom one). Prints, as JSON,
// {"version", "findings": [{rule, path, line, message}], "skipped": [path]}: a finding per validation error, at the
// start line of the node it reports (plus its shift line), in the validator's order; a file the parser or a
// validator throws on is skipped.
// Called by generate.py:  php run.php <composer project with stillat/blade-parser 2.1.0> <root> <Validator>... < paths
// (one per line, relative to <root>)

declare(strict_types=1);

use Composer\InstalledVersions;
use Stillat\BladeParser\Document\Document;

require $argv[1].'/vendor/autoload.php';

$root = $argv[2];
$validators = array_slice($argv, 3);
$findings = [];
$skipped = [];
foreach (preg_split('/\R/', trim(stream_get_contents(STDIN)), -1, PREG_SPLIT_NO_EMPTY) as $path) {
    $text = file_get_contents($root.'/'.$path);
    try {
        $found = [];
        foreach ($validators as $validator) {
            $class = 'Stillat\\BladeParser\\Validation\\Validators\\'.$validator;
            foreach (Document::fromText($text)->addValidator(new $class)->validate()->getValidationErrors() as $error) {
                $found[] = ['rule' => $validator, 'path' => $path, 'line' => $error->position->startLine + $error->shiftLine,
                    'message' => $error->message];
            }
        }
        array_push($findings, ...$found);
    } catch (Throwable) {
        $skipped[] = $path;
    }
}
echo json_encode([
    'version' => ltrim(InstalledVersions::getPrettyVersion('stillat/blade-parser'), 'v'),
    'findings' => $findings,
    'skipped' => $skipped,
], JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE), "\n";
