def test_x(a=1):
    pass

def test_x(a=1):  # noqa
    pass

def test_x(a=1):  # noqa: PT028
    pass

def test_x(a=1):  # NOQA: PT028
    pass

def test_x(a=1):  # noqa: E501
    pass
