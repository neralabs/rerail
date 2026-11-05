"""Exact regex-to-glob conversion for text tests (ALIGNMENT.md, "Text: globs first").

text_test(rx, subject) takes a whole-string regex, as souffle.whole() writes it, and returns a glob-family
constraint that matches exactly the same strings, or None. It never guesses: any construct whose glob reading
differs for some string makes it return None, and the caller keeps `match`.

Glob semantics (this branch): `*` any run without `/`, `**` any run, `?` one character other than `/`,
`[abc]` / `[a-z]` one listed character, `{a,b}` alternatives, `\\` escapes the next character.
"""
import re

GLOB_SPECIAL = set("*?[]{},\\")
TESTS = ("glob", "iglob", "prefix", "iprefix", "suffix", "isuffix", "contains", "icontains", "word", "iword")


class _No(Exception):
    pass


def _lit(ch):
    return "\\" + ch if ch in GLOB_SPECIAL else ch


def _class(s, i):
    """A regex class at s[i] == '['; returns (glob class, next index). Negated classes are refused: `[^_]` matches
    `/`, which a glob class reading would have to special-case."""
    j = i + 1
    if j < len(s) and s[j] == "^":
        raise _No
    out = "["
    while j < len(s) and s[j] != "]":
        c = s[j]
        if c == "\\":
            if j + 1 >= len(s):
                raise _No
            e = s[j + 1]
            if e == "d":
                out += "0-9"
            elif e in "wsWSDbB":
                raise _No
            elif e.isalnum():
                raise _No
            else:
                out += "\\" + e
            j += 2
            continue
        if c == "[":
            raise _No
        out += "\\" + c if c in "!\\" and j == i + 1 else c
        j += 1
    if j >= len(s) or out == "[":
        raise _No
    return out + "]", j + 1


def _split_alts(s):
    out, d, cur, j = [], 0, "", 0
    while j < len(s):
        c = s[j]
        if c == "\\":
            cur += s[j:j + 2]
            j += 2
            continue
        if c == "[":
            k = s.index("]", j + 1) if "]" in s[j + 1:] else len(s)
            cur += s[j:k + 1]
            j = k + 1
            continue
        if c == "(":
            d += 1
        elif c == ")":
            d -= 1
        if c == "|" and d == 0:
            out.append(cur)
            cur = ""
        else:
            cur += c
        j += 1
    out.append(cur)
    return out


def _group_end(s, i):
    d, j = 0, i
    while j < len(s):
        c = s[j]
        if c == "\\":
            j += 2
            continue
        if c == "[":
            j = s.index("]", j + 1) + 1 if "]" in s[j + 1:] else len(s)
            continue
        if c == "(":
            d += 1
        elif c == ")":
            d -= 1
            if d == 0:
                return j
        j += 1
    raise _No


def _seq(s):
    """A regex without top-level `|` to a glob fragment."""
    out, i = "", 0
    while i < len(s):
        c = s[i]
        nxt = s[i + 1] if i + 1 < len(s) else ""
        if c == "." and nxt == "*":
            if s[i + 2:i + 3] in ("?", "+", "*"):
                raise _No
            out += "**"
            i += 2
            continue
        if c in ".^$+*?|)":
            raise _No
        if c == "(":
            e = _group_end(s, i)
            inner = s[i + 1:e]
            if inner.startswith("?:"):
                inner = inner[2:]
            elif inner.startswith("?"):
                raise _No              # named groups, flags, look-around
            frag = _alts(inner)
            i = e + 1
            if s[i:i + 1] in ("?", "*", "+", "{"):
                if s[i:i + 1] == "?" and s[i + 1:i + 2] not in ("?", "+"):
                    # (x)? is the alternatives {x,}: x or nothing.
                    frag = frag[:-1] + ",}" if _flat_alts(frag) else "{" + frag + ",}"
                    i += 1
                else:
                    raise _No
            out += frag
            continue
        if c == "[":
            item, i = _class(s, i)
        elif c == "\\":
            if not nxt:
                raise _No
            if nxt == "d":
                item = "[0-9]"
            elif nxt.isalnum():
                raise _No              # \w \s \b \A … and escapes like \n
            else:
                item = _lit(nxt)
            i += 2
        else:
            item = _lit(c)
            i += 1
        m = re.match(r"\{(\d+)\}", s[i:])
        if m:
            item *= int(m.group(1))
            i += m.end()
        elif s[i:i + 1] in ("?", "*", "+") or s[i:i + 1] == "{":
            raise _No
        out += item
    return out


def _flat_alts(frag):
    """`{a,b}` with no nested braces."""
    return frag.startswith("{") and frag.endswith("}") and "{" not in frag[1:-1].replace("\\{", "")


def _alts(s):
    parts = [_seq(p) for p in _split_alts(s)]
    if len(parts) == 1:
        return parts[0]
    if any(p == "" for p in parts):
        parts = [p for p in parts if p] + [""]
    return "{" + ",".join(parts) + "}"


def _plain(g):
    """The literal text of a glob fragment with no wildcards, or None."""
    out, i = "", 0
    while i < len(g):
        if g[i] == "\\":
            out += g[i + 1]
            i += 2
        elif g[i] in "*?[]{},":
            return None
        else:
            out += g[i]
            i += 1
    return out


def glob_of(rx):
    """(glob, case_insensitive) for a whole-string regex, or None."""
    ci = False
    m = re.match(r"\(\?([a-zA-Z]+)\)", rx)
    if m:
        if set(m.group(1)) - {"i"}:
            return None
        ci, rx = True, rx[m.end():]
    if "(?<" in rx or "(?P<" in rx:
        return None
    try:
        return _alts(rx), ci
    except (_No, ValueError, IndexError):
        return None


def quote(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def text_test(rx, subject):
    r = glob_of(rx)
    if r is None:
        return None
    g, ci = r
    i = "i" if ci else ""
    if g.startswith("**") and g.endswith("**") and len(g) > 4:
        lit = _plain(g[2:-2])
        if lit:
            return f"{i}contains({quote(lit)}, {subject})"
    if g.endswith("**") and len(g) > 2:
        lit = _plain(g[:-2])
        if lit:
            return f"{i}prefix({quote(lit)}, {subject})"
    if g.startswith("**") and len(g) > 2:
        lit = _plain(g[2:])
        if lit:
            return f"{i}suffix({quote(lit)}, {subject})"
    return f"{i}glob({quote(g)}, {subject})"


if __name__ == "__main__":
    # Each case: regex, the expected test, and strings it must agree with re.fullmatch on.
    cases = [
        ("hasattr", 'glob("hasattr", s)', ["hasattr", "hasattrs"]),
        (".*_async", 'suffix("_async", s)', ["f_async", "_async_x"]),
        ("Sql.*", 'prefix("Sql", s)', ["SqlX", "xSql"]),
        (".*cache.*", 'contains("cache", s)', ["a_cache_b", "cach"]),
        ("(?:compile|lower)", 'glob("{compile,lower}", s)', ["compile", "lower", "compiler"]),
        ("(?i).*encoding.*", 'icontains("encoding", s)', ["X-Encoding", "enc"]),
        ("_.*(?:branch_tip|revision_tip).*", 'glob("_**{branch_tip,revision_tip}**", s)', ["_x_branch_tip_y", "branch_tip"]),
        ("l\\d{4}_.*", 'glob("l[0-9][0-9][0-9][0-9]_**", s)', ["l2070_batching", "l207_x"]),
        ("[A-Z].*", 'glob("[A-Z]**", s)', ["Abc", "abc"]),
        ("a\\.b", 'glob("a.b", s)', ["a.b", "axb"]),
        ("(?:api/)?impl/.*", 'glob("{api/,}impl/**", s)', ["impl/a.py", "api/impl/b", "x/impl/c"]),
    ]
    refused = ["_\\w*cache\\w*", "[^_].*", "(?<x>a)", ".+", "a?b", "(?m)^x", "\\bSELECT\\b"]
    bad = 0
    for rx, want, samples in cases:
        got = text_test(rx, "s")
        if got != want:
            print("MISMATCH", rx, got, want)
            bad += 1
        g, ci = glob_of(rx)
        # Check against a reference glob matcher.
        def to_re(gl):
            o, k = "", 0
            while k < len(gl):
                ch = gl[k]
                if ch == "\\":
                    o += re.escape(gl[k + 1]); k += 2; continue
                if gl.startswith("**", k):
                    o += "(?s:.*)"; k += 2; continue
                if ch == "*":
                    o += "[^/]*"
                elif ch == "?":
                    o += "[^/]"
                elif ch == "[":
                    e = gl.index("]", k); o += gl[k:e + 1]; k = e + 1; continue
                elif ch == "{":
                    o += "(?:"
                elif ch == "}":
                    o += ")"
                elif ch == ",":
                    o += "|"
                else:
                    o += re.escape(ch)
                k += 1
            return o
        for smp in samples:
            a = bool(re.fullmatch(rx, smp))
            b = bool(re.fullmatch(to_re(g), smp, re.I if ci else 0))
            if a != b:
                print("DISAGREE", rx, g, smp, a, b)
                bad += 1
    for rx in refused:
        if glob_of(rx) is not None:
            print("SHOULD REFUSE", rx, glob_of(rx))
            bad += 1
    print("ok" if not bad else f"{bad} problems")
