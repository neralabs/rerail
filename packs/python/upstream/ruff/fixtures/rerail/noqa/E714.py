x, y = 1, 2
if not x is y:
    pass

x, y = 1, 2
if not x is y:  # noqa
    pass

x, y = 1, 2
if not x is y:  # noqa: E714
    pass

x, y = 1, 2
if not x is y:  # NOQA: E714
    pass

x, y = 1, 2
if not x is y:  # noqa: E501
    pass
