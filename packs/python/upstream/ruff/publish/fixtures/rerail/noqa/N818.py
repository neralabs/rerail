class Foo(Exception):
    pass

class Foo(Exception):  # noqa
    pass

class Foo(Exception):  # noqa: N818
    pass

class Foo(Exception):  # NOQA: N818
    pass

class Foo(Exception):  # noqa: E501
    pass
