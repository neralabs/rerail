try:
    pass
except (ValueError,):
    pass

try:
    pass
except (ValueError,):  # noqa
    pass

try:
    pass
except (ValueError,):  # noqa: B013
    pass

try:
    pass
except (ValueError,):  # NOQA: B013
    pass

try:
    pass
except (ValueError,):  # noqa: E501
    pass
