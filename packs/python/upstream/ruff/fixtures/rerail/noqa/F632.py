x = 1
if x is 'abc':
    pass

x = 1
if x is 'abc':  # noqa
    pass

x = 1
if x is 'abc':  # noqa: F632
    pass

x = 1
if x is 'abc':  # NOQA: F632
    pass

x = 1
if x is 'abc':  # noqa: E501
    pass
