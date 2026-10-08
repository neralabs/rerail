zip(a, b)
zip(a, b, strict=True)
zip(a, b, strict=False)
zip(a, b, **kw)
zip(a, b, **{"strict": True})
zip(a, b, **{})
zip(a, b, *rest)
zip(*rest)
zip(*rest, strict=True)
zip(a)
zip(a, **kw)
zip()
zip(a, b, c, **{"strict": True, **kw})
zip(a, strict=True, *rest)
zip(a, b, strict=True, **kw)


def shadowed(zip):
    zip(a, b)
