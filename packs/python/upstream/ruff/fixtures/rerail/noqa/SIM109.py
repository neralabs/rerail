a, b, c = 1, 2, 3
if a == b or a == c:
    pass

a, b, c = 1, 2, 3
if a == b or a == c:  # noqa
    pass

a, b, c = 1, 2, 3
if a == b or a == c:  # noqa: SIM109
    pass

a, b, c = 1, 2, 3
if a == b or a == c:  # NOQA: SIM109
    pass

a, b, c = 1, 2, 3
if a == b or a == c:  # noqa: E501
    pass
