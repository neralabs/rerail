class A(metaclass=type):
    pass

class A(metaclass=type):  # noqa
    pass

class A(metaclass=type):  # noqa: UP050
    pass

class A(metaclass=type):  # NOQA: UP050
    pass

class A(metaclass=type):  # noqa: E501
    pass
