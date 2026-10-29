if isinstance(x, int):
    pass
else:
    raise ValueError

if isinstance(x, int):
    pass
else:
    raise ValueError  # noqa

if isinstance(x, int):
    pass
else:
    raise ValueError  # noqa: TRY004

if isinstance(x, int):
    pass
else:
    raise ValueError  # NOQA: TRY004

if isinstance(x, int):
    pass
else:
    raise ValueError  # noqa: E501
