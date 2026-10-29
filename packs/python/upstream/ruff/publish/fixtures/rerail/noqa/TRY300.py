try:
    a = 1
    return a
except ValueError:
    pass

try:
    a = 1
    return a  # noqa
except ValueError:
    pass

try:
    a = 1
    return a  # noqa: TRY300
except ValueError:
    pass

try:
    a = 1
    return a  # NOQA: TRY300
except ValueError:
    pass

try:
    a = 1
    return a  # noqa: E501
except ValueError:
    pass
