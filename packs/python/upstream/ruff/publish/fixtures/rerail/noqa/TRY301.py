try:
    raise ValueError
except Exception:
    pass

try:
    raise ValueError  # noqa
except Exception:
    pass

try:
    raise ValueError  # noqa: TRY301
except Exception:
    pass

try:
    raise ValueError  # NOQA: TRY301
except Exception:
    pass

try:
    raise ValueError  # noqa: E501
except Exception:
    pass
