"""Writes annotation.expected.json: for string annotations, what Ruff 0.16.9 reports as F722 (its parser's syntax
error on the whole literal, or its checker's semantic syntax errors inside it) and whether CPython 3.13 compiles the
value as an expression (`compile(value, "", "eval")`). Each case is written as an annotation `x: <literal>` in a host:
at module level, or in a function, an async function, a class, an async function's parameter, or a function that
declares `global a, b` after it (the checks depend on the scopes around the string). The literal is single- or
triple-quoted (triple quotes read it as a parenthesized expression, as Ruff does), or escaped so its source is not its
value (Ruff's complex annotations, reported on the whole literal). A case is [value, style, literal, host, reports,
compiles], each report [start, end, message] in bytes from the literal's start. The cases are hand-written plus seeded
random strings over Python tokens; the generator also checks a larger fuzz set and prints how often Ruff and CPython
agree. Run offline, not in CI:
python3.13 generate_annotation.py <path to a ruff executable, 0.16.9> [fuzz count, default 20000]"""

from __future__ import annotations

import json
import pathlib
import random
import subprocess
import sys
import tempfile
import warnings

HERE: pathlib.Path = pathlib.Path(__file__).parent

CASES: list[str] = [
    "int",
    " int",
    "int ",
    "\tint",
    "int # comment",
    "List[int]",
    "List[int",
    "Dict[str, int]",
    "a, b",
    "(a, b)",
    "a,",
    "*a",
    "(*a,)",
    "a | b",
    "a |",
    "int |\nstr",
    "(int |\nstr)",
    "a b",
    "print x",
    "exec 'x'",
    "async",
    "await",
    "await x",
    "match",
    "case",
    "type",
    "_",
    "yield",
    "yield x",
    "lambda: 1",
    "lambda x: x",
    "x := 1",
    "(x := 1)",
    "a = 1",
    "a if b else c",
    "a if b",
    "not a",
    "a and b",
    "-1",
    "1 +",
    "1 + 2",
    "'str'",
    "\"str\"",
    "f'{x}'",
    "b'x'",
    "...",
    "None",
    "True",
    "x.y",
    "x.",
    "x[1:2]",
    "x()",
    "x(",
    "[int]",
    "{int: str}",
    "{int}",
    "[x for x in y]",
    "(x for x in y)",
    "",
    " ",
    "\n",
    "int\n",
    "\nint",
    "int\\",
    "a \\\nb",
    "☃",
    "é",
    "x y",
    "()",
    "[]",
    "{}",
    "@x",
    "a @ b",
    "a[b][c]",
    "Callable[[int], str]",
    "Callable[..., str]",
    "Literal['a']",
    "Annotated[int, 'x']",
    "a.b.c[d.e]",
    "0x10",
    "1_000",
    "1e5",
    "1j",
    "a < b",
    "a is not b",
    "a not in b",
    "~a",
    "a ** b",
    "a // b",
    "a;",
    "a; b",
    "if x",
    "for",
    "class",
    "def",
    "import os",
    "global x",
    "(yield y)",
    "(yield from y)",
    "(yield *y)",
    "(yield from *y)",
    "(yield) if a else b",
    "(x := yield)",
    "f(x=(yield))",
    "f(yield y)",
    "[yield y]",
    "a[yield y]",
    "f(await y)",
    "await await y",
    "-await y",
    "await y ** 2",
    "not await y",
    "f(**await y)",
    "a if await b else c",
    "List[await y]",
    "lambda: await y",
    "lambda: (yield)",
    "lambda __debug__: 1",
    "lambda a, a: 1",
    "lambda a, *a: 1",
    "f(a=1, a=2)",
    "f(a=1, b=2, **c)",
    "(__debug__ := 1)",
    "(__debug__)",
    "[a for *a in b]",
    "[a for (*a, *c) in b]",
    "[a for __debug__ in b]",
    "[a for a.b in c]",
    "[a async for a in b]",
    "(a async for a in b)",
    "{a async for a in b}",
    "{a: b async for a in c}",
    "[await a for a in b]",
    "(await a for a in b)",
    "(a for a in (await b))",
    "(await a async for a in b)",
    "[[a async for a in b] for c in d]",
    "[[a async for a in b] async for c in d]",
    "[a for a in b if (a := 1)]",
    "[a for a in (b := c)]",
    "[(a := 1) for a in b]",
    "[(a := 1) for b in c]",
    "[x for a in b if (c := 1) for c in d]",
    "[[(c := 1) for e in f] for a in b if 1 for c in d]",
    "{(a := 1): 1 for a in b}",
    "[lambda b=(a := 1): a for a in c]",
]

# Hosts: where the annotation sits. `{}` is the literal.
HOSTS: dict[str, str] = {
    "module": "x: {}\n",
    "function": "def f():\n    x: {}\n",
    "async": "async def f():\n    x: {}\n",
    "class": "class C:\n    x: {}\n",
    "parameter": "async def f(p: {}): pass\n",
    "global": "def f():\n    x: {}\n    global a, b\n",
}

# Values read in every host: the checks that depend on the scopes around the string.
CONTEXTUAL: list[str] = [
    "await x",
    "(yield)",
    "(yield from x)",
    "[a async for a in b]",
    "(a async for a in b)",
    "[await a for a in b]",
    "(await a for a in b)",
    "lambda: await x",
    "lambda: (yield)",
    "[(q := 1) for a in b]",
    "[[a async for a in b] for c in d]",
    "a",
    "a.b",
    "f(b=a)",
    "[a for c in d]",
    "b or (a := 1)",
    "lambda c=a: a",
]

TOKENS: list[str] = [
    "int", "str", "a", "b", "List", "[", "]", "(", ")", ",", ".", "|", " ", "\n", "*", ":", "=", "1", "'x'",
    "lambda", "if", "else", "not", "await", "async", "{", "}", "#", "\\", "\t",
]


def fuzz(count: int, seed: int) -> list[str]:
    generator: random.Random = random.Random(seed)
    return ["".join(generator.choice(TOKENS) for _ in range(generator.randint(1, 6))) for _ in range(count)]


def literal(value: str, style: str) -> str | None:
    """The value as a Python literal of the style, or `None` where the style cannot hold it."""
    if style == "single":
        if any(c in value for c in "\n\r'\\"):
            return None
        return f"'{value}'"
    if style == "triple":
        if "'''" in value or "\\" in value or value.endswith("'"):
            return None
        return f"'''{value}'''"
    return "'" + "".join(f"\\x{ord(c):02x}" if ord(c) < 0x80 else c for c in value) + "'"


def cpython(value: str) -> bool:
    try:
        compile(value, "<annotation>", "eval")
    except SyntaxError:
        return False
    except ValueError:
        return False
    return True


def ruff(executable: str, cases: list[tuple[str, str]]) -> list[list[list[object]]]:
    """Ruff's F722 reports on each (host, literal) case, relative to the literal; each case is a file of its own. Ruff
    leaves a comprehension's range unrelocated in a complex string, so a report may fall outside its literal."""
    found: list[list[list[object]]] = [[] for _ in cases]
    prefix: str = "Syntax error in forward annotation: "
    with tempfile.TemporaryDirectory() as directory:
        sources: list[str] = []
        for index, (host, literal) in enumerate(cases):
            source: str = HOSTS[host].format(literal)
            sources.append(source)
            (pathlib.Path(directory) / f"case{index}.py").write_text(source)
        output: str = subprocess.run(
            [executable, "check", "--isolated", "--no-cache", "--select", "F722", "--output-format", "json", directory],
            capture_output=True,
            text=True,
            check=False,
        ).stdout
    for report in json.loads(output):
        if report["code"] != "F722":
            continue
        index: int = int(pathlib.Path(report["filename"]).stem.removeprefix("case"))
        source = sources[index]
        lines: list[str] = source.splitlines(keepends=True)
        offset: int = len(source[: source.index(cases[index][1])].encode())
        start: int = byte_offset(lines, int(report["location"]["row"]), int(report["location"]["column"]))
        end: int = byte_offset(lines, int(report["end_location"]["row"]), int(report["end_location"]["column"]))
        message: str = str(report["message"]).removeprefix(prefix)
        found[index].append([start - offset, end - offset, message])
    return [sorted(reports, key=lambda report: (report[0], report[1], report[2])) for reports in found]


def byte_offset(lines: list[str], row: int, column: int) -> int:
    return len(("".join(lines[: row - 1]) + lines[row - 1][: column - 1]).encode())


def rows(executable: str, values: list[str], hosts: list[str]) -> list[list[object]]:
    cases: list[tuple[str, str, str, str]] = []
    for host in hosts:
        for value in values:
            for style in ("single", "triple", "escaped"):
                source = literal(value, style)
                if source is not None:
                    cases.append((value, style, source, host))
    reports: list[list[list[object]]] = ruff(executable, [(host, source) for _, _, source, host in cases])
    return [
        [value, style, source, host, reports[index], cpython(value)]
        for index, (value, style, source, host) in enumerate(cases)
    ]


def main() -> None:
    warnings.filterwarnings("ignore", category=SyntaxWarning)
    executable: str = sys.argv[1]
    fuzz_count: int = int(sys.argv[2]) if len(sys.argv) > 2 else 20_000
    version: str = subprocess.run([executable, "--version"], capture_output=True, text=True, check=True).stdout.split()[-1]
    committed: list[list[object]] = rows(executable, CASES + fuzz(700, 17), ["module"])
    committed += rows(executable, CONTEXTUAL, [host for host in HOSTS if host != "module"])
    lines: list[str] = [json.dumps(row, ensure_ascii=False) for row in committed]
    (HERE / "annotation.expected.json").write_text(
        '{"python": %s, "ruff": %s, "cases": [\n%s\n]}\n'
        % (json.dumps(sys.version.split()[0]), json.dumps(version), ",\n".join(lines))
    )
    outcomes: dict[str, int] = {}
    for _, style, _, _, reports, compiles in rows(executable, fuzz(fuzz_count, 19), ["module"]):
        key: str = f"{style}: ruff {'error' if reports else 'ok'} / python {'ok' if compiles else 'error'}"
        outcomes[key] = outcomes.get(key, 0) + 1
    print(json.dumps({"fuzz": fuzz_count, "outcomes": dict(sorted(outcomes.items()))}, indent=1))


if __name__ == "__main__":
    main()
