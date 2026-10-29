try:
    pass
except IOError:
    pass

try:
    pass
except IOError:  # noqa
    pass

try:
    pass
except IOError:  # noqa: UP024
    pass

try:
    pass
except IOError:  # NOQA: UP024
    pass

try:
    pass
except IOError:  # noqa: E501
    pass
