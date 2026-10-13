a, b = 1, 2
if not a == b:
    pass

a, b = 1, 2
if not a == b:  # noqa
    pass

a, b = 1, 2
if not a == b:  # noqa: SIM201
    pass

a, b = 1, 2
if not a == b:  # NOQA: SIM201
    pass

a, b = 1, 2
if not a == b:  # noqa: E501
    pass
