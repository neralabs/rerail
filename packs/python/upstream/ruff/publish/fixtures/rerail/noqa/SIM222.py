x = 1
if x or True:
    pass

x = 1
if x or True:  # noqa
    pass

x = 1
if x or True:  # noqa: SIM222
    pass

x = 1
if x or True:  # NOQA: SIM222
    pass

x = 1
if x or True:  # noqa: E501
    pass
