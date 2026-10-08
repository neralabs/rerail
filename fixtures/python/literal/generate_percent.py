"""Writes percent.expected.json: for printf-style format strings, what CPython 3.13 reads (the error `%` raises, the
mapping keys it looks up and the positional values it consumes) and what Ruff 0.16.9 reports (F501's message, F509's
character). The cases are hand-written plus seeded random strings over the characters the grammar gives meaning; the
generator also checks a larger fuzz set against both and prints the disagreements it summarizes, without writing it.
Run offline, not in CI:
python3.13 generate_percent.py <path to a ruff executable, 0.16.9> [fuzz count, default 200000]"""

from __future__ import annotations

import json
import pathlib
import random
import subprocess
import sys
import tempfile

HERE: pathlib.Path = pathlib.Path(__file__).parent

CASES: list[str] = [
    "",
    "plain",
    "%s",
    "%%",
    "%% %s %%",
    "%",
    "abc %",
    "%s %",
    "%(a",
    "%(a)",
    "%(a)s",
    "%(a)s %(b)d",
    "%(a(b))s",
    "%(a(b)s",
    "%((a)s",
    "%()s",
    "%(a)s %s",
    "%s %(a)s",
    "%(a)*s",
    "%(a).*s",
    "%*s",
    "%.*f",
    "%*.*f",
    "%-*.*f",
    "%5",
    "%.3",
    "%-",
    "%z",
    "%(a)z",
    "% z",
    "%5z",
    "%.5z",
    "%hz",
    "%lld",
    "%ld",
    "%Lf",
    "%hd",
    "%*z",
    "%.*z",
    "%%z",
    "%s %z %",
    "%b",
    "%a",
    "%r",
    "%c",
    "%i",
    "%u",
    "%o",
    "%x",
    "%X",
    "%e",
    "%E",
    "%f",
    "%F",
    "%g",
    "%G",
    "%#o",
    "%0d",
    "%+d",
    "% d",
    "%-5d",
    "%  0   -+++###10d",
    "%10.3f",
    "%.f",
    "%.",
    "%2147483647d",
    "%2147483648d",
    "%99999999999d",
    "%.2147483648f",
    "%.99999999999f",
    "%1$s",
    "%ü",
    "é%ü",
    "é%s",
    "%(é)s",
    "%\n",
    "%(a\n)s",
    "%s\n%s",
    "%(a)s%%(b)s",
    "100%",
    "100%%",
    "%(a)s %(a)s",
    "%s%s%s%s",
    "%(key)#5.3lx",
    "%j",
    "%S",
    "%D",
    "% ",
    "%#",
    "%(a) s",
    "%(a)-s",
    "%.*%",
    "%*%",
    "%(a)%",
    "%5%",
]

# These width-boundary cases establish CPython's argument consumption, not its padded output.
# Formatting their original widths would allocate gigabytes merely to count one argument.
POSITIONAL_COUNT_FORMATS: dict[str, str] = {
    "%2147483647d": "%0d",
    "%2147483648d": "%0d",
    "%99999999999d": "%0d",
}


ALPHABET: str = "%()#0- +*.12349hlLdsrxzaé\n"


def fuzz(count: int, seed: int) -> list[str]:
    generator: random.Random = random.Random(seed)
    return ["".join(generator.choice(ALPHABET) for _ in range(generator.randint(0, 8))) for _ in range(count)]


def cpython(text: str) -> dict[str, object]:
    """The error `%` raises for `text`, or the keys it looks up and the positional values it consumes."""

    class Keys(dict):  # type: ignore[type-arg]
        def __init__(self) -> None:
            super().__init__()
            self.seen: list[str] = []

        def __getitem__(self, key: str) -> int:
            self.seen.append(key)
            return 0

    try:
        keys = Keys()
        text % keys
        mapped: list[str] | None = keys.seen
    except TypeError:
        mapped = None
    except ValueError as error:
        return {"error": str(error)}
    except OverflowError as error:
        return {"error": f"OverflowError: {error}"}
    positional_text: str = POSITIONAL_COUNT_FORMATS.get(text, text)

    for count in range(0, 16):
        try:
            positional_text % ((0,) * count)
        except TypeError as error:
            message: str = str(error)
            if "not enough arguments" in message:
                continue
            if "not all arguments converted" in message:
                return {"keys": mapped or [], "positional": None}
            if "format requires a mapping" in message:
                return {"keys": mapped or [], "positional": None}
            return {"keys": mapped or [], "positional": None}
        except ValueError as error:
            return {"error": str(error)}
        except OverflowError as error:
            return {"error": f"OverflowError: {error}"}
        return {"keys": mapped or [], "positional": count}
    return {"keys": mapped or [], "positional": None}


def ruff(executable: str, cases: list[str]) -> list[dict[str, object]]:
    """Ruff's F501 message or F509 character for each case, as `repr(case) % x` on a line of its own."""
    with tempfile.TemporaryDirectory() as directory:
        path: pathlib.Path = pathlib.Path(directory) / "cases.py"
        path.write_text("".join(f"{case!r} % x\n" for case in cases))
        output: str = subprocess.run(
            [executable, "check", "--isolated", "--no-cache", "--select", "F501,F509", "--output-format", "json", str(path)],
            capture_output=True,
            text=True,
            check=False,
        ).stdout
    found: list[dict[str, object]] = [{} for _ in cases]
    for report in json.loads(output):
        row: int = int(report["location"]["row"]) - 1
        message: str = str(report["message"])
        if report["code"] == "F501":
            found[row] = {"F501": message.removeprefix("`%`-format string has invalid format string: ")}
        else:
            found[row] = {"F509": message.removeprefix("`%`-format string has unsupported format character `")[:-1]}
    return found


def main() -> None:
    executable: str = sys.argv[1]
    fuzz_count: int = int(sys.argv[2]) if len(sys.argv) > 2 else 200_000
    version: str = subprocess.run([executable, "--version"], capture_output=True, text=True, check=True).stdout.split()[-1]
    cases: list[str] = CASES + [case for case in fuzz(5_000, 11) if case not in CASES]
    ruffs: list[dict[str, object]] = ruff(executable, cases)
    rows: list[list[object]] = [[case, cpython(case), ruffs[index]] for index, case in enumerate(cases)]
    result: dict[str, object] = {"python": sys.version.split()[0], "ruff": version, "cases": rows}
    lines: list[str] = [json.dumps(row, ensure_ascii=False) for row in rows]
    (HERE / "percent.expected.json").write_text(
        '{"python": %s, "ruff": %s, "cases": [\n%s\n]}\n'
        % (json.dumps(result["python"]), json.dumps(version), ",\n".join(lines))
    )
    # The larger fuzz set: where CPython and Ruff disagree on whether a string is valid.
    big: list[str] = fuzz(fuzz_count, 29)
    big_ruff: list[dict[str, object]] = ruff(executable, big)
    disagreements: dict[str, int] = {}
    for case, found in zip(big, big_ruff):
        python: dict[str, object] = cpython(case)
        key: str = f"python {'error' if 'error' in python else 'ok'} / ruff {'error' if found else 'ok'}"
        disagreements[key] = disagreements.get(key, 0) + 1
    print(json.dumps({"fuzz": fuzz_count, "outcomes": disagreements}, indent=1))


if __name__ == "__main__":
    main()
