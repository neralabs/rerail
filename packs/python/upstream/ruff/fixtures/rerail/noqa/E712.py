x = 1
if x == True:
    pass

x = 1
if x == True:  # noqa
    pass

x = 1
if x == True:  # noqa: E712
    pass

x = 1
if x == True:  # NOQA: E712
    pass

x = 1
if x == True:  # noqa: E501
    pass
