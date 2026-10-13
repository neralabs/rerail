x = 1
if 'a' == x:
    pass

x = 1
if 'a' == x:  # noqa
    pass

x = 1
if 'a' == x:  # noqa: SIM300
    pass

x = 1
if 'a' == x:  # NOQA: SIM300
    pass

x = 1
if 'a' == x:  # noqa: E501
    pass
