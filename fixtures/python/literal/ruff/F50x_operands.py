"""Operands Ruff's F507 types (`ResolvedPythonType`), for the `%` rules."""

# A tuple Ruff types but cannot count: no F507, whatever the placeholders.
"hello" % ((1,) + (2,))
"%s" % ((1,) + (2,))
"%s %s" % ((1,) + (2,))
"hello" % (() + ())
# A starred display is a tuple (or list, set, dict) of unknown length.
"hello" % (*a,)
"%s" % (*a, *b)
"hello" % [*a]
"%s %s" % [*a]
"hello" % {*a}
"hello" % {**a}
# Unknown or a type error to Ruff: one value, F507 with no placeholder only.
"hello" % ((1,) * 2)
"%s %s" % ((1,) * 2)
"hello" % (1 + "a")
"hello" % (a + b)
"hello" % (1 if c else (2,))
"%s %s" % (1 if c else (2,))
# Known non-tuple types: one value.
"hello" % ([1] + [2])
"%s %s" % ([1] + [2])
"%s %s" % ("a" * 2)
"%s %s" % (x := 1)
"%s %s" % (not x)
"%s %s" % -1.5
"%s %s" % (1 or 2.0)
