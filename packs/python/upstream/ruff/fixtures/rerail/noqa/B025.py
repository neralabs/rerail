try:
    pass
except ValueError:
    pass
except ValueError:
    pass

try:
    pass
except ValueError:
    pass
except ValueError:  # noqa
    pass

try:
    pass
except ValueError:
    pass
except ValueError:  # noqa: B025
    pass

try:
    pass
except ValueError:
    pass
except ValueError:  # NOQA: B025
    pass

try:
    pass
except ValueError:
    pass
except ValueError:  # noqa: E501
    pass
