def add(a: int, b: int):
    return a + b

def add(a: int, b: int):  # noqa
    return a + b

def add(a: int, b: int):  # noqa: ANN201
    return a + b

def add(a: int, b: int):  # NOQA: ANN201
    return a + b

def add(a: int, b: int):  # noqa: E501
    return a + b
