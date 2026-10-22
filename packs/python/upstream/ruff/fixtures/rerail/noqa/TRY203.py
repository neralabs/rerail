try:
    pass
except ValueError:
    raise

try:
    pass
except ValueError:  # noqa
    raise

try:
    pass
except ValueError:  # noqa: TRY203
    raise

try:
    pass
except ValueError:  # NOQA: TRY203
    raise

try:
    pass
except ValueError:  # noqa: E501
    raise
