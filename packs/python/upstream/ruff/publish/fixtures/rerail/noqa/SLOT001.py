class A(tuple):
    pass

class A(tuple):  # noqa
    pass

class A(tuple):  # noqa: SLOT001
    pass

class A(tuple):  # NOQA: SLOT001
    pass

class A(tuple):  # noqa: E501
    pass
