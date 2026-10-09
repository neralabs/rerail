<?php
// Runs tlint 9.7.0 linters over templates as `tlint lint <file>` does (LintCommand::lintFile: each linter class is
// constructed with the file's contents and its path, and TLint::lint() returns its lints), and prints, as JSON,
// {"version", "findings": [{rule, path, line}], "skipped": [path]}: a finding per lint, at its node's start line
// (`tlint lint --json`'s `line`), in the linter's order; a file a linter throws on is skipped.
// Called by generate.py:  php run.php <composer project with tightenco/tlint 9.7.0> <root> <Lint>... < paths
// (one per line, relative to <root>)

declare(strict_types=1);

use Composer\InstalledVersions;
use Tighten\TLint\TLint;

require $argv[1].'/vendor/autoload.php';

$root = $argv[2];
$lints = array_slice($argv, 3);
$tlint = new TLint;
$findings = [];
$skipped = [];
foreach (preg_split('/\R/', trim(stream_get_contents(STDIN)), -1, PREG_SPLIT_NO_EMPTY) as $path) {
    $file = $root.'/'.$path;
    try {
        $found = [];
        foreach ($lints as $lint) {
            $class = 'Tighten\\TLint\\Linters\\'.$lint;
            foreach ($tlint->lint(new $class(file_get_contents($file), $file)) as $result) {
                $found[] = ['rule' => $lint, 'path' => $path, 'line' => $result->getNode()->getStartLine()];
            }
        }
        array_push($findings, ...$found);
    } catch (Throwable) {
        $skipped[] = $path;
    }
}
echo json_encode([
    'version' => ltrim(InstalledVersions::getPrettyVersion('tightenco/tlint'), 'v'),
    'findings' => $findings,
    'skipped' => $skipped,
], JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE), "\n";
