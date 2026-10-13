try:
    pass
except ValueError:
    raise RuntimeError('x')

try:
    pass
except ValueError:
    raise RuntimeError('x')  # noqa

try:
    pass
except ValueError:
    raise RuntimeError('x')  # noqa: B904

try:
    pass
except ValueError:
    raise RuntimeError('x')  # NOQA: B904

try:
    pass
except ValueError:
    raise RuntimeError('x')  # noqa: E501
