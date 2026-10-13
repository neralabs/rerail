x = 1
if type(x) == type(1):
    pass

x = 1
if type(x) == type(1):  # noqa
    pass

x = 1
if type(x) == type(1):  # noqa: E721
    pass

x = 1
if type(x) == type(1):  # NOQA: E721
    pass

x = 1
if type(x) == type(1):  # noqa: E501
    pass
