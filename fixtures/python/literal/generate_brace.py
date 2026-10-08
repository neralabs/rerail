"""Writes brace.expected.json: for `str.format` templates, what CPython 3.13 reads (`_string.formatter_parser` over the
template and one level into each spec, `formatter_field_name_split` over each field name, and the error formatting
raises) and what Ruff 0.16.9 reports (F521's message). The cases are hand-written plus seeded random strings over the
characters the grammar gives meaning; the generator also checks a larger fuzz set against both and prints how often
they agree, without writing it. Run offline, not in CI:
python3.13 generate_brace.py <path to a ruff executable, 0.16.9> [fuzz count, default 200000]"""

from __future__ import annotations

import _string  # type: ignore[import-not-found]
import json
import pathlib
import random
import string
import subprocess
import sys
import tempfile

HERE: pathlib.Path = pathlib.Path(__file__).parent

CASES: list[str] = [
    "",
    "plain",
    "{}",
    "{{}}",
    "{{",
    "}}",
    "{",
    "}",
    "a}b",
    "a{b",
    "{0}",
    "{1}{0}",
    "{a}",
    "{a.b}",
    "{a[0]}",
    "{a[x]}",
    "{a[x][1].c}",
    "{a.}",
    "{a..b}",
    "{a[]}",
    "{a[}",
    "{a[0]b}",
    "{a!r}",
    "{a!s:>10}",
    "{a!}",
    "{a!rr}",
    "{a!x}",
    "{a:}",
    "{a:>{b}}",
    "{:{}}",
    "{:{}{}}",
    "{:{:{}}}",
    "{0:{1:{2}}}",
    "{a:{b.}}",
    "{a{b}}",
    "{0}{}",
    "{}{0}",
    "{:{0}}",
    "{0:{}}",
    "{}{}{}",
    "{2}",
    "{bar}{}",
    "{bar:{spam}}",
    "{key.attr[0][string]!r:0{2['arg']}{1}}",
    "{1.arg[1]!r:0{2['arg']}{1}}",
    "I am a \\N{snowman}",
    "I am a \\N{snowman}{snowman}",
    "I am a \\\\N{snowman}",
    "\\N{",
    "\\N{x",
    "{:%ЫйЯЧ}",
    "{é}",
    "{0:<>-#23,.11b}",
    "{0:033}",
    "{0:z}",
    "{0:.}",
    "{0:99999999999999999999999}",
    "{ }",
    "{0 }",
    "{-1}",
    "{+1}",
    "{01}",
    "{1_0}",
    "{١}",
    "{a[}]}",
    "{a[{]}",
    "{a:{{}}}",
    "{!r}",
    "{:}",
    "{!}",
    "{:!}",
    "{0!r:{1!r}}",
]

ALPHABET: str = "{}{}{}:!.[]0123aNr\\ x"


def fuzz(count: int, seed: int) -> list[str]:
    generator: random.Random = random.Random(seed)
    return ["".join(generator.choice(ALPHABET) for _ in range(generator.randint(0, 8))) for _ in range(count)]


def field(name: str, autos: list[int], indices: list[int], keywords: list[str]) -> None:
    first, rest = _string.formatter_field_name_split(name)
    list(rest)
    if first == "":
        autos.append(len(autos))
    elif isinstance(first, int):
        indices.append(first)
    else:
        keywords.append(first)


def cpython(text: str) -> dict[str, object]:
    """The template's fields as Ruff's FormatSummary lists them, or the parse error; and what formatting raises."""
    autos: list[int] = []
    indices: list[int] = []
    keywords: list[str] = []
    result: dict[str, object] = {}
    try:
        for _, name, spec, _ in _string.formatter_parser(text):
            if name is None:
                continue
            field(name, autos, indices, keywords)
            for _, inner, inner_spec, _ in _string.formatter_parser(spec or ""):
                if inner is None:
                    continue
                field(inner, autos, indices, keywords)
                if any(deeper is not None for _, deeper, _, _ in _string.formatter_parser(inner_spec or "")):
                    raise ValueError("Max string recursion exceeded")
        result = {"autos": autos, "indices": indices, "keywords": keywords}
    except ValueError as error:
        result = {"error": str(error)}

    class Keys(dict):  # type: ignore[type-arg]
        def __missing__(self, key: str) -> int:
            return 0

    try:
        string.Formatter().vformat(text, [0] * 20, Keys())
    except Exception as error:  # noqa: BLE001
        result["format"] = f"{type(error).__name__}: {error}"
    return result


def ruff(executable: str, cases: list[str]) -> list[str | None]:
    """Ruff's F521 message for each case, as `repr(case).format(x)` on a line of its own."""
    with tempfile.TemporaryDirectory() as directory:
        path: pathlib.Path = pathlib.Path(directory) / "cases.py"
        path.write_text("".join(f"{case!r}.format(x)\n" for case in cases))
        output: str = subprocess.run(
            [executable, "check", "--isolated", "--no-cache", "--select", "F521", "--output-format", "json", str(path)],
            capture_output=True,
            text=True,
            check=False,
        ).stdout
    found: list[str | None] = [None for _ in cases]
    for report in json.loads(output):
        found[int(report["location"]["row"]) - 1] = str(report["message"]).removeprefix(
            "`.format` call has invalid format string: "
        )
    return found


def main() -> None:
    executable: str = sys.argv[1]
    fuzz_count: int = int(sys.argv[2]) if len(sys.argv) > 2 else 200_000
    version: str = subprocess.run([executable, "--version"], capture_output=True, text=True, check=True).stdout.split()[-1]
    cases: list[str] = CASES + [case for case in fuzz(2_000, 13) if case not in CASES]
    ruffs: list[str | None] = ruff(executable, cases)
    rows: list[list[object]] = [[case, cpython(case), ruffs[index]] for index, case in enumerate(cases)]
    lines: list[str] = [json.dumps(row, ensure_ascii=False) for row in rows]
    (HERE / "brace.expected.json").write_text(
        '{"python": %s, "ruff": %s, "cases": [\n%s\n]}\n'
        % (json.dumps(sys.version.split()[0]), json.dumps(version), ",\n".join(lines))
    )
    big: list[str] = fuzz(fuzz_count, 31)
    big_ruff: list[str | None] = ruff(executable, big)
    outcomes: dict[str, int] = {}
    for case, found in zip(big, big_ruff):
        python: dict[str, object] = cpython(case)
        key: str = f"python {'error' if 'error' in python else 'ok'} / ruff {'error' if found else 'ok'}"
        outcomes[key] = outcomes.get(key, 0) + 1
    print(json.dumps({"fuzz": fuzz_count, "outcomes": outcomes}, indent=1))


if __name__ == "__main__":
    main()
