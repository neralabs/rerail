"""Reference readings of SQL text, shared by the SQL pack's generators. Offline only (see common.py for the pins).

- PostgreSQL: the server's own parser through libpg_query (pglast 8.5, PostgreSQL 18.6). Statements are split by the
  server's scanner (`pglast.parser.split(..., with_parser=False)`) and each is parsed alone, so one rejected statement
  (the regression suite's deliberate errors) does not hide the rest. psql meta-commands (`\\set`, `\\d`, ...) and
  `COPY ... FROM stdin` data blocks are not SQL; `clean_psql` blanks them in place (offsets are kept).
- SQLite: sqlite3 itself. Statements are split with `sqlite3_complete` and run in order in a fresh in-memory database;
  a statement is rejected when SQLite reports a parse error (`syntax error`, `incomplete input`, `unrecognized token`).
- Names and values: `pg_names_values` walks a libpg_query tree for relations (RangeVar), column references (ColumnRef)
  and constants (A_Const), each at its byte offset in the statement.
"""

from __future__ import annotations

import json
import re
import sqlite3
from dataclasses import dataclass, field
from decimal import Decimal, InvalidOperation

from pglast import parser as pg

META_LINE = re.compile(r"^[ \t]*\\")
COPY_FROM_STDIN = re.compile(r"\bfrom\s+stdin\b[^;]*;\s*$", re.IGNORECASE)
SQLITE_DOT_LINE = re.compile(r"^\.[a-z]")


@dataclass
class Statement:
    start: int  # character offsets in the (cleaned) file text
    end: int
    accepted: bool
    kind: str | None = None  # libpg_query statement node, e.g. SelectStmt (PostgreSQL only)
    error: str | None = None
    tree: dict | None = field(default=None, repr=False)

    def text(self, source: str) -> str:
        return source[self.start:self.end]


def clean_psql(text: str) -> tuple[str, int, int]:
    """Blank psql meta-command lines and COPY FROM stdin data (up to `\\.`), keeping every offset. A line counts as a
    meta-command only when it starts outside a quoted literal, quoted identifier, dollar quote or block comment."""
    lines = text.split("\n")
    meta = copy = 0
    in_copy = False
    state: list = [None]
    for index, line in enumerate(lines):
        if in_copy:
            copy += 1
            if line.rstrip("\r") == "\\.":
                in_copy = False
            lines[index] = " " * len(line)
            continue
        if state[0] is None and META_LINE.match(line):
            meta += 1
            lines[index] = " " * len(line)
            continue
        _scan_line(line, state)
        if state[0] is None and COPY_FROM_STDIN.search(line):
            in_copy = True
    return "\n".join(lines), meta, copy


DOLLAR_TAG = re.compile(r"\$([A-Za-z_\x80-\uffff][A-Za-z_0-9\x80-\uffff]*)?\$")


def _scan_line(line: str, state: list) -> None:
    """Advance a PostgreSQL lexical state over one line: state[0] is None, ("'", escape), '"', ("$", tag) or
    ("/*", depth)."""
    position = 0
    length = len(line)
    while position < length:
        current = state[0]
        if current is None:
            char = line[position]
            if line.startswith("--", position):
                return
            if line.startswith("/*", position):
                state[0] = ("/*", 1)
                position += 2
            elif char == "'":
                escape = position > 0 and line[position - 1] in "eE" and (
                    position == 1 or not (line[position - 2].isalnum() or line[position - 2] == "_"))
                state[0] = ("'", escape)
                position += 1
            elif char == '"':
                state[0] = '"'
                position += 1
            elif char == "$" and (position == 0 or not (line[position - 1].isalnum() or line[position - 1] == "_")):
                match = DOLLAR_TAG.match(line, position)
                if match:
                    state[0] = ("$", match.group(0))
                    position = match.end()
                else:
                    position += 1
            else:
                position += 1
        elif current == '"':
            end = line.find('"', position)
            if end < 0:
                return
            if line.startswith('""', end):
                position = end + 2
            else:
                state[0] = None
                position = end + 1
        elif current[0] == "'":
            char = line[position]
            if current[1] and char == "\\":
                position += 2
            elif char == "'":
                if line.startswith("''", position):
                    position += 2
                else:
                    state[0] = None
                    position += 1
            else:
                position += 1
        elif current[0] == "$":
            end = line.find(current[1], position)
            if end < 0:
                return
            state[0] = None
            position = end + len(current[1])
        else:  # block comment, nested
            if line.startswith("/*", position):
                state[0] = ("/*", current[1] + 1)
                position += 2
            elif line.startswith("*/", position):
                state[0] = None if current[1] == 1 else ("/*", current[1] - 1)
                position += 2
            else:
                position += 1


def clean_sqlite(text: str) -> tuple[str, int]:
    lines = text.split("\n")
    dot = 0
    for index, line in enumerate(lines):
        if SQLITE_DOT_LINE.match(line):
            dot += 1
            lines[index] = " " * len(line)
    return "\n".join(lines), dot


def postgres_statements(text: str) -> list[Statement]:
    """Every statement the server's scanner finds, each parsed alone by the server's parser."""
    statements = []
    for piece in pg.split(text, with_parser=False, only_slices=True):
        body = text[piece.start:piece.stop]
        if not body.strip():
            continue
        try:
            tree = json.loads(pg.parse_sql_json(body))
        except pg.ParseError as error:
            statements.append(Statement(piece.start, piece.stop, False, error=str(error)))
            continue
        if not tree.get("stmts"):
            continue  # a statement made only of comments
        node = tree["stmts"][0]["stmt"]
        statements.append(Statement(piece.start, piece.stop, True, kind=next(iter(node)), tree=tree))
    return statements


SQLITE_PARSE_ERROR = re.compile(r"syntax error|incomplete input|unrecognized token|near \"")


def sqlite_statements(text: str) -> list[Statement]:
    statements = []
    connection = sqlite3.connect(":memory:")
    connection.set_authorizer(lambda action, *_: sqlite3.SQLITE_DENY if action == sqlite3.SQLITE_ATTACH
                              else sqlite3.SQLITE_OK)
    start = 0
    for match in re.finditer(";", text):
        end = match.end()
        chunk = text[start:end]
        if not sqlite3.complete_statement(chunk):
            continue
        body = chunk.strip()
        if body and body != ";" and not _only_comments(body):
            offset = start + (len(chunk) - len(chunk.lstrip()))
            try:
                connection.execute(chunk)
                accepted, error = True, None
            except sqlite3.Warning:  # "You can only execute one statement at a time": not a parse verdict
                accepted, error = True, None
            except sqlite3.Error as failure:
                message = str(failure)
                accepted = not SQLITE_PARSE_ERROR.search(message)
                error = message
            statements.append(Statement(offset, match.start(), accepted, error=error))
        start = end
    tail = text[start:]
    if tail.strip() and not _only_comments(tail.strip()):
        offset = start + (len(tail) - len(tail.lstrip()))
        try:
            connection.execute(tail)
            statements.append(Statement(offset, len(text.rstrip()), True))
        except sqlite3.Error as failure:
            statements.append(Statement(offset, len(text.rstrip()), not SQLITE_PARSE_ERROR.search(str(failure)),
                                        error=str(failure)))
    connection.close()
    return statements


def _only_comments(body: str) -> bool:
    stripped = re.sub(r"--[^\n]*|/\*.*?\*/", "", body, flags=re.DOTALL).strip()
    return stripped in ("", ";")


# ---- names and values from a libpg_query tree ----

def _is_range_var(node: dict) -> bool:
    return "relname" in node and "relpersistence" in node


def pg_names_values(tree: dict) -> dict:
    """Relations, column references and constants of one parsed statement, at byte offsets in that statement."""
    relations: list[dict] = []
    columns: list[dict] = []
    constants: list[dict] = []

    def walk(node, wrapper: str | None = None) -> None:
        if isinstance(node, list):
            for item in node:
                walk(item)
            return
        if not isinstance(node, dict):
            return
        if wrapper is None and len(node) == 1:
            key, value = next(iter(node.items()))
            if key[:1].isupper() and isinstance(value, dict):
                walk(value, key)
                return
        if _is_range_var(node):
            parts = [node[key] for key in ("catalogname", "schemaname", "relname") if node.get(key)]
            relations.append({"location": node.get("location", 0), "parts": parts})
        if wrapper == "ColumnRef":
            parts = []
            for item in node.get("fields", []):
                if "String" in item:
                    parts.append(item["String"].get("sval", ""))
                elif "A_Star" in item:
                    parts.append("*")
            columns.append({"location": node.get("location", 0), "parts": parts})
        if wrapper == "A_Const":
            constants.append({"location": node.get("location", 0), "value": _const_value(node)})
        for value in node.values():
            walk(value)

    walk(tree)
    return {"relations": relations, "columns": columns, "constants": constants}


def _const_value(node: dict) -> list:
    if node.get("isnull"):
        return ["null"]
    if "ival" in node:
        return ["number", _canonical_number(str(node["ival"].get("ival", 0)))]
    if "fval" in node:
        return ["number", _canonical_number(node["fval"].get("fval", "0"))]
    if "sval" in node:
        return ["string", node["sval"].get("sval", "")]
    if "boolval" in node:
        return ["bool", bool(node["boolval"].get("boolval", False))]
    if "bsval" in node:
        return ["bits", node["bsval"].get("bsval", "")]
    return ["unknown"]


def _canonical_number(text: str) -> str:
    cleaned = text.replace("_", "")
    try:
        if re.fullmatch(r"-?0[xX][0-9a-fA-F]+|-?0[oO][0-7]+|-?0[bB][01]+", cleaned):
            return str(int(cleaned, 0))
        return format(Decimal(cleaned).normalize(), "f")
    except (InvalidOperation, ValueError):
        return text


# ---- reading a grammar node's text the way the server reads the token ----

IDENT_PART = re.compile(r'\s*("(?:[^"]|"")*"|[^."\s]+)\s*')


def identifier_parts(text: str) -> list[str] | None:
    parts = []
    position = 0
    while True:
        match = IDENT_PART.match(text, position)
        if not match:
            return None
        token = match.group(1)
        part = token[1:-1].replace('""', '"') if token.startswith('"') else _ascii_lower(token)
        parts.append(_truncate_identifier(part))
        position = match.end()
        if position == len(text):
            return parts
        if text[position] != ".":
            return None
        position += 1


def _ascii_lower(token: str) -> str:
    """The server folds only ASCII letters of an unquoted identifier (downcase_identifier, UTF-8 database)."""
    return token.translate(str.maketrans("ABCDEFGHIJKLMNOPQRSTUVWXYZ", "abcdefghijklmnopqrstuvwxyz"))


def _truncate_identifier(part: str) -> str:
    """NAMEDATALEN - 1 = 63 bytes, cut at a character boundary (truncate_identifier)."""
    data = part.encode("utf-8")
    if len(data) <= 63:
        return part
    return data[:63].decode("utf-8", "ignore")


def literal_value(text: str) -> list | None:
    stripped = text.strip()
    lowered = stripped.lower()
    if lowered == "null":
        return ["null"]
    if lowered in ("true", "false"):
        return ["bool", lowered == "true"]
    if re.fullmatch(r"-?\s*(?:0[xXoObB][0-9a-fA-F_]+|[0-9][0-9_]*(?:\.[0-9_]*)?(?:[eE][+-]?[0-9]+)?|\.[0-9_]+(?:[eE][+-]?[0-9]+)?)",
                    stripped):
        return ["number", _canonical_number(re.sub(r"\s+", "", stripped))]
    quoted = _quoted_segments(stripped)
    if quoted is not None:
        prefix, segments = quoted
        if prefix == "":
            return ["string", "".join(segment.replace("''", "'") for segment in segments)]
        if prefix in "eE":
            return ["string", "".join(_unescape_e(segment) for segment in segments)]
        if prefix in "nN":
            return ["string", "".join(segment.replace("''", "'") for segment in segments)]
        if prefix in "bB" and re.fullmatch(r"[01]*", "".join(segments)):
            return ["bits", "b" + "".join(segments)]
        if prefix in "xX" and re.fullmatch(r"[0-9a-fA-F]*", "".join(segments)):
            return ["bits", "x" + "".join(segments)]
        return None
    dollar = re.fullmatch(r"(\$[A-Za-z_0-9]*\$)(.*)\1", stripped, re.DOTALL)
    if dollar:
        return ["string", dollar.group(2)]
    return None


ESCAPE_SEGMENT = re.compile(r"'((?:[^'\\]|''|\\.)*)'", re.DOTALL)
PLAIN_SEGMENT = re.compile(r"'((?:[^']|'')*)'", re.DOTALL)
SEPARATOR = re.compile(r"(?:[ \t\r\f]*(?:--[^\n]*)?\n)+[ \t\r\f\n]*", re.DOTALL)


def _quoted_segments(text: str) -> tuple[str, list[str]] | None:
    """A quoted literal and its continuation segments: `'a'` newline `'b'` is `'ab'` (SQL standard, PostgreSQL)."""
    prefix = ""
    if text[:1] and text[:1] in "eEbBxXnN" and text[1:2] == "'":
        prefix, text = text[0], text[1:]
    segment = ESCAPE_SEGMENT if prefix in ("e", "E") else PLAIN_SEGMENT
    segments = []
    position = 0
    while True:
        match = segment.match(text, position)
        if not match:
            return None
        segments.append(match.group(1))
        position = match.end()
        if position == len(text):
            return prefix, segments
        separator = SEPARATOR.match(text, position)
        if not separator:
            return None
        position = separator.end()


def _unescape_e(body: str) -> str:
    body = body.replace("''", "'")
    escapes = {"b": "\b", "f": "\f", "n": "\n", "r": "\r", "t": "\t"}

    def replace(match: re.Match) -> str:
        token = match.group(1)
        if token in escapes:
            return escapes[token]
        if token[0] in "01234567":
            return chr(int(token, 8))
        if token[0] == "x":
            return chr(int(token[1:], 16))
        if token[0] in "uU":
            return chr(int(token[1:], 16))
        return token

    return re.sub(r"\\(u[0-9a-fA-F]{4}|U[0-9a-fA-F]{8}|x[0-9a-fA-F]{1,2}|[0-7]{1,3}|.)", replace, body)
