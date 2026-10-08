"""Runs project/main.py under sys.monitoring and writes, as JSON, every call the workspace's own
code made: per file, each call expression's byte range and line, and what it called each time it
ran. Run by generate.py, under the CPython it pins:

    python3.13 trace.py <project directory> <output JSON>

A call is a CALL event (`sys.monitoring.events.CALL`) in a code object of a project file; its site
is the call expression's span (`co_positions` of the instruction), kept only where it is an
`ast.Call` node's span, so implicit calls (a decorator's application, a class body) are left out,
except where a decorator is itself a call (`@functools.wraps(f)`): its application shares that
call's span, so both are recorded there. CPython compiles `super().m()` without calling `super`,
so that `super()` is never recorded.
What it called:
- ["def", file, line]: a function the workspace defines, at its `def` line (a bound method's
  function, a class's `__init__` or an instance's `__call__` where the workspace defines them);
- ["lambda", file, line]: a lambda of the workspace;
- ["class", file, line]: a class of the workspace whose `__init__` is no workspace function;
- ["external", name]: anything else (builtins, the standard library, C callables)."""

from __future__ import annotations

import ast
import json
import pathlib
import runpy
import sys
import types

TOOL = sys.monitoring.DEBUGGER_ID

root = pathlib.Path(sys.argv[1]).resolve()
sys.path.insert(0, str(root))
sys.dont_write_bytecode = True


def project_file(filename: str) -> str | None:
    path = pathlib.Path(filename)
    if not path.is_absolute():
        return None
    try:
        return path.resolve().relative_to(root).as_posix()
    except ValueError:
        return None


class Source:
    """One project file: its line starts (bytes), its calls' spans, and the `def` line of each
    function by the line its code object starts at (the first decorator's, where it has any)."""

    def __init__(self, relative: str) -> None:
        data = (root / relative).read_bytes()
        self.starts = [0]
        for index, byte in enumerate(data):
            if byte == 0x0A:
                self.starts.append(index + 1)
        tree = ast.parse(data)
        self.calls: set[tuple[int, int]] = set()
        self.defs: dict[int, int] = {}
        for node in ast.walk(tree):
            if isinstance(node, ast.Call):
                self.calls.add(self.span(node.lineno, node.col_offset, node.end_lineno, node.end_col_offset))
            elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                first = min([node.lineno, *(decorator.lineno for decorator in node.decorator_list)])
                self.defs[first] = node.lineno

    def span(self, line: int, column: int, end_line: int, end_column: int) -> tuple[int, int]:
        # `co_positions` and `ast` columns are UTF-8 byte offsets in their line.
        return self.starts[line - 1] + column, self.starts[end_line - 1] + end_column


sources: dict[str, Source] = {}


def source(relative: str) -> Source:
    if relative not in sources:
        sources[relative] = Source(relative)
    return sources[relative]


def function_target(function: types.FunctionType) -> list[object]:
    code = function.__code__
    relative = project_file(code.co_filename)
    if relative is None:
        return ["external", f"{function.__module__}.{function.__qualname__}"]
    if code.co_name == "<lambda>":
        return ["lambda", relative, code.co_firstlineno]
    return ["def", relative, source(relative).defs.get(code.co_firstlineno, code.co_firstlineno)]


def target(callable_: object) -> list[object]:
    if isinstance(callable_, types.MethodType):
        callable_ = callable_.__func__
    if isinstance(callable_, types.FunctionType):
        return function_target(callable_)
    if isinstance(callable_, type):
        init = callable_.__init__
        if isinstance(init, types.FunctionType) and project_file(init.__code__.co_filename) is not None:
            return function_target(init)
        module = sys.modules.get(callable_.__module__)
        filename = getattr(module, "__file__", None)
        relative = project_file(filename) if filename else None
        if relative is not None:
            for node in ast.walk(ast.parse((root / relative).read_bytes())):
                if isinstance(node, ast.ClassDef) and node.name == callable_.__name__:
                    return ["class", relative, node.lineno]
        return ["external", f"{callable_.__module__}.{callable_.__qualname__}"]
    call = getattr(type(callable_), "__call__", None)
    if isinstance(call, types.FunctionType) and project_file(call.__code__.co_filename) is not None:
        return function_target(call)
    kind = type(callable_)
    name = getattr(callable_, "__qualname__", None) or getattr(callable_, "__name__", None) or kind.__qualname__
    return ["external", f"{getattr(callable_, '__module__', None) or kind.__module__}.{name}"]


sites: dict[tuple[str, int, int], dict[str, object]] = {}
positions: dict[types.CodeType, list[tuple[int | None, int | None, int | None, int | None]]] = {}


def on_call(code: types.CodeType, offset: int, callable_: object, _arg0: object) -> object:
    relative = project_file(code.co_filename)
    if relative is None:
        return sys.monitoring.DISABLE
    if code not in positions:
        positions[code] = list(code.co_positions())
    line, end_line, column, end_column = positions[code][offset // 2]
    if None in (line, end_line, column, end_column):
        return None
    file = source(relative)
    start, end = file.span(line, column, end_line, end_column)
    if (start, end) not in file.calls:
        return None
    site = sites.setdefault((relative, start, end), {"start": start, "end": end, "line": line, "callees": []})
    callee = target(callable_)
    if callee not in site["callees"]:
        site["callees"].append(callee)
    return None


sys.monitoring.use_tool_id(TOOL, "rerail-calls-oracle")
sys.monitoring.register_callback(TOOL, sys.monitoring.events.CALL, on_call)
sys.monitoring.set_events(TOOL, sys.monitoring.events.CALL)
try:
    runpy.run_path(str(root / "main.py"), run_name="__main__")
finally:
    sys.monitoring.set_events(TOOL, 0)
    sys.monitoring.free_tool_id(TOOL)

found: dict[str, list[dict[str, object]]] = {}
for (relative, _, _), site in sorted(sites.items()):
    site["callees"] = sorted(site["callees"], key=json.dumps)
    found.setdefault(relative, []).append(site)
pathlib.Path(sys.argv[2]).write_text(json.dumps(found))
