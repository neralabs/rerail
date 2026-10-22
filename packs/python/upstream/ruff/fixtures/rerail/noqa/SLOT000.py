class A(str):
    pass

class A(str):  # noqa
    pass

class A(str):  # noqa: SLOT000
    pass

class A(str):  # NOQA: SLOT000
    pass

class A(str):  # noqa: E501
    pass
