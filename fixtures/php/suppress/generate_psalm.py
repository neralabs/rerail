"""Writes psalm.expected.json with Psalm 6.19.1's reports for each suppress/psalm fixture: with its
`@psalm-suppress` tags renamed to `@xsalm-suppress` (`all`, same byte offsets), as written under
Psalm's default configuration (`kept`), and as written with `disableSuppressAll="false"`
(`keptSuppressAll`, where `@psalm-suppress all` counts). `parents` records
`Psalm\\Config::getParentIssueType` for every issue class Psalm ships (null where it has none).
Run offline, never in CI:
python3 generate_psalm.py /tmp/rerail-php-tools

Requires PHP 8.5.11 and Psalm 6.19.1 (vimeo/psalm, installed with Composer in the directory
given). Each fixture is analysed alone at errorLevel 1. Reports are [issue type, start byte,
end byte]."""
from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).parent

CONFIG = """<?xml version="1.0"?>
<psalm errorLevel="1" findUnusedCode="false" findUnusedBaselineEntry="false"
       resolveFromConfigFile="true"{extra}>
    <projectFiles><directory name="src" /></projectFiles>
</psalm>
"""

PARENTS = """
require $argv[1] . '/vendor/autoload.php';
$parents = [];
foreach (glob($argv[1] . '/vendor/vimeo/psalm/src/Psalm/Issue/*.php') as $file) {
    $name = basename($file, '.php');
    $class = 'Psalm\\\\Issue\\\\' . $name;
    $reflection = new ReflectionClass($class);
    if ($reflection->isAbstract() || !$reflection->isSubclassOf('Psalm\\\\Issue\\\\CodeIssue')) {
        continue;
    }
    $parents[$name] = Psalm\\Config::getParentIssueType($name);
}
ksort($parents);
echo json_encode($parents);
"""


def report(psalm: str, project: pathlib.Path, source: str, extra: str) -> list[list[object]]:
    (project / "psalm.xml").write_text(CONFIG.format(extra=extra))
    (project / "src" / "fixture.php").write_text(source)
    output = subprocess.run(
        [psalm, "--no-cache", "--no-progress", "--threads=1", "--output-format=json"],
        cwd=project,
        capture_output=True,
        text=True,
        check=False,
    ).stdout
    return sorted([issue["type"], issue["from"], issue["to"]] for issue in json.loads(output))


def main() -> None:
    tools = pathlib.Path(sys.argv[1])
    psalm = str(tools / "vendor/bin/psalm")
    version = subprocess.run([psalm, "--version"], capture_output=True, text=True, check=True).stdout.strip()
    results: dict[str, object] = {"psalm": version}
    parents = subprocess.run(
        ["php", "-r", PARENTS, str(tools)], capture_output=True, text=True, check=True
    ).stdout
    results["parents"] = json.loads(parents)
    for path in sorted((HERE / "psalm").glob("*.php")):
        source = path.read_text()
        with tempfile.TemporaryDirectory() as temp:
            project = pathlib.Path(temp)
            (project / "src").mkdir()
            results[path.name] = {
                "all": report(psalm, project, source.replace("@psalm-suppress", "@xsalm-suppress"), ""),
                "kept": report(psalm, project, source, ""),
                "keptSuppressAll": report(psalm, project, source, ' disableSuppressAll="false"'),
            }
    (HERE / "psalm.expected.json").write_text(json.dumps(results, indent=1) + "\n")


if __name__ == "__main__":
    main()
