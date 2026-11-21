"""`rdl.py`'s `sql_select` builds the SQL in its golden, byte for byte.

The cases are bodies built by hand, one for each way `sql_select` reads a literal, each called with and without the
native `nearest` walk, with every literal as the delta, and with no tables, empty tables, the first relation's or every
relation's full and delta tables. The golden holds each distinct result once, under the labels of the calls that build
it. `RERAIL_BLESS_SQL=1` rewrites it.
"""
import os
import sys
import unittest
from pathlib import Path

LOGIC = Path(__file__).resolve().parents[2] / "native/src/code_rule_dsl/logic"
GOLDEN = LOGIC / "plan/golden/sql_select.txt"
sys.path.insert(0, str(LOGIC / "front"))
import rdl  # noqa: E402
from rdl import Agg, Atom, Call, Cmp, Const, Nearest, TopT, Var as V, Wild  # noqa: E402


def program():
    prog = rdl.Program()
    prog.inputs = ["node", "child", "text", "kind", "doc"]
    prog.kindsets = {"scope": ["module", "function_definition"], "cls": ["class_definition"]}
    prog.docof = {"docof": (0, 1)}
    return prog


def two_nearests():
    return [
        (Atom("kind", [V("n"), Const("identifier")]), None), (Cmp(V("n"), ">", Const(3)), None),
        (Nearest("n", "s", "scope"), "climb"), (Atom("text", [V("s"), V("t")]), None),
        (Nearest("n", "c", "cls"), "climb"), (Cmp(V("c"), "<=", V("s")), None),
        (Atom("child", [V("c"), Wild(), Const(0)], True), None), (Agg("k", Atom("child", [V("c"), Wild(), Wild()])), None),
    ]


CASES = {
    "neg all-wild": (Atom("h", [V("n")]), [(Atom("kind", [V("n"), Const("x")]), None), (Atom("child", [Wild(), Wild(), Wild()], True), None)]),
    "count all-wild": (Atom("h", [V("n"), V("k")]), [(Atom("kind", [V("n"), Const("x")]), None), (Agg("k", Atom("child", [Wild(), Wild(), Wild()])), None)]),
    "TopT in positive atom": (Atom("h", [V("n")]), [(Atom("cv", [V("n"), TopT()]), None)]),
    "repeated var in atom": (Atom("h", [V("n")]), [(Atom("child", [V("n"), V("n"), Wild()]), None)]),
    "descend with n bound": (Atom("h", [V("n"), V("s")]), [(Atom("kind", [V("s"), Const("module")]), None), (Atom("kind", [V("n"), Const("identifier")]), None), (Nearest("n", "s", "scope"), "descend")]),
    "climb with a bound": (Atom("h", [V("n"), V("s")]), [(Atom("kind", [V("s"), Const("module")]), None), (Atom("kind", [V("n"), Const("identifier")]), None), (Nearest("n", "s", "scope"), "climb")]),
    "nearest first (empty FROM)": (Atom("h", [V("s")]), [(Nearest("n", "s", "scope"), "climb")]),
    "nearest first descend": (Atom("h", [V("n")]), [(Nearest("n", "s", "scope"), "descend"), (Atom("kind", [V("n"), Const("x")]), None)]),
    "two nearests then literals": (Atom("h", [V("n"), V("s"), V("c")]), two_nearests()),
    "cmp only (empty final FROM)": (Atom("h", [V("x")]), [(Cmp(V("x"), "=", Const(1)), None)]),
    "empty body": (Atom("h", [Const(1)]), []),
    "unknown literal skipped": (Atom("h", [V("n")]), [(Atom("kind", [V("n"), Const("x")]), None), ("not a literal", None), (Wild(), None)]),
    "ops": (Atom("h", [V("n")]), [(Atom("intval", [V("n"), V("v")]), None)] + [(Cmp(V("v"), op, Const(2)), None) for op in ("<", ">", "<=", ">=", "!=", "=")]),
    "bind then test": (Atom("h", [V("n"), V("y")]), [(Atom("intval", [V("n"), V("v")]), None), (Cmp(V("y"), "=", Call("flat_bin", [Const("+"), V("v"), Const(1)])), None), (Cmp(V("y"), "=", V("v")), None)]),
    "cat bind and full split": (Atom("h", [V("q"), V("x"), V("w")]), [(Atom("text", [V("n"), V("w")]), None), (Cmp(V("w"), "=", Call("cat", [V("q"), V("x")])), None), (Cmp(V("z"), "=", Call("cat", [V("q"), V("x")])), None)]),
    "cat split, prefix bound": (Atom("h", [V("x")]), [(Atom("text", [V("n"), V("w")]), None), (Atom("text", [V("m"), V("q")]), None), (Cmp(V("w"), "=", Call("cat", [V("q"), V("x")])), None)]),
    "cat split, suffix bound": (Atom("h", [V("q")]), [(Atom("text", [V("n"), V("w")]), None), (Atom("text", [V("m"), V("x")]), None), (Cmp(V("w"), "=", Call("cat", [V("q"), V("x")])), None)]),
    "range with const doc": (Atom("h", [V("v")]), [(Atom("kind", [V("v"), Const("identifier")]), "range"), (Atom("docof", [V("v"), Const(3)]), None)]),
    "range with bound doc": (Atom("h", [V("v")]), [(Atom("doc", [V("d"), Wild(), Wild()]), None), (Atom("kind", [V("v"), Const("identifier")]), "range"), (Atom("docof", [V("v"), V("d")]), None)]),
    "range no docof (raises)": (Atom("h", [V("v")]), [(Atom("kind", [V("v"), Const("identifier")]), "range")]),
    "unbound head var (raises)": (Atom("h", [V("zz")]), [(Atom("kind", [V("v"), Const("identifier")]), None)]),
    "string escaping": (Atom("h", [V("v")]), [(Atom("text", [V("v"), Const("it's")]), None)]),
}


def table_choices(order):
    """No tables, empty tables, the first relation's, and every relation's full and delta tables (`t0` to `t3`)."""
    rels = [lit.rel for lit, _ in order if isinstance(lit, Atom)]
    named = lambda chosen: {r: (f"full_{r}", f"dl_{r}") for r in chosen}  # noqa: E731
    return [None, {}, named(rels[:1]), named(rels)]


def calls(order):
    """Each call's label and its `NATIVE`, `delta_at` and `tables`."""
    for native in (set(), {"nearest"}):
        for delta_at in [None] + list(range(len(order))):
            for t, tables in enumerate(table_choices(order)):
                label = f"n{len(native)}d{'-' if delta_at is None else delta_at}t{t}"
                yield label, native, delta_at, tables


def built(prog, head, order, native, delta_at, tables):
    rdl.NATIVE = native
    try:
        return rdl.sql_select(prog, head, order, delta_at=delta_at, tables=tables)
    except Exception as e:  # a body `sql_select` refuses is pinned by its error
        return f"EXC {type(e).__name__}: {e}"


def golden_text():
    legend = ("# test/scripts/test_rdl_sql_select.py (RERAIL_BLESS_SQL=1 rewrites this file): per case, each distinct result "
              "under its calls. n: native nearest; d: delta literal; t: no, empty, first or every relation's tables.\n\n")
    prog, saved, text = program(), rdl.NATIVE, [legend]
    try:
        for name, (head, order) in CASES.items():
            by_sql = {}
            for label, native, delta_at, tables in calls(order):
                by_sql.setdefault(built(prog, head, order, native, delta_at, tables), []).append(label)
            text.append(f"## {name}\n" + "".join(f"{' '.join(labels)}\n{sql}\n\n" for sql, labels in by_sql.items()))
    finally:
        rdl.NATIVE = saved
    return "".join(text)


class SqlSelect(unittest.TestCase):
    def test_sql_select_builds_the_golden_sql(self):
        made = golden_text()
        if os.environ.get("RERAIL_BLESS_SQL"):
            GOLDEN.write_text(made)
        self.maxDiff = 4000
        self.assertEqual(GOLDEN.read_text(), made, f"sql_select's SQL differs from {GOLDEN} (RERAIL_BLESS_SQL=1 rewrites it)")


if __name__ == "__main__":
    unittest.main()
