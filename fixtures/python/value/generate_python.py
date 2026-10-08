"""Writes python.expected.json: for every expression in python.cases.json, what CPython makes of it. `literalEval` is
ast.literal_eval, `folded` the constant ast.parse(optimize=1) folds it to (null when it is not folded), and `eval` the
value it evaluates to, with the names x (an object), a ([7]) and d ({"k": 1}) bound. Run offline, not in CI, with
CPython 3.13: python3.13 generate_python.py"""

from __future__ import annotations

import ast
import builtins
import json
import pathlib
import sys
import warnings

HERE: pathlib.Path = pathlib.Path(__file__).parent


def encode(value: object) -> object:
    match value:
        case None:
            return {"none": None}
        case bool():
            return {"bool": value}
        case int():
            return {"int": str(value)}
        case float():
            return {"float": repr(value)}
        case complex():
            return {"complex": [repr(value.real), repr(value.imag)]}
        case str():
            try:
                value.encode("utf-8")
            except UnicodeEncodeError:
                return {"codePoints": [ord(c) for c in value]}
            return {"str": value}
        case bytes():
            return {"bytes": value.hex()}
        case tuple():
            return {"tuple": [encode(item) for item in value]}
        case list():
            return {"list": len(value)}
        case set():
            return {"set": len(value)}
        case frozenset():
            return {"frozenset": len(value)}
        case dict():
            return {"dict": len(value)}
    if value is Ellipsis:
        return {"ellipsis": None}
    return {"other": type(value).__name__}


def outcome(run: object) -> object:
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return encode(run())  # type: ignore[operator]
    except Exception as error:  # noqa: BLE001 - the oracle records every failure by type
        return {"error": type(error).__name__}


def folded(source: str) -> object:
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            tree: ast.Expression = ast.parse(source, mode="eval", optimize=1)
    except SyntaxError:
        return None
    body: ast.expr = tree.body
    return encode(body.value) if isinstance(body, ast.Constant) else None


def evaluate(source: str) -> object:
    names: dict[str, object] = {"__builtins__": builtins, "x": object(), "a": [7], "d": {"k": 1}}
    return eval(compile(source, "<case>", "eval"), names)  # noqa: S307 - the cases are this file's own


def main() -> None:
    if sys.version_info[:2] != (3, 13):
        raise SystemExit(f"run with CPython 3.13, not {sys.version}")
    cases: list[object] = json.loads((HERE / "python.cases.json").read_text())
    results: list[dict[str, object]] = []
    for case in cases:
        source: str = case if isinstance(case, str) else case["src"]  # type: ignore[index]
        result: dict[str, object] = {
            "src": source,
            "literalEval": outcome(lambda: ast.literal_eval(source)),
            "folded": folded(source),
            "eval": outcome(lambda: evaluate(source)),
        }
        if isinstance(case, dict):
            result["unknown"] = case["unknown"]
        results.append(result)
    lines: list[str] = [json.dumps(result, ensure_ascii=False) for result in results]
    version: str = json.dumps(sys.version.split()[0])
    (HERE / "python.expected.json").write_text(f'{{"python": {version}, "cases": [\n' + ",\n".join(lines) + "\n]}\n")


if __name__ == "__main__":
    main()
