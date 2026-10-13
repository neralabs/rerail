items = [1]
for items in items:
    pass

items = [1]
for items in items:  # noqa
    pass

items = [1]
for items in items:  # noqa: B020
    pass

items = [1]
for items in items:  # NOQA: B020
    pass

items = [1]
for items in items:  # noqa: E501
    pass
