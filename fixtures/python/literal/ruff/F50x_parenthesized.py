"""Format strings in parentheses, at any depth (Ruff's AST has none), and f-strings, which Ruff's `%` rules skip."""

("%(a)s") % {"b": 1}
(("%(a)s")) % {"b": 1}
((("%(a)s" "%(c)s"))) % {"b": 1}
"%(a)s" % ({"b": 1})
("%s %s") % (1,)
(("%s")) % (1, 2)
(("hello")) % 1
(("%s %s")) % 42
(  # a comment
    "%(a)s"
) % {"a": 1, "b": 2}
f"%(a)s" % {"b": 1}
(f"%s %s") % (1,)
("%s" + x) % (1, 2)
