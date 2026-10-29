x = 1
if x and False:
    pass

x = 1
if x and False:  # noqa
    pass

x = 1
if x and False:  # noqa: SIM223
    pass

x = 1
if x and False:  # NOQA: SIM223
    pass

x = 1
if x and False:  # noqa: E501
    pass
