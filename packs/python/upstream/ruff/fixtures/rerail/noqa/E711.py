x = 1
if x == None:
    pass

x = 1
if x == None:  # noqa
    pass

x = 1
if x == None:  # noqa: E711
    pass

x = 1
if x == None:  # NOQA: E711
    pass

x = 1
if x == None:  # noqa: E501
    pass
