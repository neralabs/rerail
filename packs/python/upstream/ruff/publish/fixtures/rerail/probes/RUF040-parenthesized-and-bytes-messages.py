x = 1
assert x, (1)
assert x, (
    None  # why
)
assert x, b"a" b"b"
assert x, ((...))
assert x, -1
assert x, 1 + 1
assert x, ("text")
assert (x, 1), 2
assert x, "ok"  # noqa: RUF040
assert x, 3  # noqa: RUF040
