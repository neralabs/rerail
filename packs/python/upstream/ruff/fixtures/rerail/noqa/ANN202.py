def _add(a: int, b: int):
    return a + b

def _add(a: int, b: int):  # noqa
    return a + b

def _add(a: int, b: int):  # noqa: ANN202
    return a + b

def _add(a: int, b: int):  # NOQA: ANN202
    return a + b

def _add(a: int, b: int):  # noqa: E501
    return a + b
