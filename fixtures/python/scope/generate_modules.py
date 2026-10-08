"""Writes modules/expected.json: how Ruff qualifies the definitions and relative imports of every modules/**/*.py by the
file's module path. TID251 (banned-api) reports attribute chains that resolve to a banned name; B008 is silent on a call
in a default whose callee resolves to an `extend-immutable-calls` name. Run offline, not in CI:
python3 generate_modules.py <path to a ruff executable>."""

from __future__ import annotations

import ast
import json
import pathlib
import subprocess
import sys
import tempfile

# Qualified names the fixtures may or may not resolve to; each list is also the policy's `import` list.
BANNED: list[str] = [
    "contextlib.Box.open",
    "fn.contextlib.Box.open",
    "pkg.E.z",
    "pkg.__init__.E.z",
    "pkg.mod.C.x",
    "pkg.sub.mod.C.x",
    "pkg.sub.mod.D.y",
    "pkg.sub.mod.f.D.y",
    "pkg.sub.sibling.attr",
    "pkg.top.attr",
    "beyond.attr",
    "far.attr",
    "pkg.sub.sibling.thing.attr",
    "mod.C.x",
    "pkg.loose.mod.C.x",
    "sibling.attr",
]
IMMUTABLE: list[str] = [
    "contextlib.suppress",
    "contextlib.Box",
    "pkg.__init__.E",
    "pkg.E",
    "pkg.mod.C",
    "pkg.sub.sibling.make",
    "pkg.sub.mod.f",
    "pkg.sub.mod.C",
    "mod.C",
    "sibling",
]


def module_path(path: pathlib.Path) -> str | None:
    """Ruff's `detect_package_root` and `to_module_path`: climb while each directory holds an `__init__.py`."""
    root: pathlib.Path | None = None
    directory: pathlib.Path = path.parent
    while (directory / "__init__.py").is_file():
        root = directory
        directory = directory.parent
    if root is None:
        return None
    return ".".join(pathlib.PurePath(part).stem for part in path.relative_to(root.parent).parts)


def default_calls(source: bytes) -> list[int]:
    """Byte offsets of the callee of each call in a parameter default."""
    lines: list[bytes] = source.splitlines(keepends=True)
    starts: list[int] = []
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.FunctionDef | ast.AsyncFunctionDef):
            for default in [*node.args.defaults, *node.args.kw_defaults]:
                if isinstance(default, ast.Call):
                    starts.append(len(b"".join(lines[: default.lineno - 1])) + default.col_offset)
    return sorted(starts)


def main() -> None:
    ruff: str = sys.argv[1]
    here: pathlib.Path = pathlib.Path(__file__).parent
    root: pathlib.Path = here / "modules"
    banned: str = "\n".join(f'"{name}".msg = "banned"' for name in BANNED)
    config: str = (
        'target-version = "py313"\n[lint]\nselect = ["TID251", "B008"]\n'
        f"[lint.flake8-bugbear]\nextend-immutable-calls = {json.dumps(IMMUTABLE)}\n"
        f"[lint.flake8-tidy-imports.banned-api]\n{banned}\n"
    )
    files: list[pathlib.Path] = sorted(root.rglob("*.py"))
    with tempfile.TemporaryDirectory() as scratch:
        config_path: pathlib.Path = pathlib.Path(scratch) / "ruff.toml"
        config_path.write_text(config)
        command: list[str] = [ruff, "check", "--no-cache", "--config", str(config_path), "--output-format", "json"]
        report = subprocess.run([*command, *map(str, files)], capture_output=True, text=True, check=False, cwd=root)
    diagnostics: list[dict[str, object]] = json.loads(report.stdout)
    version: str = subprocess.run([ruff, "--version"], capture_output=True, text=True, check=True).stdout.strip()
    out: list[dict[str, object]] = []
    for path in files:
        source: bytes = path.read_bytes()
        lines: list[bytes] = source.splitlines(keepends=True)

        def offset(location: dict[str, int]) -> int:
            row: int = location["row"]
            prefix: str = lines[row - 1].decode()[: location["column"] - 1]
            return len(b"".join(lines[: row - 1])) + len(prefix.encode())

        mine = [item for item in diagnostics if pathlib.Path(str(item["filename"])).resolve() == path.resolve()]
        tid251: list[dict[str, object]] = [item for item in mine if item["code"] == "TID251"]
        banned_at: list[dict[str, object]] = sorted(
            ({"start": offset(item["location"]), "name": str(item["message"]).split("`")[1]} for item in tid251),
            key=lambda item: int(str(item["start"])),
        )
        mutable: set[int] = {offset(item["location"]) for item in mine if item["code"] == "B008"}
        calls: list[int] = default_calls(source)
        unexpected: set[str] = {str(item["code"]) for item in mine} - {"TID251", "B008"}
        if unexpected:
            raise SystemExit(f"{path}: unexpected {unexpected}")
        out.append(
            {
                "path": str(path.relative_to(root)),
                "module": module_path(path),
                "banned": banned_at,
                "immutable": [start for start in calls if start not in mutable],
                "mutable": [start for start in calls if start in mutable],
            }
        )
    expected: dict[str, object] = {
        "oracle": f"{version} TID251 banned-api and B008 extend-immutable-calls, --target-version py313",
        "banned": BANNED,
        "immutableCalls": IMMUTABLE,
        "files": out,
    }
    (root.parent / "modules.expected.json").write_text(json.dumps(expected, indent=1) + "\n")


if __name__ == "__main__":
    main()
