x = y = 1
if x:
    if y:
        pass

x = y = 1
if x:  # noqa
    if y:
        pass

x = y = 1
if x:  # noqa: SIM102
    if y:
        pass

x = y = 1
if x:  # NOQA: SIM102
    if y:
        pass

x = y = 1
if x:  # noqa: E501
    if y:
        pass
