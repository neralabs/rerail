a = 1
if a or not a:
    pass

a = 1
if a or not a:  # noqa
    pass

a = 1
if a or not a:  # noqa: SIM221
    pass

a = 1
if a or not a:  # NOQA: SIM221
    pass

a = 1
if a or not a:  # noqa: E501
    pass
