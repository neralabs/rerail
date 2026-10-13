a = 1
if a and not a:
    pass

a = 1
if a and not a:  # noqa
    pass

a = 1
if a and not a:  # noqa: SIM220
    pass

a = 1
if a and not a:  # NOQA: SIM220
    pass

a = 1
if a and not a:  # noqa: E501
    pass
