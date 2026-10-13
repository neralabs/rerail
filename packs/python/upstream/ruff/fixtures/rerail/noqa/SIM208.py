a = 1
if not (not a):
    pass

a = 1
if not (not a):  # noqa
    pass

a = 1
if not (not a):  # noqa: SIM208
    pass

a = 1
if not (not a):  # NOQA: SIM208
    pass

a = 1
if not (not a):  # noqa: E501
    pass
