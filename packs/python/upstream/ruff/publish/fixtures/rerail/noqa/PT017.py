try:
    pass
except Exception as e:
    assert e.args

try:
    pass
except Exception as e:
    assert e.args  # noqa

try:
    pass
except Exception as e:
    assert e.args  # noqa: PT017

try:
    pass
except Exception as e:
    assert e.args  # NOQA: PT017

try:
    pass
except Exception as e:
    assert e.args  # noqa: E501
