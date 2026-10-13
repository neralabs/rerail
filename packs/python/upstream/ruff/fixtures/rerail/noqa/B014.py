try:
    pass
except (ValueError, ValueError):
    pass

try:
    pass
except (ValueError, ValueError):  # noqa
    pass

try:
    pass
except (ValueError, ValueError):  # noqa: B014
    pass

try:
    pass
except (ValueError, ValueError):  # NOQA: B014
    pass

try:
    pass
except (ValueError, ValueError):  # noqa: E501
    pass
