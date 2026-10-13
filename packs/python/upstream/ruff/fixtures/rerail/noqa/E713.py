x, y = 1, [1]
if not x in y:
    pass

x, y = 1, [1]
if not x in y:  # noqa
    pass

x, y = 1, [1]
if not x in y:  # noqa: E713
    pass

x, y = 1, [1]
if not x in y:  # NOQA: E713
    pass

x, y = 1, [1]
if not x in y:  # noqa: E501
    pass
