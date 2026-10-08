"""Templates in parentheses, at any depth (Ruff's AST has none), and f-strings, which Ruff's `.format` rules skip."""

("{}").format(1, 2)
(("{a}")).format(b=1)
((("{}" "{}"))).format(1)
(  # a comment
    "{0}"
).format(1, 2)
f"{{}}".format(1, 2)
(f"{{a}}").format(b=1)
("{}" + x).format(1, 2)
